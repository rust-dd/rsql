use std::{
    future::Future,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{runtime::Handle, task::JoinHandle};

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error(
        "Open this profile in native RSQL to resolve its pending import before opening services"
    )]
    RecoveryRequired,
    #[error(
        "This source profile is reserved for a native handoff. Keep the export and check the prepared native profile before resuming edits."
    )]
    SourceHandoffPending,
    #[error("Close the other RSQL process using this profile, then try again")]
    ProfileInUse,
    #[error("Expected a regular, nonempty RSQL database file")]
    InvalidSource,
    #[error("Choose a new staging directory outside the source directory")]
    InvalidStage,
    #[error("The snapshot database length or SHA-256 differs from the expected content")]
    ContentMismatch,
    #[error(transparent)]
    Manifest(super::ManifestError),
    #[error("Unknown migration manifest format {0}")]
    UnsupportedFormat(u64),
    #[error("Migration manifest exceeds 64 MiB")]
    ManifestTooLarge,
    #[error(
        "Bundle published at {path}, but directory durability is uncertain ({kind}). Keep this copy for inspection and retry export to create a fresh verified bundle."
    )]
    PublishedDurability {
        path: std::path::PathBuf,
        kind: io::ErrorKind,
    },
    #[error("The database failed its integrity check")]
    Integrity,
    #[error("The source database changed during export")]
    SourceChanged,
    #[error("The staged database still has outstanding journal writes")]
    UnfinishedJournal,
    #[error("Database operation failed during {0}")]
    Database(&'static str),
    #[error("Migration file operation failed ({0})")]
    Io(io::ErrorKind),
    #[error("Database operation was canceled")]
    Canceled,
    #[error("Database worker did not return a result")]
    Worker,
}

impl From<io::Error> for SnapshotError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

pub(super) type Result<T> = std::result::Result<T, SnapshotError>;

#[derive(Default, Clone)]
pub(super) struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire) {
            Err(SnapshotError::Canceled)
        } else {
            Ok(())
        }
    }
}

/// An admitted blocking worker owns its input and stage through completion.
/// Dropping the handle requests cancellation. Keep it and join to observe completion.
pub struct DatabaseOperation<T> {
    cancel: Cancellation,
    worker: Option<JoinHandle<Result<T>>>,
}

impl<T: Send + 'static> DatabaseOperation<T> {
    pub(super) fn start<F: Future<Output = Result<T>>>(
        run: impl FnOnce(Cancellation) -> F + Send + 'static,
    ) -> Self {
        let cancel = Cancellation::default();
        let signal = cancel.clone();
        let worker = tokio::task::spawn_blocking(move || {
            signal.check()?;
            let value = Handle::current().block_on(run(signal.clone()))?;
            signal.check()?;
            Ok(value)
        });
        Self {
            cancel,
            worker: Some(worker),
        }
    }

    /// Works between chunks and SQLite calls. A running SQLite call finishes on its worker.
    pub fn cancel(&self) {
        self.cancel.0.store(true, Ordering::Release);
    }

    /// Canceling this await preserves the handle for a subsequent join.
    pub async fn join(&mut self) -> Result<T> {
        let result = self.worker.as_mut().ok_or(SnapshotError::Worker)?.await;
        self.worker.take();
        let value = result.map_err(|_| SnapshotError::Worker)??;
        self.cancel.check()?;
        Ok(value)
    }
}

impl<T> Drop for DatabaseOperation<T> {
    fn drop(&mut self) {
        self.cancel.0.store(true, Ordering::Release);
    }
}
