mod activate;
pub mod cli;
mod destination;
pub mod handoff;
mod legacy;
mod prepare;
mod receipt;
mod record;
mod recovery;
pub mod review;
mod rollback;
pub use rollback::{Backup, backup_locations, inspect_backup, restore_backup};

pub use activate::activate_import;
pub use legacy::{ImportedDocuments, MigrationError, convert_legacy};
pub use prepare::{prepare_import, prepare_replacement};
pub use receipt::{ActivatedReceipt, PreparedReceipt};
pub use recovery::{RecoveryOutcome, recover};
pub use rsql_core::migration::LegacyUi;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error(transparent)]
    Snapshot(#[from] rsql_core::migration::SnapshotError),
    #[error(transparent)]
    Conversion(#[from] MigrationError),
    #[error("Migration file operation failed ({0})")]
    File(std::io::ErrorKind),
    #[error("Profile already contains data. Choose a new profile or review replacement.")]
    Conflict,
    #[error("No verified native activation for this profile")]
    MissingReceipt,
    #[error("The native activation receipt is invalid")]
    InvalidReceipt,
    #[error("The export bundle or manifest is missing")]
    MissingExport,
    #[error("The export manifest or database is invalid")]
    InvalidExport,
    #[error("Snapshot ID was reused for different source data")]
    ReusedSnapshot,
    #[error("Profile or stage changed since preparation")]
    Stale,
    #[error("Import recovery requires inspection: {0}")]
    Integrity(&'static str),
    #[error("Import worker did not return a result")]
    Worker,
}

impl From<std::io::Error> for ImportError {
    fn from(error: std::io::Error) -> Self {
        Self::File(error.kind())
    }
}

type Result<T> = std::result::Result<T, ImportError>;

fn boundary(_name: &'static str) -> Result<()> {
    Ok(())
}
