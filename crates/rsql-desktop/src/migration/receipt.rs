use super::record::Record;
use rsql_core::migration::{PreparedDatabase, Sha256Digest, SnapshotId};

/// Privately minted after strict database migration and exact workspace round-trip, or after
/// validating this same source's existing activation. Dropping an unused stage releases it.
///
/// ```compile_fail
/// use rsql_desktop::migration::PreparedReceipt;
/// let _: PreparedReceipt = serde_json::from_str("{}").unwrap();
/// ```
pub struct PreparedReceipt {
    pub(super) record: Record,
    pub(super) database: Option<PreparedDatabase>,
    pub(super) backup_warnings: Vec<std::path::PathBuf>,
}

/// Evidence for a completed, verified profile activation.
///
/// ```compile_fail
/// use rsql_desktop::migration::ActivatedReceipt;
/// let _: ActivatedReceipt = serde_json::from_str("{}").unwrap();
/// ```
///
/// ```compile_fail
/// use rsql_desktop::migration::{ActivatedReceipt, PreparedReceipt};
/// fn requires_preparation(_: &PreparedReceipt) {}
/// fn check(receipt: &ActivatedReceipt) { requires_preparation(receipt); }
/// ```
pub struct ActivatedReceipt(pub(super) Record, pub(super) Vec<std::path::PathBuf>);

macro_rules! source_accessors {
    () => {
        pub fn import_evidence(&self) -> rsql_core::migration::handoff::ImportEvidence {
            let record = self.evidence();
            rsql_core::migration::handoff::ImportEvidence {
                source: record.source,
                database: record.database,
                workspace: record.workspace,
            }
        }

        pub fn durable_tabs(&self) -> usize {
            self.evidence().durable_tabs
        }
        pub fn discarded_tabs(&self) -> usize {
            self.evidence().discarded_tabs
        }
        pub fn snapshot_id(&self) -> SnapshotId {
            self.evidence().source.snapshot_id
        }
        pub fn source_database_sha256(&self) -> Sha256Digest {
            self.evidence().source.database
        }
        pub fn source_ui_sha256(&self) -> Sha256Digest {
            self.evidence().source.ui
        }
        pub fn staged_database_sha256(&self) -> Sha256Digest {
            self.evidence().database
        }
        pub fn workspace_sha256(&self) -> Sha256Digest {
            self.evidence().workspace
        }
    };
}

impl PreparedReceipt {
    pub fn unverified_backups(&self) -> &[std::path::PathBuf] {
        &self.backup_warnings
    }

    fn evidence(&self) -> &Record {
        &self.record
    }
    source_accessors!();
}

impl ActivatedReceipt {
    pub fn unverified_backups(&self) -> &[std::path::PathBuf] {
        &self.1
    }

    fn evidence(&self) -> &Record {
        &self.0
    }
    source_accessors!();
}
