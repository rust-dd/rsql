use super::{Relaunch, VerifiedArtifact};
use rsql_core::update::PackageFormat;
use std::{
    fs,
    io::{self, Read},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use tempfile::TempPath;

pub(super) struct Staged {
    file: TempPath,
    target: PathBuf,
}

/// The AppImage file this process runs from, as reported by the AppImage runtime.
pub(in crate::update) fn current(exe: &Path) -> Option<PathBuf> {
    let mount = std::env::var_os("APPDIR")?;
    if !exe.starts_with(mount) {
        return None;
    }
    let path = PathBuf::from(std::env::var_os("APPIMAGE")?);
    (path.is_absolute() && path.is_file()).then_some(path)
}

pub(super) fn stage(artifact: VerifiedArtifact) -> Result<Staged, String> {
    if artifact.candidate().artifact().format != PackageFormat::AppImage {
        return Err("This update package cannot be installed on Linux.".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = current(&exe).ok_or("RSQL can only update itself when it runs as an AppImage.")?;
    write(artifact.into_reader(), &target)
        .map_err(|e| format!("Cannot write the update next to {}: {e}", target.display()))
}

fn write(mut source: impl Read, target: &Path) -> io::Result<Staged> {
    let dir = target
        .parent()
        .ok_or_else(|| io::Error::other("the AppImage has no parent folder"))?;
    let mut file = tempfile::Builder::new()
        .prefix(".rsql-update-")
        .suffix(".AppImage")
        .tempfile_in(dir)?;
    io::copy(&mut source, file.as_file_mut())?;
    let mode = fs::metadata(target)?.permissions().mode() & 0o777 | 0o111;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    file.as_file().sync_all()?;
    Ok(Staged {
        file: file.into_temp_path(),
        target: target.to_owned(),
    })
}

impl Staged {
    pub(super) fn apply(self, relaunch: &Relaunch) -> Result<(), String> {
        let target = self.target;
        self.file
            .persist(&target)
            .map_err(|e| format!("Cannot replace {}: {}", target.display(), e.error))?;
        relaunch.start()
    }
}
