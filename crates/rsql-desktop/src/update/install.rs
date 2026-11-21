#[cfg(target_os = "linux")]
pub(super) mod appimage;
#[cfg(target_os = "macos")]
pub(super) mod bundle;
#[cfg(windows)]
mod installer;

#[cfg(target_os = "linux")]
use appimage as platform;
#[cfg(target_os = "macos")]
use bundle as platform;
#[cfg(windows)]
use installer as platform;

use super::VerifiedArtifact;
use crate::platform::Relaunch;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    use super::*;

    pub(super) enum Staged {}

    pub(super) fn stage(_: VerifiedArtifact) -> Result<Staged, String> {
        Err("Updates cannot be installed on this system.".into())
    }

    impl Staged {
        pub(super) fn apply(self, _: &Relaunch) -> Result<(), String> {
            match self {}
        }
    }
}

/// A verified update written to disk, ready to replace the running installation.
pub struct Staged(platform::Staged);

/// Blocking. Leaves the current installation untouched; dropping the result removes the staged files.
pub fn stage(artifact: VerifiedArtifact) -> Result<Staged, String> {
    platform::stage(artifact).map(Staged)
}

impl Staged {
    /// Blocking. Run only after the profile is saved and released, since it replaces or relaunches the app.
    pub fn apply(self, relaunch: &Relaunch) -> Result<(), String> {
        self.0.apply(relaunch)
    }
}
