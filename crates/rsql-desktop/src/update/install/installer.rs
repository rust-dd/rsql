use super::{Relaunch, VerifiedArtifact};
use rsql_core::update::PackageFormat;
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read},
    os::windows::ffi::OsStrExt,
    path::PathBuf,
};
use tempfile::TempDir;

pub(super) struct Staged {
    dir: TempDir,
    file: PathBuf,
    msi: bool,
}

pub(super) fn stage(artifact: VerifiedArtifact) -> Result<Staged, String> {
    let candidate = artifact.candidate();
    let (msi, suffix) = match candidate.artifact().format {
        PackageFormat::Nsis => (false, "-setup.exe"),
        PackageFormat::Msi => (true, ".msi"),
        _ => return Err("This update package cannot be installed on Windows.".into()),
    };
    let name = format!("RSQL_{}_x64{suffix}", candidate.version());
    write(artifact.into_reader(), &name, msi)
        .map_err(|e| format!("Cannot prepare the update installer: {e}"))
}

fn write(mut source: impl Read, name: &str, msi: bool) -> io::Result<Staged> {
    let dir = tempfile::Builder::new().prefix("rsql-update-").tempdir()?;
    let file = dir.path().join(name);
    let mut out = std::fs::File::create_new(&file)?;
    io::copy(&mut source, &mut out)?;
    out.sync_all()?;
    Ok(Staged { dir, file, msi })
}

impl Staged {
    /// The installer replaces RSQL once this process exits.
    pub(super) fn apply(self, _: &Relaunch) -> Result<(), String> {
        let _ = self.dir.keep();
        if self.msi {
            let mut parameters = OsString::from("/i \"");
            parameters.push(&self.file);
            parameters.push("\" /passive /promptrestart");
            let system = std::env::var_os("SYSTEMROOT").unwrap_or_else(|| r"C:\Windows".into());
            let msiexec = PathBuf::from(system).join(r"System32\msiexec.exe");
            launch(msiexec.as_os_str(), &parameters)
        } else {
            // NSIS `/P` runs passively and `/R` starts RSQL again when it finishes.
            launch(self.file.as_os_str(), OsStr::new("/P /R /UPDATE"))
        }
    }
}

fn launch(program: &OsStr, parameters: &OsStr) -> Result<(), String> {
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOW},
        core::{PCWSTR, w},
    };
    let wide = |s: &OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let (program, parameters) = (wide(program), wide(parameters));
    // Unlike CreateProcess, ShellExecute shows the elevation prompt an installer may request.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(program.as_ptr()),
            PCWSTR(parameters.as_ptr()),
            PCWSTR::null(),
            SW_SHOW,
        )
    };
    let code = result.0 as isize;
    if code <= 32 {
        return Err(format!("Cannot start the update installer (error {code})."));
    }
    Ok(())
}
