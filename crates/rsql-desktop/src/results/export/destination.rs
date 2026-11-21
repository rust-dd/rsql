use super::{check, incomplete_file};
use std::{
    fs::{File, TryLockError},
    io,
    path::Path,
    time::Duration,
};

pub(super) fn replace(path: &Path, current: &impl Fn() -> bool) -> io::Result<File> {
    replace_with_lock(path, current, File::try_lock)
}

pub(super) fn replace_with_lock(
    path: &Path,
    current: &impl Fn() -> bool,
    try_lock: impl Fn(&File) -> Result<(), TryLockError>,
) -> io::Result<File> {
    let (file, created) = match File::options().write(true).create_new(true).open(path) {
        Ok(file) => (file, true),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (open_existing(path)?, false),
        Err(e) => return Err(e),
    };
    lock_and_truncate(&file, current, try_lock)
        .map_err(|e| if created { incomplete_file(path, e) } else { e })?;
    Ok(file)
}

fn open_existing(path: &Path) -> io::Result<File> {
    let mut options = File::options();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound && path.is_symlink() {
            io::Error::other(
                "The selected link has no target. Choose the target filename directly.",
            )
        } else {
            io::Error::new(e.kind(), format!("Choose a writable regular file. {e}"))
        }
    })?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("Choose a regular file for export."));
    }
    Ok(file)
}

fn lock_and_truncate(
    file: &File,
    current: &impl Fn() -> bool,
    try_lock: impl Fn(&File) -> Result<(), TryLockError>,
) -> io::Result<()> {
    loop {
        check(current)?;
        match try_lock(file) {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(20)),
            Err(TryLockError::Error(e)) => return Err(e),
        }
    }
    check(current)?;
    file.set_len(0)
}

pub(super) fn create_new(path: &Path, current: &impl Fn() -> bool) -> io::Result<File> {
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // Acquire the same file lock as local exports before the selected name becomes visible.
    let mut builder = tempfile::Builder::new();
    builder.prefix(".rsql-export-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Match ordinary file creation, including the process umask.
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    let file = builder.tempfile_in(directory)?;
    file.as_file().try_lock()?;
    check(current)?;
    file.persist_noclobber(path).map_err(|failure| {
        let e = failure.error;
        let hint = if e.kind() == io::ErrorKind::AlreadyExists {
            " Choose a new filename."
        } else {
            ""
        };
        io::Error::new(e.kind(), format!("{}: {e}.{hint}", path.display()))
    })
}
