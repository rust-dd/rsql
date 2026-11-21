use super::{Artifact, InstallationKind, NativeManifest, UpdateError};
use crate::migration::handoff::{ExportIdentity, ImportEvidence};
use semver::Version;

pub struct CurrentRelease {
    pub version: Version,
    pub channel: String,
}

/// Supplied by the local installation owner. The runtime check includes actual
/// platform support, including Linux graphical/Vulkan/GTK availability.
pub struct TargetIdentity {
    pub triple: String,
    pub installation: InstallationKind,
    pub os_version: Version,
    pub runtime_supported: bool,
}

/// Local observations only. Matching copies describe a candidate for review.
/// Before transition the M10 producers must revalidate the current source and
/// verified activation while holding their original leases.
pub enum MigrationState<'a> {
    PreBridge,
    Bridge {
        current: Option<&'a ExportIdentity>,
        prepared: Option<&'a ImportEvidence>,
        verified: Option<&'a ImportEvidence>,
    },
    Native,
}

/// Immutable description for review. Download and installation revalidate the
/// publisher, artifact identity and current installation before using it.
#[derive(Clone, Debug)]
pub struct ReleaseCandidate {
    version: Version,
    revision: String,
    channel: String,
    notes: String,
    date: String,
    artifact: Artifact,
}
impl ReleaseCandidate {
    pub fn version(&self) -> &Version {
        &self.version
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn channel(&self) -> &str {
        &self.channel
    }
    pub fn notes(&self) -> &str {
        &self.notes
    }
    pub fn date(&self) -> &str {
        &self.date
    }
    pub fn artifact(&self) -> &Artifact {
        &self.artifact
    }
}

#[derive(Debug)]
pub enum Eligibility {
    UpToDate,
    DifferentChannel,
    Unsupported,
    MigrationRequired,
    PackageManager {
        version: Version,
        installation: InstallationKind,
    },
    Available(Box<ReleaseCandidate>),
}

pub fn select_update(
    current: &CurrentRelease,
    target: &TargetIdentity,
    migration: MigrationState<'_>,
    manifest: &NativeManifest,
) -> Result<Eligibility, UpdateError> {
    if current.channel.is_empty()
        || !current
            .channel
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(UpdateError::Invalid("current release channel"));
    }
    if manifest.channel() != current.channel {
        return Ok(Eligibility::DifferentChannel);
    }
    if !manifest.version().cmp_precedence(&current.version).is_gt() {
        return Ok(Eligibility::UpToDate);
    }
    let Some(artifact) = manifest
        .artifacts()
        .iter()
        .find(|a| a.target == target.triple && a.installation == target.installation)
    else {
        return Ok(Eligibility::Unsupported);
    };
    if !target.runtime_supported
        || target
            .os_version
            .cmp_precedence(&artifact.minimum_os)
            .is_lt()
    {
        return Ok(Eligibility::Unsupported);
    }
    match migration {
        MigrationState::Native => {}
        MigrationState::Bridge {
            current: Some(source),
            prepared: Some(p),
            verified: Some(v),
        } if p == v && p.source == *source => {}
        _ => return Ok(Eligibility::MigrationRequired),
    }
    if matches!(
        target.installation,
        InstallationKind::Deb | InstallationKind::Rpm
    ) {
        return Ok(Eligibility::PackageManager {
            version: manifest.version().clone(),
            installation: target.installation,
        });
    }
    Ok(Eligibility::Available(Box::new(ReleaseCandidate {
        version: manifest.version().clone(),
        revision: manifest.revision().into(),
        channel: manifest.channel().into(),
        notes: manifest.notes().into(),
        date: manifest.date().into(),
        artifact: artifact.clone(),
    })))
}
