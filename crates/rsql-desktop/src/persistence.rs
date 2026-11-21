use crate::document::{Appearance, Documents, Tab};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tempfile::NamedTempFile;

pub const FILENAME: &str = "workspace.json";

#[derive(Serialize)]
struct Snapshot<'a> {
    version: u32,
    theme: Appearance,
    tabs: Vec<&'a Tab>,
    selected_id: Option<&'a str>,
    selected_index: Option<usize>,
}

pub fn encode(docs: &Documents) -> io::Result<Vec<u8>> {
    let tabs: Vec<_> = docs.tabs.iter().filter(|t| t.kind.durable()).collect();
    let selected = docs
        .selected
        .and_then(|index| {
            docs.tabs
                .iter()
                .take(index.saturating_add(1))
                .rev()
                .find(|t| t.kind.durable())
        })
        .or_else(|| tabs.first().copied());
    let selected_id = selected.map(|t| t.id.as_str());
    let selected_index = tabs.iter().position(|t| Some(t.id.as_str()) == selected_id);
    Ok(serde_json::to_vec(&Snapshot {
        version: 1,
        theme: docs.theme,
        tabs,
        selected_id,
        selected_index,
    })?)
}

pub(crate) fn encode_import(documents: &Documents) -> io::Result<Vec<u8>> {
    let bytes = encode(documents)?;
    if encode(&decode(&bytes)?)? != bytes {
        return Err(io::Error::other(
            "Imported workspace failed its durable round trip",
        ));
    }
    Ok(bytes)
}

pub(crate) fn validate_existing(bytes: &[u8]) -> io::Result<()> {
    decode(bytes).map(drop)
}

fn read_current(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn decode(bytes: &[u8]) -> io::Result<Documents> {
    let record: Value = serde_json::from_slice(bytes)?;
    if record["version"].as_u64() != Some(1) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported workspace format",
        ));
    }
    let entries = record["tabs"]
        .as_array()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "expected a tab list"))?;
    let mut seen = HashSet::new();
    let tabs: Vec<Tab> = entries
        .iter()
        .filter_map(|v| serde_json::from_value::<Tab>(v.clone()).ok())
        .filter(|t| t.kind.durable() && !t.id.is_empty() && seen.insert(t.id.clone()))
        .collect();
    let index = record["selected_index"].as_u64().unwrap_or(0);
    let selected = tabs
        .iter()
        .position(|t| Some(t.id.as_str()) == record["selected_id"].as_str())
        .or_else(|| (!tabs.is_empty()).then(|| index.min((tabs.len() - 1) as u64) as usize));
    let theme = serde_json::from_value(record["theme"].clone()).unwrap_or_default();
    Ok(Documents {
        tabs,
        selected,
        theme,
    })
}

/// Publish a recoverable workspace without replacing any existing file.
pub fn save_workspace_copy(directory: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    save_workspace_copy_before_publish(directory, bytes, || {})
}

#[doc(hidden)]
pub fn save_workspace_copy_before_publish(
    directory: &Path,
    bytes: &[u8],
    before_publish: impl FnOnce(),
) -> io::Result<PathBuf> {
    let path = directory.join(FILENAME);
    let mut stage = NamedTempFile::new_in(directory)?;
    stage.write_all(bytes)?;
    stage.as_file().sync_all()?;
    if fs::read(stage.path())? != bytes || encode(&decode(bytes)?)? != bytes {
        return Err(io::Error::other("Workspace copy failed verification"));
    }
    before_publish();
    stage.persist_noclobber(&path)?;
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    Ok(path)
}

pub struct SnapshotFile {
    path: PathBuf,
    expected: Mutex<Option<Vec<u8>>>,
}

impl SnapshotFile {
    pub fn open(path: &Path) -> io::Result<(Self, Documents)> {
        let expected = read_current(path)?;
        let documents = match expected.as_deref() {
            Some(bytes) => decode(bytes)?,
            None => Documents::default(),
        };
        Ok((
            Self {
                path: path.into(),
                expected: Mutex::new(expected),
            },
            documents,
        ))
    }

    pub fn save(&self, bytes: Vec<u8>) -> io::Result<()> {
        let path = &self.path;
        let mut stage = NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
        stage.write_all(&bytes)?;
        stage.as_file().sync_all()?;

        // The snapshot inode changes on rename. All preview writers lock this stable sibling.
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_added_extension("lock"))?;
        lock.lock()?;
        let mut expected = self
            .expected
            .lock()
            .map_err(|_| io::Error::other("workspace writer poisoned"))?;
        if read_current(path)? != *expected {
            return Err(io::Error::other(
                "workspace changed on disk since it was loaded or last saved",
            ));
        }
        stage.persist(path)?;
        *expected = Some(bytes);
        Ok(())
    }
}
