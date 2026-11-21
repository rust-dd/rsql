use super::{Relaunch, VerifiedArtifact};
use flate2::bufread::GzDecoder;
use rsql_core::update::PackageFormat;
use std::{
    fs,
    io::{self, BufReader, Read},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

const BUNDLE: &str = "RSQL.app";
const EXECUTABLE: &str = "Contents/MacOS/rsql-desktop";
const BACKUP: &str = "previous.app";

pub(super) struct Staged {
    target: PathBuf,
    root: TempDir,
}

pub(super) fn stage(artifact: VerifiedArtifact) -> Result<Staged, String> {
    if artifact.candidate().artifact().format != PackageFormat::AppTarGz {
        return Err("This update package cannot be installed on macOS.".into());
    }
    let target = installed_bundle().ok_or("RSQL can only update its own RSQL.app bundle.")?;
    extract(artifact.into_reader(), &target)
        .map_err(|e| format!("Cannot prepare the update for {}: {e}", target.display()))
}

fn installed_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    is_bundle_executable(&exe)
        .then(|| exe.ancestors().nth(3).map(Path::to_path_buf))
        .flatten()
}

// Macs updated from 1.x by the Tauri updater keep the old lowercase `rsql.app` folder name.
pub(crate) fn is_bundle_executable(exe: &Path) -> bool {
    exe.ends_with(EXECUTABLE)
        && exe
            .ancestors()
            .nth(3)
            .and_then(Path::file_name)
            .is_some_and(|name| name.eq_ignore_ascii_case(BUNDLE))
}

fn extract(archive: impl Read, target: &Path) -> io::Result<Staged> {
    let volume = fs::symlink_metadata(target)?.dev();
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("the application has no parent folder"))?;
    // The swap is two renames, which only work within one volume.
    let base = [std::env::temp_dir(), parent.to_owned()]
        .into_iter()
        .find(|dir| fs::metadata(dir).is_ok_and(|m| m.dev() == volume))
        .ok_or_else(|| io::Error::other("no temporary folder on the application's volume"))?;
    let root = tempfile::Builder::new()
        .prefix(".rsql-update-")
        .tempdir_in(base)?;
    tar::Archive::new(GzDecoder::new(BufReader::new(archive))).unpack(root.path())?;
    if !root.path().join(BUNDLE).join(EXECUTABLE).is_file() {
        return Err(io::Error::other("the update has no application executable"));
    }
    Ok(Staged {
        target: target.to_owned(),
        root,
    })
}

impl Staged {
    pub(super) fn apply(self, relaunch: &Relaunch) -> Result<(), String> {
        let target = self.target.clone();
        self.replace()
            .map_err(|e| format!("Cannot replace {}: {e}", target.display()))?;
        relaunch.start()
    }

    fn replace(self) -> io::Result<()> {
        let staged = self.root.path().join(BUNDLE);
        let backup = self.root.path().join(BACKUP);
        let moved = match fs::rename(&self.target, &backup) {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                privileged(&self.target, &staged, &backup)
            }
            Err(e) => return Err(e),
            Ok(()) => fs::rename(&staged, &self.target).map_err(|error| {
                match fs::rename(&backup, &self.target) {
                    Ok(()) => error,
                    Err(restore) => io::Error::other(format!("{error}; restore failed: {restore}")),
                }
            }),
        };
        if let Err(error) = moved {
            if fs::symlink_metadata(&self.target).is_err() && backup.exists() {
                let kept = self.root.keep().join(BACKUP);
                return Err(io::Error::other(format!(
                    "{error}. The previous version is kept at {}",
                    kept.display()
                )));
            }
            return Err(error);
        }
        drop(self.root);
        // LaunchServices refreshes the bundle's cached version and icon from its modification time.
        let _ = Command::new("/usr/bin/touch").arg(&self.target).status();
        Ok(())
    }
}

/// Same moves as `replace`, run by the shell as an administrator when the app's folder is not writable.
fn privileged_script(target: &Path, staged: &Path, backup: &Path) -> String {
    let [target, staged, backup] = [target, staged, backup].map(shell_quote);
    format!(
        "mv -f {target} {backup} && {{ mv -f {staged} {target} || {{ mv -f {backup} {target}; exit 1; }}; }} && rm -rf {backup}"
    )
}

fn privileged(target: &Path, staged: &Path, backup: &Path) -> io::Result<()> {
    let script = privileged_script(target, staged, backup);
    let command = format!(
        "do shell script \"{}\" with administrator privileges",
        script.replace('\\', r"\\").replace('"', "\\\"")
    );
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", &command])
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "administrator authorization failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    ))
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}
