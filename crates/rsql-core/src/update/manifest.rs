use super::JsonObject;
use crate::migration::Sha256Digest;
use semver::Version;
use serde::Deserialize;
use std::collections::HashSet;
use url::Url;

pub const METADATA_LIMIT: usize = 1024 * 1024;
pub const ARTIFACT_LIMIT: u64 = 512 * 1024 * 1024;
pub const APP_ID: &str = "com.rust-dd.rsql";

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("release metadata exceeds 1 MiB")]
    Oversized,
    #[error("malformed release JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid release field: {0}")]
    Invalid(&'static str),
}

fn bounded_record<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, UpdateError> {
    if bytes.len() > METADATA_LIMIT {
        return Err(UpdateError::Oversized);
    }
    let JsonObject(record) = serde_json::from_slice(bytes)?;
    Ok(record)
}

// The outer JSON string preserves every signed UTF-8 byte, including whitespace.
// Only a verifier with an independently configured publisher key can authenticate it.
#[derive(Debug)]
pub struct SignedInventory {
    inventory: String,
    signature: String,
}

impl SignedInventory {
    pub fn parse(bytes: &[u8]) -> Result<Self, UpdateError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Envelope {
            inventory: String,
            signature: String,
        }
        let Envelope {
            inventory,
            signature,
        } = bounded_record(bytes)?;
        let envelope = Self {
            inventory,
            signature,
        };
        if envelope.inventory.is_empty() || !(1..=8192).contains(&envelope.signature.len()) {
            return Err(UpdateError::Invalid("signed inventory"));
        }
        Ok(envelope)
    }
    pub fn inventory(&self) -> &[u8] {
        self.inventory.as_bytes()
    }
    pub fn signature(&self) -> &str {
        &self.signature
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InstallationKind {
    App,
    Nsis,
    Msi,
    AppImage,
    Deb,
    Rpm,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PackageFormat {
    Dmg,
    #[serde(rename = "app-tar-gz")]
    AppTarGz,
    Nsis,
    Msi,
    AppImage,
    Deb,
    Rpm,
}

impl PackageFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Dmg => ".dmg",
            Self::AppTarGz => ".app.tar.gz",
            Self::Nsis => ".exe",
            Self::Msi => ".msi",
            Self::AppImage => ".AppImage",
            Self::Deb => ".deb",
            Self::Rpm => ".rpm",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub target: String,
    pub installation: InstallationKind,
    pub format: PackageFormat,
    pub minimum_os: Version,
    pub url: String,
    pub bytes: u64,
    pub sha256: Sha256Digest,
    pub signature: String,
}

impl Artifact {
    fn valid_target(&self) -> bool {
        use InstallationKind as Install;
        use PackageFormat as Format;
        match (self.target.as_str(), self.installation, self.format) {
            (
                "aarch64-apple-darwin" | "x86_64-apple-darwin",
                Install::App,
                Format::Dmg | Format::AppTarGz,
            ) => self.minimum_os.major >= 15,
            ("x86_64-pc-windows-msvc", Install::Msi, Format::Msi)
            | ("x86_64-pc-windows-msvc", Install::Nsis, Format::Nsis) => {
                self.minimum_os.major >= 10
            }
            ("x86_64-unknown-linux-gnu", Install::AppImage, Format::AppImage)
            | ("x86_64-unknown-linux-gnu", Install::Deb, Format::Deb)
            | ("x86_64-unknown-linux-gnu", Install::Rpm, Format::Rpm) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    format: u32,
    app: String,
    channel: String,
    version: Version,
    revision: String,
    notes: String,
    date: String,
    #[serde(deserialize_with = "artifact_records")]
    artifacts: Vec<Artifact>,
}

fn artifact_records<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Artifact>, D::Error> {
    let records = Vec::<JsonObject<Artifact>>::deserialize(deserializer)?;
    Ok(records
        .into_iter()
        .map(|JsonObject(artifact)| artifact)
        .collect())
}

/// Structurally validated inventory. A caller consuming a remote feed must first
/// authenticate the exact signed bytes, and reverify any selected bytes on download.
#[derive(Debug)]
pub struct NativeManifest(Inventory);

impl NativeManifest {
    /// The verifier owns signature decoding, the trusted key and the supported
    /// algorithm. This check can reject a malformed artifact signature before
    /// selection. Full payload verification happens after the subsequent download.
    pub fn parse_inventory(
        bytes: &[u8],
        check_signature: impl Fn(&str) -> Result<(), UpdateError>,
    ) -> Result<Self, UpdateError> {
        let m: Inventory = bounded_record(bytes)?;
        if m.format != 1 || m.app != APP_ID {
            return Err(UpdateError::Invalid("format or app"));
        }
        let prerelease = m.version.pre.as_str();
        if (prerelease.is_empty() && m.channel != "stable")
            || (!prerelease.is_empty()
                && (m.channel == "stable"
                    || prerelease.split('.').next() != Some(m.channel.as_str())))
        {
            return Err(UpdateError::Invalid("release channel"));
        }
        if !matches!(m.revision.len(), 40 | 64)
            || !m.revision.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(UpdateError::Invalid("revision"));
        }
        chrono::DateTime::parse_from_rfc3339(&m.date).map_err(|_| UpdateError::Invalid("date"))?;
        if m.artifacts.is_empty() {
            return Err(UpdateError::Invalid("artifacts"));
        }
        let mut targets = HashSet::new();
        let mut urls = HashSet::new();
        let mut payloads = HashSet::new();
        for a in &m.artifacts {
            if !a.valid_target()
                || !a.minimum_os.pre.is_empty()
                || !a.minimum_os.build.is_empty()
                || !targets.insert((&a.target, a.installation))
            {
                return Err(UpdateError::Invalid("target, installation or minimum OS"));
            }
            let url = https_url(&a.url)?;
            if !url.path().ends_with(a.format.extension())
                || !urls.insert(url)
                || !payloads.insert(a.sha256.to_string())
                || !(1..=ARTIFACT_LIMIT).contains(&a.bytes)
                || !(1..=8192).contains(&a.signature.len())
            {
                return Err(UpdateError::Invalid(
                    "artifact URL, digest, length or signature",
                ));
            }
            check_signature(&a.signature)?;
        }
        Ok(Self(m))
    }
    pub fn version(&self) -> &Version {
        &self.0.version
    }
    pub fn channel(&self) -> &str {
        &self.0.channel
    }
    pub fn revision(&self) -> &str {
        &self.0.revision
    }
    pub fn notes(&self) -> &str {
        &self.0.notes
    }
    pub fn date(&self) -> &str {
        &self.0.date
    }
    pub fn artifacts(&self) -> &[Artifact] {
        &self.0.artifacts
    }
}

pub fn https_url(text: &str) -> Result<Url, UpdateError> {
    let url = Url::parse(text).map_err(|_| UpdateError::Invalid("URL"))?;
    if !text.starts_with("https://")
        || text.starts_with("https:///")
        || text.bytes().any(|b| b <= b' ' || b == b'\\' || b == 127)
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(UpdateError::Invalid("HTTPS artifact URL"));
    }
    Ok(url)
}
