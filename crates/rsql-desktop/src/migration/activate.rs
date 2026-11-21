use super::{
    ActivatedReceipt, ImportError, PreparedReceipt, Result, boundary,
    prepare::previous,
    record::{self, JOURNAL},
    recovery::{Journal, RecoveryOutcome, resolve},
};
use rsql_core::migration::ProfileLease;

/// Transfer the unique closed-profile lease and the operation to a blocking worker. Dropping
/// the waiter cannot release the lease or cancel a partially committed activation.
pub async fn activate_import(
    receipt: PreparedReceipt,
    lease: ProfileLease,
) -> Result<ActivatedReceipt> {
    tokio::task::spawn_blocking(move || activate_closed(receipt, lease))
        .await
        .map_err(|_| ImportError::Worker)?
}

pub(super) fn activate_closed(
    receipt: PreparedReceipt,
    mut lease: ProfileLease,
) -> Result<ActivatedReceipt> {
    let record = receipt.record;
    if record.directory != lease.verify_directory()?
        || record.destination
            != record::bytes_hash(lease.directory().as_os_str().as_encoded_bytes())?
    {
        return Err(ImportError::Stale);
    }
    let mut warnings = receipt.backup_warnings;
    warnings.extend(super::recover(&mut lease)?.into_backup_warnings());
    let previous = previous(&lease)?;
    if let Some(previous) = previous {
        if previous.source == record.source {
            return Ok(ActivatedReceipt(previous, warnings));
        }
        if previous.source.snapshot_id == record.source.snapshot_id {
            return Err(ImportError::ReusedSnapshot);
        }
    }
    let database = receipt.database.ok_or(ImportError::Stale)?;
    let profile = lease.directory();
    let stage = record.validate(&lease)?;
    if (!record.replace && super::prepare::other_profile_data(profile)?)
        || record::live_hashes(profile)? != record.old
    {
        return Err(ImportError::Stale);
    }
    record::standalone(profile)?;
    record::standalone(&stage)?;
    let installed = record.installed()?;
    for (name, expected) in record::LIVE[..2].iter().zip(installed) {
        if record::file_hash(&stage.join(name))? != expected {
            return Err(ImportError::Stale);
        }
    }
    let journal = Journal::new(record.clone());
    if record::file_hash(&stage.join(rsql_core::migration::PRE_REKEY_BACKUP))? != record.pre_rekey {
        return Err(ImportError::Stale);
    }
    database.verify_stage_identity()?;
    let intent = record::temp_bytes(&stage, &journal.encode()?)?;
    boundary("before journal publication")?;
    record::publish_new(intent, &profile.join(JOURNAL))?;
    // The visible journal now owns these names even when the parent flush fails.
    database.into_directory();
    boundary("journal renamed")?;
    record::sync_directory(&stage)?;
    record::sync_directory(profile)?;
    boundary("journal synced")?;
    match resolve(&mut lease, journal)? {
        RecoveryOutcome::Installed => Ok(ActivatedReceipt(record, warnings)),
        RecoveryOutcome::InstalledWithUnverifiedBackups(paths) => {
            warnings.extend(paths);
            Ok(ActivatedReceipt(record, warnings))
        }
        _ => Err(ImportError::Integrity("import rolled back")),
    }
}
