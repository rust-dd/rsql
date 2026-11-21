use super::{
    DATABASE_FILENAME, Sha256Digest,
    files::{create_private_dir, file_options, hash},
    operation::{Cancellation, Result, SnapshotError},
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Seek},
    path::{Path, PathBuf},
};

pub const BACKUP_FILENAME: &str = "rsql.db.before-rekey.bak";

struct Entry {
    path: PathBuf,
    file: Option<File>,
    directory: bool,
    owns_path: bool,
    #[cfg(windows)]
    closed_identity: Option<(u64, u64)>,
}

impl Entry {
    fn create_file(path: PathBuf) -> Result<Self> {
        let mut options = file_options();
        options.write(true).create_new(true);
        #[cfg(windows)]
        std::os::windows::fs::OpenOptionsExt::share_mode(&mut options, 3);
        let file = options.open(&path)?;
        Ok(Self {
            path,
            file: Some(file),
            directory: false,
            owns_path: true,
            #[cfg(windows)]
            closed_identity: None,
        })
    }

    fn create_dir(path: PathBuf) -> Result<Self> {
        create_private_dir(&path, false)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::custom_flags(
            &mut options,
            libc::O_DIRECTORY | libc::O_NOFOLLOW,
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0200_0000).share_mode(3);
        }
        let file = options.open(&path).inspect_err(|_| {
            // The directory was claimed in the caller-owned parent, before creating any files.
            let _ = fs::remove_dir(&path);
        })?;
        Ok(Self {
            path,
            file: Some(file),
            directory: true,
            owns_path: true,
            #[cfg(windows)]
            closed_identity: None,
        })
    }

    fn file(&self) -> &File {
        self.file
            .as_ref()
            .expect("owned entry remains open until removal")
    }

    fn check_identity(&self) -> io::Result<()> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if (self.directory && !metadata.is_dir()) || (!self.directory && !metadata.is_file()) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let owned = self.file().metadata()?;
            if (metadata.dev(), metadata.ino()) != (owned.dev(), owned.ino()) {
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let named = OpenOptions::new()
                .read(true)
                .custom_flags(0x0200_0000 | 0x0020_0000)
                .open(&self.path)?;
            let named = winapi_util::file::information(named)?;
            let owned = match self.closed_identity {
                Some(id) => id,
                None => self.handle_identity()?,
            };
            if owned != (named.volume_serial_number(), named.file_index()) {
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        Ok(())
    }

    #[cfg(windows)]
    fn handle_identity(&self) -> io::Result<(u64, u64)> {
        let info = winapi_util::file::information(self.file())?;
        Ok((info.volume_serial_number(), info.file_index()))
    }

    #[cfg(windows)]
    fn close_for_rename(&mut self) -> io::Result<()> {
        // NTFS requires closed descendants to move the directory. Cleanup still owns this identity.
        self.closed_identity = Some(self.handle_identity()?);
        self.file.take();
        Ok(())
    }

    fn disarm(&mut self) {
        self.owns_path = false;
        self.file.take();
    }

    fn remove(&mut self) -> io::Result<()> {
        if !self.owns_path {
            return Ok(());
        }
        self.check_identity()?;
        self.disarm();
        if self.directory {
            fs::remove_dir(&self.path)
        } else {
            fs::remove_file(&self.path)
        }
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

/// Fields drop in declaration order so only our files are unlinked before the directory.
/// Pinning each created entry also protects a replacement at the same path from cleanup.
pub(super) struct Stage {
    database: Entry,
    backup: Option<Entry>,
    manifest: Option<Entry>,
    import_files: Vec<Entry>,
    directory: Entry,
}

impl Stage {
    pub fn create(requested: &Path, source_directory: &Path) -> Result<Self> {
        let name = requested.file_name().ok_or(SnapshotError::InvalidStage)?;
        let parent = std::path::absolute(requested)?
            .parent()
            .ok_or(SnapshotError::InvalidStage)?
            .canonicalize()?;
        let source = super::DirectoryId::read(source_directory)?;
        for ancestor in parent.ancestors() {
            if super::DirectoryId::read(ancestor)? == source {
                return Err(SnapshotError::InvalidStage);
            }
        }
        let directory = Entry::create_dir(parent.join(name))?;
        let database = Entry::create_file(directory.path.join(DATABASE_FILENAME))?;
        Ok(Self {
            database,
            backup: None,
            manifest: None,
            import_files: Vec::new(),
            directory,
        })
    }

    pub fn path(&self) -> &Path {
        &self.database.path
    }

    /// Close the retained filesystem handles and transfer cleanup to the caller.
    pub fn into_directory(mut self) -> PathBuf {
        self.disarm();
        self.directory.path.clone()
    }

    fn disarm(&mut self) {
        self.database.disarm();
        if let Some(backup) = &mut self.backup {
            backup.disarm();
        }
        if let Some(manifest) = &mut self.manifest {
            manifest.disarm();
        }
        for entry in &mut self.import_files {
            entry.disarm();
        }
        self.directory.disarm();
    }

    pub fn verify_import_entries(&self) -> Result<()> {
        self.directory.check_identity()?;
        let mut expected = std::collections::BTreeSet::new();
        for entry in std::iter::once(&self.database)
            .chain(&self.backup)
            .chain(&self.import_files)
        {
            entry.check_identity()?;
            expected.insert(entry.path.file_name().ok_or(SnapshotError::InvalidStage)?);
        }
        for child in fs::read_dir(&self.directory.path)? {
            if !expected.remove(child?.file_name().as_os_str()) {
                return Err(SnapshotError::InvalidStage);
            }
        }
        if !expected.is_empty() {
            return Err(SnapshotError::InvalidStage);
        }
        Ok(())
    }

    pub fn write_import_file(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        self.write_import_with(name, |mut file| {
            use std::io::Write;
            file.write_all(bytes)?;
            file.sync_all()
        })
    }

    fn write_import_with(
        &mut self,
        name: &str,
        write_synced: impl FnOnce(&File) -> io::Result<()>,
    ) -> Result<()> {
        if !matches!(name, "workspace.json" | "prepared.json") {
            return Err(SnapshotError::InvalidStage);
        }
        self.directory.check_identity()?;
        let entry = Entry::create_file(self.directory.path.join(name))?;
        write_synced(entry.file())?;
        self.import_files.push(entry);
        #[cfg(unix)]
        self.directory.file().sync_all()?;
        Ok(())
    }

    pub fn write_manifest(&mut self, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        let entry = Entry::create_file(self.directory.path.join(super::MANIFEST_FILENAME))?;
        let mut file = self.manifest.insert(entry).file();
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    }

    pub fn publish(self, manifest: &super::MigrationManifest) -> Result<PathBuf> {
        self.publish_with(manifest, sync_and_hash, sync_published_parent)
    }

    fn publish_with(
        mut self,
        manifest: &super::MigrationManifest,
        final_hash: impl FnOnce(&File, &Cancellation) -> Result<(u64, Sha256Digest)>,
        sync_parent: impl FnOnce(&Path) -> io::Result<()>,
    ) -> Result<PathBuf> {
        let parent = self
            .directory
            .path
            .parent()
            .ok_or(SnapshotError::InvalidStage)?;
        let destination = parent.join(format!("RSQL-export-{}", manifest.snapshot_id()));
        if self.finish_with(&Cancellation::default(), final_hash)?
            != (manifest.database_byte_len(), manifest.database_sha256())
        {
            return Err(SnapshotError::ContentMismatch);
        }
        let entry = self
            .manifest
            .as_ref()
            .ok_or(SnapshotError::ContentMismatch)?;
        let mut file = entry.file();
        file.rewind()?;
        let actual =
            super::MigrationManifest::read(file).map_err(|_| SnapshotError::ContentMismatch)?;
        if actual != *manifest {
            return Err(SnapshotError::ContentMismatch);
        }
        #[cfg(windows)]
        {
            self.database.close_for_rename()?;
            self.manifest.as_mut().unwrap().close_for_rename()?;
            self.directory.close_for_rename()?;
        }
        self.check_bundle_entries()?;
        super::publish::rename_exclusive(&self.directory.path, &destination)?;
        self.disarm();
        sync_parent(&destination).map_err(|err| SnapshotError::PublishedDurability {
            path: destination.clone(),
            kind: err.kind(),
        })?;
        Ok(destination)
    }

    fn check_bundle_entries(&self) -> Result<()> {
        self.directory.check_identity()?;
        let mut required = std::collections::BTreeSet::from([
            std::ffi::OsStr::new(DATABASE_FILENAME),
            std::ffi::OsStr::new(super::MANIFEST_FILENAME),
        ]);
        for child in fs::read_dir(&self.directory.path)? {
            if !required.remove(child?.file_name().as_os_str()) {
                return Err(SnapshotError::ContentMismatch);
            }
        }
        if !required.is_empty() {
            return Err(SnapshotError::ContentMismatch);
        }
        let manifest = self
            .manifest
            .as_ref()
            .ok_or(SnapshotError::ContentMismatch)?;
        for entry in [&self.database, manifest] {
            entry.check_identity()?;
            super::files::check_file(&entry.path)?;
        }
        Ok(())
    }

    pub fn reserve_backup(&mut self) -> Result<&Path> {
        let backup = Entry::create_file(self.directory.path.join(BACKUP_FILENAME))?;
        Ok(&self.backup.insert(backup).path)
    }

    pub fn drop_unused_backup(&mut self) -> Result<()> {
        if let Some(mut entry) = self.backup.take() {
            entry.remove()?;
        }
        Ok(())
    }

    pub fn finish(&self, cancel: &Cancellation) -> Result<(u64, Sha256Digest)> {
        self.finish_with(cancel, sync_and_hash)
    }

    fn finish_with(
        &self,
        cancel: &Cancellation,
        hash_database: impl FnOnce(&File, &Cancellation) -> Result<(u64, Sha256Digest)>,
    ) -> Result<(u64, Sha256Digest)> {
        self.directory.check_identity()?;
        self.database.check_identity()?;
        if let Some(backup) = &self.backup {
            backup.check_identity()?;
            backup.file().sync_all()?;
        }
        let content = hash_database(self.database.file(), cancel)?;
        #[cfg(unix)]
        {
            self.directory.file().sync_all()?;
            File::open(
                self.directory
                    .path
                    .parent()
                    .ok_or(SnapshotError::InvalidStage)?,
            )?
            .sync_all()?;
        }
        Ok(content)
    }
}

fn sync_published_parent(destination: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(destination.parent().ok_or(io::ErrorKind::InvalidInput)?)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = destination;
    Ok(())
}

fn sync_and_hash(mut file: &File, cancel: &Cancellation) -> Result<(u64, Sha256Digest)> {
    file.sync_all()?;
    file.rewind()?;
    hash(&mut file, cancel)
}
