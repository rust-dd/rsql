use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, Read, Write},
    num::NonZeroU64,
};
use uuid::Uuid;

pub const MANIFEST_FILENAME: &str = "manifest.json";
pub const DATABASE_FILENAME: &str = "rsql.db";
pub const MANIFEST_FORMAT: u64 = 1;
pub const MANIFEST_MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotId(Uuid);

impl fmt::Display for SnapshotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'de> Deserialize<'de> for SnapshotId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Uuid::try_parse(&raw)
            .ok()
            .filter(|id| !id.is_nil())
            .map(Self)
            .ok_or_else(|| de::Error::custom("expected a non-nil snapshot UUID"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sha256Digest([u8; 32]);

impl Serialize for Sha256Digest {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl Serialize for SnapshotId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl Sha256Digest {
    pub(super) fn from_hash(hash: Sha256) -> Self {
        Self(hash.finalize().into())
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let mut bytes = [0; 32];
        hex::decode_to_slice(raw, &mut bytes)
            .map_err(|_| de::Error::custom("expected a 64-character SHA-256 hex digest"))?;
        Ok(Self(bytes))
    }
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LegacyUi {
    pub tabs: Option<String>,
    pub theme: Option<String>,
}

impl LegacyUi {
    /// Hashes the raw `rsql-tabs` and `rsql-theme` values in key order.
    /// Each present value carries its UTF-8 byte length as a big-endian `u64`.
    pub fn sha256(&self) -> Sha256Digest {
        let mut hash = Sha256::new();
        hash.update(b"rsql-legacy-ui\0");
        for value in [&self.tabs, &self.theme] {
            match value {
                None => hash.update([0]),
                Some(text) => {
                    hash.update([1]);
                    hash.update((text.len() as u64).to_be_bytes());
                    hash.update(text.as_bytes());
                }
            }
        }
        Sha256Digest::from_hash(hash)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("Invalid migration manifest")]
    InvalidManifest,
    #[error("Unknown migration manifest format {0}")]
    UnsupportedFormat(u64),
    #[error("Migration manifest exceeds 64 MiB")]
    ManifestTooLarge,
    #[error("Migration UI digest mismatch")]
    UiDigestMismatch,
    #[error("Cannot read migration manifest ({0})")]
    Read(io::ErrorKind),
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ManifestFields {
    format_version: u64,
    snapshot_id: SnapshotId,
    source_app_version: String,
    created_at_unix_ms: u64,
    database_byte_len: NonZeroU64,
    database_sha256: Sha256Digest,
    #[serde(deserialize_with = "read_ui")]
    ui: LegacyUi,
    ui_sha256: Sha256Digest,
}

fn read_ui<'de, D: Deserializer<'de>>(deserializer: D) -> Result<LegacyUi, D::Error> {
    let raw = <&RawValue>::deserialize(deserializer)?;
    if !raw.get().starts_with('{') {
        return Err(de::Error::custom("expected a legacy UI object"));
    }
    serde_json::from_str(raw.get()).map_err(de::Error::custom)
}

/// Shape-validated manifest with a checked legacy UI digest.
/// The database length and digest are declarations for the subsequent file checks.
#[derive(PartialEq, Eq)]
pub struct MigrationManifest(ManifestFields);

impl MigrationManifest {
    pub fn for_export(
        snapshot: &super::DatabaseSnapshot,
        source_app_version: &str,
        created_at_unix_ms: u64,
        ui: LegacyUi,
    ) -> Result<Self, ManifestError> {
        if source_app_version.trim().is_empty() {
            return Err(ManifestError::InvalidManifest);
        }
        Ok(Self(ManifestFields {
            format_version: MANIFEST_FORMAT,
            snapshot_id: SnapshotId(Uuid::new_v4()),
            source_app_version: source_app_version.into(),
            created_at_unix_ms,
            database_byte_len: NonZeroU64::new(snapshot.byte_len())
                .ok_or(ManifestError::InvalidManifest)?,
            database_sha256: snapshot.sha256(),
            ui_sha256: ui.sha256(),
            ui,
        }))
    }

    pub fn encode(&self) -> Result<Vec<u8>, ManifestError> {
        struct Limited(Vec<u8>);
        impl Write for Limited {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if bytes.len() > MANIFEST_MAX_BYTES - self.0.len() {
                    return Err(io::ErrorKind::FileTooLarge.into());
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut output = Limited(Vec::new());
        serde_json::to_writer(&mut output, &self.0).map_err(|_| ManifestError::ManifestTooLarge)?;
        Ok(output.0)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MANIFEST_MAX_BYTES {
            return Err(ManifestError::ManifestTooLarge);
        }
        if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
            return Err(ManifestError::InvalidManifest);
        }

        #[derive(Deserialize)]
        struct Format {
            format_version: u64,
        }
        let format =
            serde_json::from_slice::<Format>(bytes).map_err(|_| ManifestError::InvalidManifest)?;
        if format.format_version != MANIFEST_FORMAT {
            return Err(ManifestError::UnsupportedFormat(format.format_version));
        }
        let fields = serde_json::from_slice::<ManifestFields>(bytes)
            .map_err(|_| ManifestError::InvalidManifest)?;
        if fields.source_app_version.trim().is_empty() {
            return Err(ManifestError::InvalidManifest);
        }
        if fields.ui.sha256() != fields.ui_sha256 {
            return Err(ManifestError::UiDigestMismatch);
        }
        Ok(Self(fields))
    }

    pub fn read(mut reader: impl Read) -> Result<Self, ManifestError> {
        let mut bytes = Vec::new();
        reader
            .by_ref()
            .take(MANIFEST_MAX_BYTES as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| ManifestError::Read(error.kind()))?;
        if bytes.len() == MANIFEST_MAX_BYTES {
            loop {
                match reader.read(&mut [0]) {
                    Ok(0) => break,
                    Ok(_) => return Err(ManifestError::ManifestTooLarge),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(ManifestError::Read(error.kind())),
                }
            }
        }
        Self::decode(&bytes)
    }

    pub fn format_version(&self) -> u64 {
        self.0.format_version
    }

    pub fn snapshot_id(&self) -> SnapshotId {
        self.0.snapshot_id
    }

    pub fn source_app_version(&self) -> &str {
        &self.0.source_app_version
    }

    pub fn created_at_unix_ms(&self) -> u64 {
        self.0.created_at_unix_ms
    }

    pub fn database_byte_len(&self) -> u64 {
        self.0.database_byte_len.get()
    }

    pub fn database_sha256(&self) -> Sha256Digest {
        self.0.database_sha256
    }

    pub fn ui_sha256(&self) -> Sha256Digest {
        self.0.ui_sha256
    }

    pub fn ui(&self) -> &LegacyUi {
        &self.0.ui
    }
}
