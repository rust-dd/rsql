use super::{MigrationManifest, Sha256Digest, SnapshotError, files};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

/// Identity of a directory on its volume. Reject reparse points and symbolic links.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct DirectoryId(u64, u64);

impl DirectoryId {
    pub fn read(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self(metadata.dev(), metadata.ino()))
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            let directory = fs::OpenOptions::new()
                .read(true)
                .custom_flags(0x0200_0000 | 0x0020_0000)
                .open(path)?;
            let info = winapi_util::file::information(directory)?;
            Ok(Self(info.volume_serial_number(), info.file_index()))
        }
    }
}

pub fn open_regular(path: &Path) -> Result<File, SnapshotError> {
    let metadata = files::check_file(path)?;
    let file = files::file_options().open(path)?;
    if !file.metadata()?.is_file() {
        return Err(SnapshotError::InvalidSource);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = file.metadata()?;
        if (metadata.dev(), metadata.ino(), 1) != (opened.dev(), opened.ino(), opened.nlink()) {
            return Err(SnapshotError::InvalidSource);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if file.metadata()?.file_attributes() & 0x400 != 0
            || metadata.file_attributes() & 0x400 != 0
            || winapi_util::file::information(&file)?.number_of_links() != 1
        {
            return Err(SnapshotError::InvalidSource);
        }
    }
    Ok(file)
}

impl Sha256Digest {
    pub fn read(mut reader: impl Read) -> Result<(u64, Self), SnapshotError> {
        files::hash(&mut reader, &Default::default())
    }
}

/// Validate the fixed bundle inventory, bounded manifest, UI and database bytes.
pub fn read_bundle(path: &Path) -> Result<MigrationManifest, SnapshotError> {
    let directory = DirectoryId::read(path)?;
    check_inventory(path)?;
    let read_manifest = || {
        let file =
            open_regular(&path.join(super::MANIFEST_FILENAME)).map_err(|error| match error {
                SnapshotError::InvalidSource => {
                    SnapshotError::Manifest(super::ManifestError::InvalidManifest)
                }
                SnapshotError::Io(kind) => {
                    SnapshotError::Manifest(super::ManifestError::Read(kind))
                }
                other => other,
            })?;
        MigrationManifest::read(file).map_err(|error| match error {
            super::ManifestError::UnsupportedFormat(version) => {
                SnapshotError::UnsupportedFormat(version)
            }
            super::ManifestError::ManifestTooLarge => SnapshotError::ManifestTooLarge,
            other => SnapshotError::Manifest(other),
        })
    };
    let manifest = read_manifest()?;
    let database_path = path.join(super::DATABASE_FILENAME);
    let identity = files::FileIdentity::capture(&database_path)?;
    let database = open_regular(&database_path)?;
    let modified = database.metadata()?.modified()?;
    if database.metadata()?.len() != manifest.database_byte_len()
        || Sha256Digest::read(database.take(manifest.database_byte_len().saturating_add(1)))?
            != (manifest.database_byte_len(), manifest.database_sha256())
        || !identity.matches(&database_path)?
        || fs::symlink_metadata(&database_path)?.modified()? != modified
        || read_manifest()? != manifest
        || DirectoryId::read(path)? != directory
    {
        return Err(SnapshotError::ContentMismatch);
    }
    check_inventory(path)?;
    Ok(manifest)
}

fn check_inventory(path: &Path) -> Result<(), SnapshotError> {
    let mut required = std::collections::BTreeSet::from([
        std::ffi::OsStr::new(super::DATABASE_FILENAME),
        std::ffi::OsStr::new(super::MANIFEST_FILENAME),
    ]);
    for entry in fs::read_dir(path)? {
        if !required.remove(entry?.file_name().as_os_str()) {
            return Err(SnapshotError::InvalidSource);
        }
    }
    if !required.is_empty() {
        return Err(SnapshotError::InvalidSource);
    }
    Ok(())
}
