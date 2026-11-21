use super::{
    ImportError, RecoveryOutcome, Result, boundary,
    record::{self, BACKUPS, Completion, FORMAT, Hashes, JOURNAL, LIVE, file_hash},
};
use rsql_core::migration::{DirectoryId, ProfileLease, Sha256Digest};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const ARCHIVE: &str = "rollback.json";
const RESTORE: [&str; 4] = [
    "restore.db",
    "restore.workspace.json",
    "restore.generation",
    "restore.receipt.json",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rollback {
    version: u32,
    operation: Uuid,
    directory: DirectoryId,
    archive: DirectoryId,
    destination: Sha256Digest,
    previous: Hashes,
    restore: Hashes,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    rollback: Rollback,
    checksum: Sha256Digest,
}

fn digest(path: &Path) -> Result<Sha256Digest> {
    record::bytes_hash(path.as_os_str().as_encoded_bytes())
}

impl Rollback {
    fn location(&self, profile: &Path) -> Result<PathBuf> {
        let folder = profile.join(format!(".rsql-rollback-{}", self.operation));
        if self.version != FORMAT
            || self.operation.is_nil()
            || self.directory != DirectoryId::read(profile)?
            || self.destination != digest(profile)?
            || self.archive != DirectoryId::read(&folder)?
        {
            return Err(ImportError::Integrity("rollback directory identity"));
        }
        Ok(folder)
    }
}

impl Intent {
    fn verify(&self, profile: &Path) -> Result<PathBuf> {
        let folder = self.rollback.location(profile)?;
        if self.checksum != record::bytes_hash(&record::encode(&self.rollback)?)?
            || record::read_record::<Self>(&folder.join(ARCHIVE))?.rollback != self.rollback
            || file_hash(&folder.join(ARCHIVE))?
                != Some(record::bytes_hash(&record::encode(self)?)?)
        {
            return Err(ImportError::Integrity("rollback archive provenance"));
        }
        Ok(folder)
    }
}

fn verify_members(folder: &Path, names: &[&str; 4], expected: Hashes) -> Result<()> {
    for (name, hash) in names.iter().zip(expected) {
        if file_hash(&folder.join(name))? != hash {
            return Err(ImportError::Integrity("retained backup digest"));
        }
    }
    Ok(())
}

fn copy_members(
    from: &Path,
    names: &[&str; 4],
    to: &Path,
    targets: &[&str; 4],
    hashes: Hashes,
) -> Result<()> {
    for ((name, target), hash) in names.iter().zip(targets).zip(hashes) {
        if let Some(hash) = hash {
            record::publish_new(
                record::temp_copy(to, &from.join(name), hash)?,
                &to.join(target),
            )?;
        }
    }
    verify_members(to, targets, hashes)
}

#[derive(Clone)]
pub struct Backup {
    folder: PathBuf,
    identity: DirectoryId,
    profile: PathBuf,
    hashes: Hashes,
}

impl Backup {
    pub fn location(&self) -> &Path {
        &self.folder
    }
}

/// Names are only navigation hints. Selecting a folder must independently validate its contents.
pub fn backup_locations(profile: &Path) -> Result<Vec<PathBuf>> {
    let mut locations = Vec::new();
    for entry in fs::read_dir(profile)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if (name.starts_with(".rsql-import-") || name.starts_with(".rsql-rollback-"))
            && entry.file_type()?.is_dir()
            && (record::present(&entry.path().join(BACKUPS[0]))?
                || record::present(&entry.path().join(BACKUPS[1]))?)
        {
            locations.push(entry.path());
        }
    }
    locations.sort();
    Ok(locations)
}

/// Read-only preview also works while the selected profile is open in this process.
pub fn inspect_backup(folder: &Path, profile: &Path) -> Result<Backup> {
    let identity = DirectoryId::read(folder)?;
    let folder = folder.canonicalize()?;
    let profile = profile.canonicalize()?;
    if folder.parent() != Some(&profile) {
        return Err(ImportError::Integrity(
            "select a retained backup inside this profile",
        ));
    }
    let hashes = if record::present(&folder.join(ARCHIVE))? {
        let intent = record::read_record::<Intent>(&folder.join(ARCHIVE))?;
        if intent.verify(&profile)? != folder {
            return Err(ImportError::Stale);
        }
        intent.rollback.previous
    } else {
        let prepared = record::read_record::<record::Record>(&folder.join(record::PREPARED))?;
        if prepared.version != FORMAT
            || prepared.directory != DirectoryId::read(&profile)?
            || prepared.destination != digest(&profile)?
            || prepared.stage != identity
            || record::stage_path(&profile, prepared.operation) != folder
            || file_hash(&folder.join(record::PREPARED))?
                != Some(prepared.completion()?.prepared_sha256)
            || file_hash(&folder.join(record::CONFIRMED))? != prepared.installed()?[record::RECEIPT]
            || prepared.completion()?
                != record::read_record::<Completion>(&folder.join(record::CONFIRMED))?
        {
            return Err(ImportError::Integrity("backup activation provenance"));
        }
        prepared.old
    };
    if hashes[..2].iter().all(Option::is_none) {
        return Err(ImportError::Integrity(
            "no previous database or workspace in backup",
        ));
    }
    verify_members(&folder, &BACKUPS, hashes)?;
    Ok(Backup {
        folder,
        identity,
        profile,
        hashes,
    })
}

/// A rollback can detach a newer activation from the current generation's provenance chain.
pub(super) fn reject_reused(lease: &ProfileLease, source: &record::Source) -> Result<()> {
    for entry in fs::read_dir(lease.directory())? {
        let entry = entry?;
        if !entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix(".rsql-rollback-"))
            .is_some_and(|id| Uuid::try_parse(id).is_ok())
        {
            continue;
        }
        let folder = entry.path();
        let intent = record::read_record::<Intent>(&folder.join(ARCHIVE))?;
        if intent.verify(lease.directory())? != folder {
            return Err(ImportError::Stale);
        }
        if let Some(hash) = intent.rollback.previous[record::RECEIPT] {
            let path = folder.join(BACKUPS[record::RECEIPT]);
            if file_hash(&path)? != Some(hash) {
                return Err(ImportError::Integrity("preserved activation history"));
            }
            let detached = record::read_record::<Completion>(&path)?.verify(lease)?;
            super::prepare::reject_reused(lease, detached, source)?;
        }
    }
    Ok(())
}

pub fn restore_backup(backup: Backup, mut lease: ProfileLease) -> Result<PathBuf> {
    lease.require_resolved_import()?;
    record::standalone(lease.directory())?;
    super::prepare::previous(&lease)?;
    let checked = inspect_backup(&backup.folder, lease.directory())?;
    if checked.identity != backup.identity
        || checked.hashes != backup.hashes
        || checked.profile != backup.profile
    {
        return Err(ImportError::Stale);
    }
    super::prepare::previous_at(&lease, &checked.folder, &BACKUPS)?;
    let profile = lease.directory();
    let previous = record::live_hashes(profile)?;
    let operation = Uuid::new_v4();
    let folder = profile.join(format!(".rsql-rollback-{operation}"));
    let stage = tempfile::Builder::new()
        .prefix(".rsql-save-")
        .tempdir_in(profile)?;
    copy_members(profile, &LIVE, stage.path(), &BACKUPS, previous)?;
    copy_members(
        &backup.folder,
        &BACKUPS,
        stage.path(),
        &RESTORE,
        backup.hashes,
    )?;
    let plan = Rollback {
        version: FORMAT,
        operation,
        directory: lease.verify_directory()?,
        archive: DirectoryId::read(stage.path())?,
        destination: digest(profile)?,
        previous,
        restore: backup.hashes,
    };
    let intent = Intent {
        checksum: record::bytes_hash(&record::encode(&plan)?)?,
        rollback: plan,
    };
    let bytes = record::encode(&intent)?;
    record::publish_new(
        record::temp_bytes(stage.path(), &bytes)?,
        &stage.path().join(ARCHIVE),
    )?;
    record::sync_directory(stage.path())?;
    rsql_core::migration::rename_exclusive(stage.path(), &folder)?;
    let _ = stage.keep();
    record::sync_directory(profile)?;
    record::publish_new(record::temp_bytes(profile, &bytes)?, &profile.join(JOURNAL))?;
    record::sync_directory(profile)?;
    boundary("rollback admitted")?;
    resolve(&mut lease, intent)?;
    Ok(folder)
}

pub(super) fn resolve(lease: &mut ProfileLease, intent: Intent) -> Result<RecoveryOutcome> {
    let profile = lease.directory();
    let folder = intent.verify(profile)?;
    record::standalone(profile)?;
    let plan = intent.rollback;
    verify_members(&folder, &BACKUPS, plan.previous)?;
    verify_members(&folder, &RESTORE, plan.restore)?;
    super::prepare::previous_at(lease, &folder, &RESTORE)?;
    let live = record::live_hashes(profile)?;
    if !(0..LIVE.len()).all(|i| live[i] == plan.previous[i] || live[i] == plan.restore[i]) {
        return Err(ImportError::Integrity(
            "unexpected live data during rollback",
        ));
    }
    for (i, name) in LIVE.iter().enumerate() {
        if live[i] != plan.restore[i] {
            let path = profile.join(name);
            if let Some(hash) = plan.restore[i] {
                record::temp_copy(&folder, &folder.join(RESTORE[i]), hash)?
                    .persist(&path)
                    .map_err(|e| e.error)?;
            } else {
                fs::remove_file(&path)?;
            }
            record::sync_directory(profile)?;
            boundary("rollback member restored")?;
        }
    }
    super::prepare::previous(lease)?;
    super::recovery::finish(profile, plan.restore)?;
    Ok(RecoveryOutcome::Restored)
}
