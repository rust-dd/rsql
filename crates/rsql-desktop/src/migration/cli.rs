use super::{ActivatedReceipt, ImportError, handoff};
use rsql_core::migration::{
    ProfileLease, SnapshotError,
    handoff::{COMMIT_REJECTED_EXIT, CommitIdentity, NativeDecision, NativeStatus, read_frame},
};
use std::{
    ffi::OsString,
    io::{self, Write},
    path::Path,
    process::ExitCode,
};

pub enum CommitOutcome {
    Committed,
    Rejected,
    Indeterminate,
}

pub fn dispatch(args: &[OsString]) -> Option<ExitCode> {
    dispatch_with_commit(args, |_, _| CommitOutcome::Rejected)
}

/// A package adapter must perform the transition in this process while the destination is leased.
/// Return Rejected only when the transition has had no effect.
pub fn dispatch_with_commit(
    args: &[OsString],
    commit: impl FnOnce(&ActivatedReceipt, &ProfileLease) -> CommitOutcome,
) -> Option<ExitCode> {
    let mode = args.first()?.to_str()?;
    if !mode.starts_with("--migration") {
        return None;
    }
    let result = match args {
        [_, bundle_flag, bundle, dest_flag, destination]
            if matches!(
                mode,
                "--migration-prepare" | "--migration-verify" | "--migration-consume"
            ) && bundle_flag == "--bundle"
                && dest_flag == "--profile-dir"
                && Path::new(bundle).is_absolute()
                && Path::new(destination).is_absolute() =>
        {
            operate(mode, bundle.as_ref(), destination.as_ref(), commit)
        }
        _ => Err(("arguments", 2)),
    };
    Some(match result {
        Ok(code) => ExitCode::from(code),
        Err((error, code)) => {
            let _ = emit(&NativeStatus::Failed {
                error: error.into(),
            });
            ExitCode::from(code)
        }
    })
}

type Failure = (&'static str, u8);

fn classify(error: ImportError) -> Failure {
    match error {
        ImportError::Snapshot(SnapshotError::RecoveryRequired) => ("recovery_required", 3),
        ImportError::Snapshot(SnapshotError::UnsupportedFormat(_)) => ("unknown_format", 3),
        ImportError::Snapshot(SnapshotError::SourceHandoffPending) => ("source_handoff_pending", 3),
        ImportError::Snapshot(SnapshotError::ProfileInUse) => ("profile_in_use", 3),
        ImportError::Conflict => ("conflict", 3),
        ImportError::MissingReceipt => ("missing_receipt", 3),
        ImportError::InvalidReceipt => ("invalid_receipt", 3),
        ImportError::MissingExport => ("missing_export", 3),
        ImportError::InvalidExport => ("invalid_export", 3),
        ImportError::Stale | ImportError::ReusedSnapshot => ("stale", 3),
        _ => ("invalid_migration", 3),
    }
}

fn emit(status: &NativeStatus) -> Result<(), Failure> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, status).map_err(|_| ("output", 4))?;
    out.write_all(b"\n")
        .and_then(|()| out.flush())
        .map_err(|_| ("output", 4))
}

fn operate(
    mode: &str,
    bundle: &Path,
    destination: &Path,
    commit: impl FnOnce(&ActivatedReceipt, &ProfileLease) -> CommitOutcome,
) -> Result<u8, Failure> {
    let rt = tokio::runtime::Runtime::new().map_err(|_| ("runtime", 4))?;
    let _runtime = rt.enter();
    let decision = || read_frame::<NativeDecision>(io::stdin().lock()).map_err(|_| ("protocol", 2));
    if mode == "--migration-prepare" {
        let (prepared, lease) = handoff::prepare_handoff(bundle, destination).map_err(classify)?;
        emit(&NativeStatus::Prepared {
            receipt: prepared.import_evidence(),
        })?;
        let NativeDecision::Activate { receipt } = decision()? else {
            return Err(("protocol", 2));
        };
        if receipt != prepared.import_evidence() {
            return Err(("stale", 3));
        }
        let activation = handoff::activate_handoff(bundle, prepared, lease).map_err(classify)?;
        emit(&NativeStatus::Activated {
            receipt: activation.import_evidence(),
        })?;
    } else {
        rsql_core::migration::DirectoryId::read(destination).map_err(|_| ("missing_receipt", 3))?;
        let lease = ProfileLease::acquire(destination).map_err(|e| classify(e.into()))?;
        let activation = handoff::verify_activation(bundle, &lease).map_err(classify)?;
        if mode == "--migration-verify" {
            emit(&NativeStatus::Verified {
                receipt: activation.import_evidence(),
            })?;
            if !matches!(decision()?, NativeDecision::Finish) {
                return Err(("protocol", 2));
            }
            emit(&NativeStatus::Complete)?;
        } else {
            let identity = CommitIdentity {
                attempt: uuid::Uuid::new_v4(),
                activation: activation.0.operation,
                directory: activation.0.directory,
                destination: activation.0.destination,
                receipt: activation.import_evidence(),
            };
            emit(&NativeStatus::CommitReady { commit: identity })?;
            if !matches!(decision()?, NativeDecision::Commit {commit} if commit == identity) {
                return Err(("protocol", 2));
            }
            let (status, exit) = match commit(&activation, &lease) {
                CommitOutcome::Committed => (NativeStatus::Committed, 0),
                CommitOutcome::Rejected => (NativeStatus::Rejected, COMMIT_REJECTED_EXIT),
                CommitOutcome::Indeterminate => (NativeStatus::Indeterminate, 4),
            };
            emit(&status)?;
            return Ok(exit);
        }
    }
    Ok(0)
}
