use super::{
    files::{check_file, create_private_dir, file_options},
    operation::{Result, SnapshotError},
};
use std::{
    fs::{File, TryLockError},
    path::{Path, PathBuf},
};

pub const SOURCE_HANDOFF_FILENAME: &str = "rsql.source-handoff.json";

pub const IMPORT_JOURNAL_FILENAME: &str = "rsql.import-journal.json";

/// Cooperating processes acquire this guard before workspace or database mutation.
/// Retain it (or an Arc clone) through the last usable service, database owner and admitted write.
pub struct ProfileLease {
    directory: PathBuf,
    identity: super::DirectoryId,
    lock: Option<File>,
    transfer: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Self>>>,
    // Fields drop in declaration order. Subscribers wake only after the file lock closes.
    released: tokio::sync::watch::Sender<()>,
}

impl ProfileLease {
    pub fn acquire(profile_path: impl AsRef<Path>) -> Result<Self> {
        let path = profile_path.as_ref().components().collect::<PathBuf>();
        create_private_dir(&path, true)?;
        let directory = path.canonicalize()?;
        let identity = super::DirectoryId::read(&directory)?;
        let path = directory.join("rsql.profile.lock");
        let mut options = file_options();
        options.write(true).create(true);
        #[cfg(windows)]
        std::os::windows::fs::OpenOptionsExt::share_mode(&mut options, 3);
        let lock = options.open(&path)?;
        check_lock(&lock, &path)?;
        match lock.try_lock() {
            Ok(()) => {
                let lease = Self {
                    directory,
                    identity,
                    lock: Some(lock),
                    transfer: Default::default(),
                    released: tokio::sync::watch::channel(()).0,
                };
                lease.require_source_editable()?;
                Ok(lease)
            }
            Err(TryLockError::WouldBlock) => Err(SnapshotError::ProfileInUse),
            Err(TryLockError::Error(error)) => Err(error.into()),
        }
    }

    /// Register a single successor before shutdown. It receives the same lock after the last
    /// shared database or workspace owner drops. The release signal still follows the file lock.
    pub fn after_last_owner(&self) -> Result<tokio::sync::oneshot::Receiver<Self>> {
        let mut transfer = self.transfer.lock().map_err(|_| SnapshotError::Worker)?;
        if transfer.is_some() {
            return Err(SnapshotError::ProfileInUse);
        }
        let (send, receive) = tokio::sync::oneshot::channel();
        *transfer = Some(send);
        Ok(receive)
    }

    pub fn released(&self) -> impl Future<Output = ()> + Send + use<> {
        let mut receiver = self.released.subscribe();
        async move {
            let _ = receiver.changed().await;
        }
    }

    /// Physical identity at acquisition, for excluding this directory from recovery copies.
    pub fn original_directory_id(&self) -> super::DirectoryId {
        self.identity
    }

    pub fn verify_directory(&self) -> Result<super::DirectoryId> {
        if super::DirectoryId::read(&self.directory)? != self.identity {
            return Err(SnapshotError::InvalidStage);
        }
        check_lock(
            self.lock.as_ref().ok_or(SnapshotError::InvalidSource)?,
            &self.directory.join("rsql.profile.lock"),
        )?;
        Ok(self.identity)
    }

    pub fn require_source_editable(&self) -> Result<()> {
        self.verify_directory()?;
        match std::fs::symlink_metadata(self.directory.join(SOURCE_HANDOFF_FILENAME)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
            Ok(_) => Err(SnapshotError::SourceHandoffPending),
        }
    }

    pub fn require_resolved_import(&self) -> Result<()> {
        self.require_source_editable()?;
        match std::fs::symlink_metadata(self.directory.join(IMPORT_JOURNAL_FILENAME)) {
            Ok(_) => Err(SnapshotError::RecoveryRequired),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for ProfileLease {
    fn drop(&mut self) {
        if let Some(send) = self
            .transfer
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let successor = Self {
                directory: std::mem::take(&mut self.directory),
                identity: self.identity,
                lock: self.lock.take(),
                transfer: Default::default(),
                released: self.released.clone(),
            };
            let _ = send.send(successor);
        }
    }
}

fn check_lock(lock: &File, path: &Path) -> Result<()> {
    let metadata = check_file(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = lock.metadata()?;
        if (metadata.dev(), metadata.ino()) != (opened.dev(), opened.ino()) {
            return Err(SnapshotError::InvalidSource);
        }
    }
    #[cfg(windows)]
    let _ = (lock, metadata); // The held Windows lock denies deletion and rename.
    Ok(())
}
