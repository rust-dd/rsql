use super::{ImportError, Result, record};
use rsql_core::migration::DirectoryId;
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

/// Resolve the existing prefix before creating any directories. A firmlink or bind mount
/// can preserve directory identity while giving the same directory two canonical paths.
pub(super) fn create_destination(bundle: &Path, requested: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(requested)?;
    let mut prefix = absolute.as_path();
    let mut missing = Vec::<OsString>::new();
    while !record::present(prefix)? {
        missing.push(prefix.file_name().ok_or(ImportError::Stale)?.into());
        prefix = prefix.parent().ok_or(ImportError::Stale)?;
    }
    if missing.is_empty() {
        DirectoryId::read(prefix)?;
    }
    let mut directory = prefix.canonicalize()?;
    let source_id = DirectoryId::read(bundle)?;
    for ancestor in directory.ancestors() {
        if DirectoryId::read(ancestor)? == source_id {
            return Err(ImportError::Integrity("source and destination overlap"));
        }
    }
    let source_ancestors = bundle
        .ancestors()
        .map(DirectoryId::read)
        .collect::<io::Result<Vec<_>>>()?;
    if missing.is_empty() && source_ancestors.contains(&DirectoryId::read(&directory)?) {
        return Err(ImportError::Integrity("source and destination overlap"));
    }
    // A previous attempt may have created this deepest prefix and failed its parent flush.
    if let Some(parent) = directory.parent() {
        record::sync_directory(parent)?;
    }
    for name in missing.into_iter().rev() {
        let child = directory.join(name);
        #[cfg(unix)]
        let created = {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(&child)
        };
        #[cfg(not(unix))]
        let created = fs::create_dir(&child);
        match created {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        super::boundary("destination directory created")?;
        if source_ancestors.contains(&DirectoryId::read(&child)?) {
            return Err(ImportError::Integrity("source and destination overlap"));
        }
        record::sync_directory(&directory)?;
        directory = child;
    }
    Ok(directory)
}
