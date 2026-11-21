use super::{MigrationManifest, Sha256Digest, SnapshotId};
use serde::{Deserialize, Serialize};

/// Identifies the exact exported snapshot. It carries no authority outside its producer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportIdentity {
    pub snapshot_id: SnapshotId,
    pub database: Sha256Digest,
    pub ui: Sha256Digest,
}

impl From<&MigrationManifest> for ExportIdentity {
    fn from(manifest: &MigrationManifest) -> Self {
        Self {
            snapshot_id: manifest.snapshot_id(),
            database: manifest.database_sha256(),
            ui: manifest.ui_sha256(),
        }
    }
}

/// Fixed-size metadata exchanged with a live native helper holding the destination lease.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportEvidence {
    pub source: ExportIdentity,
    pub database: Sha256Digest,
    pub workspace: Sha256Digest,
}

/// Identification echoed on this helper's pipe. Only its locally verified receipt and lease authorize commit.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommitIdentity {
    pub attempt: uuid::Uuid,
    pub activation: uuid::Uuid,
    pub directory: super::DirectoryId,
    pub destination: Sha256Digest,
    pub receipt: ImportEvidence,
}

pub const COMMIT_REJECTED_EXIT: u8 = 3;
pub const HANDOFF_FRAME_LIMIT: u64 = 2048;

pub fn read_frame<T: serde::de::DeserializeOwned>(
    input: impl std::io::BufRead,
) -> std::io::Result<T> {
    use std::io::BufRead;
    let mut bytes = Vec::new();
    input
        .take(HANDOFF_FRAME_LIMIT + 1)
        .read_until(b'\n', &mut bytes)?;
    if bytes.len() as u64 > HANDOFF_FRAME_LIMIT || bytes.pop() != Some(b'\n') {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    serde_json::from_slice(&bytes).map_err(|_| std::io::ErrorKind::InvalidData.into())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "status", deny_unknown_fields)]
pub enum NativeStatus {
    Prepared { receipt: ImportEvidence },
    Activated { receipt: ImportEvidence },
    Verified { receipt: ImportEvidence },
    CommitReady { commit: CommitIdentity },
    Complete,
    Committed,
    Rejected,
    Indeterminate,
    Failed { error: String },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "action", deny_unknown_fields)]
pub enum NativeDecision {
    Activate { receipt: ImportEvidence },
    Finish,
    Commit { commit: CommitIdentity },
}
