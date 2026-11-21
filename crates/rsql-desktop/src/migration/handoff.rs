use super::{
    ActivatedReceipt, ImportError, PreparedReceipt, Result, activate::activate_closed, prepare,
    record,
};
use rsql_core::migration::{
    MANIFEST_FILENAME, MigrationManifest, ProfileLease, SnapshotError, handoff::ExportIdentity,
    read_bundle,
};
use std::path::Path;

fn export(bundle: &Path) -> Result<MigrationManifest> {
    match std::fs::symlink_metadata(bundle.join(MANIFEST_FILENAME)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ImportError::MissingExport);
        }
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    read_bundle(bundle).map_err(|error| match error {
        SnapshotError::UnsupportedFormat(_) => error.into(),
        _ => ImportError::InvalidExport,
    })
}

/// The CLI holds the same closed destination from verification until its consuming caller finishes.
pub fn verify_activation(bundle: &Path, lease: &ProfileLease) -> Result<ActivatedReceipt> {
    lease.require_resolved_import()?;
    let manifest = export(bundle)?;
    let installed = prepare::previous(lease)
        .map_err(|_| ImportError::InvalidReceipt)?
        .ok_or(ImportError::MissingReceipt)?;
    if installed.source != ExportIdentity::from(&manifest) {
        return Err(ImportError::Stale);
    }
    record::standalone(lease.directory())?;
    if record::live_hashes(lease.directory())? != installed.installed()? {
        return Err(ImportError::Stale);
    }
    Ok(ActivatedReceipt(installed, Vec::new()))
}

pub fn prepare_handoff(
    bundle: &Path,
    destination: &Path,
) -> Result<(PreparedReceipt, ProfileLease)> {
    let manifest = export(bundle)?;
    let bundle = bundle.canonicalize()?;
    let destination = super::destination::create_destination(&bundle, destination)?;
    let mut lease = ProfileLease::acquire(&destination)?;
    lease.require_resolved_import()?;
    let receipt = prepare::prepare_leased(&bundle, manifest, &mut lease, false, Vec::new())?;
    if receipt.database.is_none() {
        verify_activation(&bundle, &lease)?;
    }
    Ok((receipt, lease))
}

pub fn activate_handoff(
    bundle: &Path,
    receipt: PreparedReceipt,
    lease: ProfileLease,
) -> Result<ActivatedReceipt> {
    lease.require_resolved_import()?;
    if ExportIdentity::from(&export(bundle)?) != receipt.import_evidence().source {
        return Err(ImportError::Stale);
    }
    activate_closed(receipt, lease)
}
