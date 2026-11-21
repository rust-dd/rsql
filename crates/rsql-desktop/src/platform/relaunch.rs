use super::DesktopServices;
use gpui_kit::App;
use std::{ffi::OsString, path::PathBuf, process::Command};

#[derive(Clone)]
pub struct Relaunch {
    pub executable: PathBuf,
    pub arguments: Vec<OsString>,
}

impl DesktopServices {
    pub fn relaunch(cx: &App, arguments: Vec<OsString>) -> Result<Relaunch, String> {
        // AppKit reports a bundle directory, including for some unbundled Cargo launches.
        let executable = if cfg!(target_os = "macos") {
            std::env::current_exe().ok()
        } else {
            cx.app_path().ok()
        };
        #[cfg(target_os = "linux")]
        let executable = std::env::var_os("APPDIR")
            .filter(|mount| {
                executable
                    .as_ref()
                    .is_some_and(|path| path.starts_with(mount))
            })
            .and_then(|_| std::env::var_os("APPIMAGE"))
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or(executable);
        Ok(Relaunch {
            executable: executable.ok_or("Unable to locate running RSQL executable")?,
            arguments,
        })
    }
}

impl Relaunch {
    pub fn start(&self) -> Result<(), String> {
        Command::new(&self.executable)
            .args(&self.arguments)
            .spawn()
            .map(drop)
            .map_err(|error| format!("Restart {}: {error}", self.executable.display()))
    }
}
