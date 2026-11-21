use super::{
    Sha256Digest,
    operation::{Cancellation, Result, SnapshotError},
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{self, Read},
    path::Path,
};

pub(super) fn create_private_dir(path: &Path, recursive: bool) -> io::Result<()> {
    let mut options = DirBuilder::new();
    options.recursive(recursive);
    #[cfg(unix)]
    options.mode(0o700);
    options.create(path)
}

pub(super) fn file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    #[cfg(windows)]
    options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    options
}

pub(super) fn check_file(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(SnapshotError::InvalidSource);
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(SnapshotError::InvalidSource);
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || winapi_util::file::information(file_options().open(path)?)?.number_of_links() != 1
        {
            return Err(SnapshotError::InvalidSource);
        }
    }
    Ok(metadata)
}

/// Unix identity checks use stat without closing another descriptor for a live SQLite inode.
/// On Windows an open handle without FILE_SHARE_DELETE pins the name for the owner's lifetime.
pub(super) struct FileIdentity {
    #[cfg(unix)]
    id: (u64, u64),
    #[cfg(windows)]
    _pin: std::fs::File,
}

impl FileIdentity {
    pub fn capture(path: &Path) -> Result<Self> {
        let metadata = check_file(path)?;
        if metadata.len() == 0 {
            return Err(SnapshotError::InvalidSource);
        }
        Ok(Self {
            #[cfg(unix)]
            id: (metadata.dev(), metadata.ino()),
            #[cfg(windows)]
            _pin: file_options().share_mode(3).open(path)?,
        })
    }

    pub fn matches(&self, path: &Path) -> Result<bool> {
        let metadata = check_file(path)?;
        #[cfg(unix)]
        {
            Ok((metadata.dev(), metadata.ino()) == self.id)
        }
        #[cfg(not(unix))]
        Ok(metadata.len() > 0)
    }
}

pub(super) fn hash(reader: &mut impl Read, cancel: &Cancellation) -> Result<(u64, Sha256Digest)> {
    let mut digest = Sha256::new();
    let bytes = stream(reader, cancel, |chunk| {
        digest.update(chunk);
        Ok(())
    })?;
    Ok((bytes, Sha256Digest::from_hash(digest)))
}

pub(super) fn stream(
    reader: &mut impl Read,
    cancel: &Cancellation,
    mut consume: impl FnMut(&[u8]) -> io::Result<()>,
) -> Result<u64> {
    let mut buf = [0; 65536];
    let mut total = 0;
    loop {
        cancel.check()?;
        match reader.read(&mut buf) {
            Ok(0) => return Ok(total),
            Ok(n) => {
                consume(&buf[..n])?;
                total += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
}
