#[cfg(not(target_os = "macos"))]
mod installed;
#[cfg(not(target_os = "macos"))]
use installed::installed_target;

use super::{Candidate, transport::Transport, verify};
use minisign_verify::PublicKey;
use reqwest::Url;
use rsql_core::update::{
    CurrentRelease, Eligibility, InstallationKind, METADATA_LIMIT, MigrationState, NativeManifest,
    PackageFormat, SignedInventory, TargetIdentity, UpdateError, select_update,
};
use semver::Version;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::watch;

pub(super) struct Source {
    pub(super) http: Transport,
    pub(super) key: PublicKey,
    pub(super) current: CurrentRelease,
    pub(super) target: TargetIdentity,
    pub(super) feed: Url,
    pub(super) stage_parent: Option<PathBuf>,
}

pub(super) enum Outcome {
    Available(Candidate),
    NoUpdate(String),
    Guidance(String),
}

pub(super) fn packaged() -> Option<Source> {
    if !cfg!(feature = "production") || cfg!(debug_assertions) {
        return None;
    }
    let publisher = option_env!("TAURI_UPDATER_PUBLIC_KEY")?;
    let path = std::env::current_exe().ok()?;
    let version = env!("CARGO_PKG_VERSION");
    let revision = option_env!("RSQL_REVISION")?;
    for_package(&path, version, revision, publisher)
}

pub(super) fn for_package(
    path: &std::path::Path,
    version: &str,
    revision: &str,
    publisher: &str,
) -> Option<Source> {
    let target = installed_target(path, version, revision)?;
    let version = Version::parse(version).ok()?;
    let channel = version
        .pre
        .as_str()
        .split('.')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("stable")
        .to_owned();
    Some(Source {
        http: Transport::new().ok()?,
        key: verify::publisher(publisher).ok()?,
        current: CurrentRelease { version, channel },
        target,
        feed: Url::parse(
            "https://github.com/rust-dd/rust-sql/releases/latest/download/native-latest.json",
        )
        .ok()?,
        stage_parent: None,
    })
}

#[cfg(target_os = "macos")]
fn installed_target(
    path: &std::path::Path,
    version: &str,
    revision: &str,
) -> Option<TargetIdentity> {
    use objc2_foundation::NSProcessInfo;
    use std::io::Read;
    let path = path.canonicalize().ok()?;
    if !path.is_file() || !crate::update::install::bundle::is_bundle_executable(&path) {
        return None;
    }
    let info = path.parent()?.parent()?.join("Info.plist");
    if !info.symlink_metadata().ok()?.is_file() {
        return None;
    }
    let file = std::fs::File::open(&info).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take((METADATA_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    let floor = super::package::bundle(&bytes, version, revision).ok()?;
    let os = NSProcessInfo::processInfo().operatingSystemVersion();
    let os_version = Version::new(
        os.majorVersion.try_into().ok()?,
        os.minorVersion.try_into().ok()?,
        os.patchVersion.try_into().ok()?,
    );
    if os_version < floor {
        return None;
    }
    let triple = match std::env::consts::ARCH {
        "aarch64" => "aarch64-apple-darwin",
        "x86_64" => "x86_64-apple-darwin",
        _ => return None,
    };
    Some(TargetIdentity {
        triple: triple.into(),
        os_version,
        installation: InstallationKind::App,
        runtime_supported: true,
    })
}

impl Source {
    pub(super) async fn check(&self, stop: &mut watch::Receiver<bool>) -> Result<Outcome, String> {
        let mut bytes = Vec::new();
        self.http
            .read(
                &self.feed,
                METADATA_LIMIT as u64,
                None,
                stop,
                |chunk, _, _| {
                    bytes.extend_from_slice(chunk);
                    Ok(())
                },
            )
            .await?;
        let envelope = SignedInventory::parse(&bytes).map_err(|e| e.to_string())?;
        let sig = verify::signature(envelope.signature())?;
        self.key
            .verify(envelope.inventory(), &sig, false)
            .map_err(|_| "Update inventory publisher signature mismatch.")?;
        let manifest = NativeManifest::parse_inventory(envelope.inventory(), |s| {
            let sig =
                verify::signature(s).map_err(|_| UpdateError::Invalid("signature encoding"))?;
            self.key
                .verify_stream(&sig)
                .map_err(|_| UpdateError::Invalid("artifact publisher key or algorithm"))?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;
        verify::signed_version(&sig, manifest.version())?;
        let eligibility = select_update(
            &self.current,
            &self.target,
            MigrationState::Native,
            &manifest,
        )
        .map_err(|e| e.to_string())?;
        Ok(match eligibility {
            Eligibility::Available(c)
                if !matches!(
                    c.artifact().format,
                    PackageFormat::AppTarGz
                        | PackageFormat::Nsis
                        | PackageFormat::Msi
                        | PackageFormat::AppImage
                ) =>
            {
                return Err(
                    "Package identity verification is unavailable for this update format.".into(),
                );
            }
            Eligibility::Available(c) => Outcome::Available(Arc::from(c)),
            Eligibility::UpToDate => {
                Outcome::NoUpdate("You are already on the latest published version.".into())
            }
            Eligibility::PackageManager {
                version,
                installation,
            } => Outcome::Guidance(format!(
                "Version {version} is available. Download the new .{} package from the RSQL \
                 releases page and install it with your system package manager.",
                if installation == InstallationKind::Rpm {
                    "rpm"
                } else {
                    "deb"
                }
            )),
            _ => Outcome::NoUpdate("There is no compatible update for this installation.".into()),
        })
    }
}
