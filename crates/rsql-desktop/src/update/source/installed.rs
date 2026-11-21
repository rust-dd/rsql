#[cfg(any(windows, target_os = "linux"))]
use rsql_core::update::InstallationKind;
use rsql_core::update::TargetIdentity;
#[cfg(any(windows, target_os = "linux"))]
use semver::Version;
use std::path::Path;

#[cfg(windows)]
pub(super) fn installed_target(path: &Path, _: &str, _: &str) -> Option<TargetIdentity> {
    if std::env::consts::ARCH != "x86_64" {
        return None;
    }
    // NSIS writes its uninstaller next to the executable; MSI keeps its own in the Installer cache.
    let installation = if path.parent()?.join("uninstall.exe").is_file() {
        InstallationKind::Nsis
    } else {
        InstallationKind::Msi
    };
    Some(TargetIdentity {
        triple: "x86_64-pc-windows-msvc".into(),
        installation,
        os_version: windows_version()?,
        runtime_supported: true,
    })
}

#[cfg(windows)]
fn windows_version() -> Option<Version> {
    use windows::{
        Wdk::System::SystemServices::RtlGetVersion,
        Win32::System::SystemInformation::OSVERSIONINFOW,
    };
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // GetVersionEx reports a compatibility version to processes without an OS manifest.
    if unsafe { RtlGetVersion(&mut info) }.0 != 0 {
        return None;
    }
    Some(Version::new(
        info.dwMajorVersion.into(),
        info.dwMinorVersion.into(),
        info.dwBuildNumber.into(),
    ))
}

#[cfg(target_os = "linux")]
pub(super) fn installed_target(path: &Path, _: &str, _: &str) -> Option<TargetIdentity> {
    if std::env::consts::ARCH != "x86_64" {
        return None;
    }
    let installation = if crate::update::install::appimage::current(path).is_some() {
        InstallationKind::AppImage
    } else if owned_by(path, "dpkg", "-S") {
        InstallationKind::Deb
    } else if owned_by(path, "rpm", "-qf") {
        InstallationKind::Rpm
    } else {
        return None;
    };
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    Some(TargetIdentity {
        triple: "x86_64-unknown-linux-gnu".into(),
        installation,
        os_version: kernel_version(&release)?,
        runtime_supported: true,
    })
}

#[cfg(target_os = "linux")]
fn owned_by(path: &Path, manager: &str, query: &str) -> bool {
    use std::process::{Command, Stdio};
    Command::new(manager)
        .arg(query)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "linux")]
fn kernel_version(release: &str) -> Option<Version> {
    let numeric = release
        .trim()
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .next()?;
    let mut parts = numeric.split('.').map(str::parse::<u64>);
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some(Version::new(major, minor, patch))
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn installed_target(_: &Path, _: &str, _: &str) -> Option<TargetIdentity> {
    None
}
