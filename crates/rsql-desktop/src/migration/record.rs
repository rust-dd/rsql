use super::{ImportError, Result};
use crate::persistence;
use rsql_core::migration::{
    DATABASE_FILENAME, DirectoryId, ProfileLease, Sha256Digest, open_regular,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use uuid::Uuid;

pub(super) use rsql_core::migration::IMPORT_JOURNAL_FILENAME as JOURNAL;
pub(super) const CONFIRMED: &str = "confirmed.json";
pub(super) const PREPARED: &str = "prepared.json";
pub(super) const FORMAT: u32 = 2;
pub(super) const GENERATION: usize = 2;
pub(super) const RECEIPT: usize = 3;
pub(super) const LIVE: [&str; 4] = [
    DATABASE_FILENAME,
    persistence::FILENAME,
    "rsql.import-generation",
    "rsql.import-receipt.json",
];
pub(super) const BACKUPS: [&str; LIVE.len()] = [
    "previous.db",
    "previous.workspace.json",
    "previous.generation",
    "previous.receipt.json",
];
pub(super) type Hashes = [Option<Sha256Digest>; LIVE.len()];
const RECORD_LIMIT: u64 = 16 * 1024;

pub(super) use rsql_core::migration::handoff::ExportIdentity as Source;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub version: u32,
    pub operation: Uuid,
    pub replace: bool,
    pub durable_tabs: usize,
    pub discarded_tabs: usize,
    pub directory: DirectoryId,
    pub destination: Sha256Digest,
    pub stage: DirectoryId,
    pub source: Source,
    pub database: Sha256Digest,
    pub workspace: Sha256Digest,
    pub old: Hashes,
    pub pre_rekey: Option<Sha256Digest>,
}

#[derive(Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Completion {
    version: u32,
    pub operation: Uuid,
    pub(super) prepared_sha256: Sha256Digest,
}

impl Completion {
    pub fn verify(&self, lease: &ProfileLease) -> Result<Record> {
        let stage = stage_path(lease.directory(), self.operation);
        DirectoryId::read(&stage)?;
        let record = read_record::<Record>(&stage.join(PREPARED))?;
        if record.validate(lease)? != stage
            || record.completion()? != *self
            || file_hash(&stage.join(CONFIRMED))? != record.installed()?[RECEIPT]
        {
            return Err(ImportError::Integrity("activation provenance"));
        }
        Ok(record)
    }
}

impl Record {
    pub fn location(&self, lease: &ProfileLease) -> Result<PathBuf> {
        if self.version != FORMAT
            || self.operation.is_nil()
            || self.directory != lease.verify_directory()?
            || self.destination != bytes_hash(lease.directory().as_os_str().as_encoded_bytes())?
        {
            return Err(ImportError::Integrity("profile identity or record version"));
        }
        let stage = stage_path(lease.directory(), self.operation);
        if DirectoryId::read(&stage)? != self.stage {
            return Err(ImportError::Integrity("import directory identity"));
        }
        Ok(stage)
    }

    pub fn validate(&self, lease: &ProfileLease) -> Result<PathBuf> {
        let stage = self.location(lease)?;
        let path = stage.join(PREPARED);
        if read_record::<Self>(&path)? != *self
            || file_hash(&path)? != Some(self.completion()?.prepared_sha256)
        {
            return Err(ImportError::Integrity("prepared import record"));
        }
        Ok(stage)
    }

    pub fn completion(&self) -> Result<Completion> {
        Ok(Completion {
            version: FORMAT,
            operation: self.operation,
            prepared_sha256: bytes_hash(&encode(self)?)?,
        })
    }

    pub fn installed(&self) -> Result<Hashes> {
        Ok([
            Some(self.database),
            Some(self.workspace),
            Some(bytes_hash(self.operation.as_bytes())?),
            Some(bytes_hash(&encode(&self.completion()?)?)?),
        ])
    }
}

pub(super) fn stage_path(profile: &Path, operation: Uuid) -> PathBuf {
    profile.join(format!(".rsql-import-{operation}"))
}

pub(super) fn bytes_hash(bytes: &[u8]) -> Result<Sha256Digest> {
    Ok(Sha256Digest::read(bytes)?.1)
}

pub(super) fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| ImportError::Integrity("record encoding"))
}

pub(super) fn read_record<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    open_regular(path)?
        .take(RECORD_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Err(ImportError::Integrity("oversized migration record"));
    }
    serde_json::from_slice(&bytes).map_err(|_| ImportError::Integrity("invalid migration record"))
}

pub(super) fn present(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub(super) fn file_hash(path: &Path) -> Result<Option<Sha256Digest>> {
    if !present(path)? {
        return Ok(None);
    }
    Ok(Some(Sha256Digest::read(open_regular(path)?)?.1))
}

pub(super) fn live_hashes(profile: &Path) -> Result<Hashes> {
    let mut hashes = [None; LIVE.len()];
    for (slot, name) in hashes.iter_mut().zip(LIVE) {
        *slot = file_hash(&profile.join(name))?;
    }
    Ok(hashes)
}

pub(super) fn standalone(profile: &Path) -> Result<()> {
    for name in ["rsql.db-wal", "rsql.db-shm", "rsql.db-journal"] {
        if present(&profile.join(name))? {
            return Err(ImportError::Integrity("outstanding SQLite sidecar"));
        }
    }
    Ok(())
}

pub(super) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(super) fn publish_new(file: NamedTempFile, destination: &Path) -> Result<()> {
    #[cfg(windows)]
    file.persist_noclobber(destination).map_err(|e| e.error)?;
    #[cfg(not(windows))]
    {
        // The hard-link fallback of persist_noclobber can leave two names after a crash.
        let mut source = file.into_temp_path();
        rsql_core::migration::rename_exclusive(&source, destination)?;
        source.disable_cleanup(true);
    }
    Ok(())
}

pub(super) fn temp_bytes(dir: &Path, bytes: &[u8]) -> Result<NamedTempFile> {
    let mut file = NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    Ok(file)
}

pub(super) fn temp_copy(
    dir: &Path,
    source: &Path,
    expected: Sha256Digest,
) -> Result<NamedTempFile> {
    let mut file = NamedTempFile::new_in(dir)?;
    io::copy(&mut open_regular(source)?, &mut file)?;
    file.as_file().sync_all()?;
    if file_hash(file.path())? != Some(expected) {
        return Err(ImportError::Integrity("copied file digest"));
    }
    Ok(file)
}
