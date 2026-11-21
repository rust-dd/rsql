use super::{
    BODY_LIMIT, DISK_LIMIT, Result, TileKey,
    policy::{METADATA_LIMIT, Metadata},
};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};
use tokio::sync::watch;

const ENTRY_LIMIT: usize = 4096;
const MAX_FILE: u64 = (BODY_LIMIT + METADATA_LIMIT + 8) as u64;
const TEMP_PREFIX: &str = ".pending-";

pub(super) struct Record {
    pub metadata: Metadata,
    pub bytes: Vec<u8>,
}

pub(super) struct Disk {
    directory: PathBuf,
    index: Mutex<Option<Index>>,
    _profile: Option<Arc<rsql_core::migration::ProfileLease>>,
}

struct Index {
    _lock: File,
    entries: HashMap<String, (u64, SystemTime)>,
    bytes: u64,
}

impl Disk {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            _profile: None,
            index: Mutex::default(),
        }
    }

    pub fn for_profile(
        profile: Arc<rsql_core::migration::ProfileLease>,
        provider: super::TileProvider,
    ) -> Self {
        let path = profile
            .directory()
            .join("tile-cache")
            .join(provider.directory());
        Self {
            _profile: Some(profile),
            ..Self::new(path)
        }
    }

    async fn access<T: Send + 'static>(
        self: &Arc<Self>,
        stop: watch::Receiver<bool>,
        f: impl FnOnce(&mut Index, &Path) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let disk = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut cache = disk.index.lock().unwrap();
            if *stop.borrow() {
                return Err("Tile request cancelled".into());
            }
            if cache.is_none() {
                *cache = Some(Index::open(&disk.directory, &stop).map_err(|e| e.to_string())?);
            }
            if *stop.borrow() {
                return Err("Tile request cancelled".into());
            }
            f(cache.as_mut().unwrap(), &disk.directory)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn read(
        self: &Arc<Self>,
        key: TileKey,
        stop: watch::Receiver<bool>,
    ) -> Result<Option<Record>> {
        self.access(stop, move |index, dir| {
            let name = filename(key);
            let Some((size, stamp)) = index.entries.get_mut(&name) else {
                return Ok(None);
            };
            *stamp = SystemTime::now();
            let path = dir.join(&name);
            match path.symlink_metadata() {
                Ok(meta) if meta.is_file() => {}
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
                _ => {
                    index.remove(dir, &name).map_err(|e| e.to_string())?;
                    return Ok(None);
                }
            }
            let file = File::open(path).map_err(|e| e.to_string())?;
            let record = read_record(file, *size, key);
            match record {
                Ok(record) => Ok(Some(record)),
                Err(e) if matches!(e.kind(), ErrorKind::InvalidData | ErrorKind::UnexpectedEof) => {
                    index.remove(dir, &name).map_err(|e| e.to_string())?;
                    Ok(None)
                }
                Err(e) => Err(e.to_string()),
            }
        })
        .await
    }

    pub async fn discard(
        self: &Arc<Self>,
        key: TileKey,
        stop: watch::Receiver<bool>,
    ) -> Result<()> {
        self.access(stop, move |index, dir| {
            index.remove(dir, &filename(key)).map_err(|e| e.to_string())
        })
        .await
    }

    pub async fn store(
        self: &Arc<Self>,
        key: TileKey,
        record: Record,
        stop: watch::Receiver<bool>,
    ) -> Result<()> {
        self.access(stop.clone(), move |index, dir| {
            let name = filename(key);
            if !record.metadata.store {
                return index.remove(dir, &name).map_err(|e| e.to_string());
            }
            let metadata = serde_json::to_vec(&record.metadata).map_err(|e| e.to_string())?;
            if metadata.len() > METADATA_LIMIT || record.bytes.len() > BODY_LIMIT {
                return Err("Tile record exceeds cache limits".into());
            }
            let length = (8 + metadata.len() + record.bytes.len()) as u64;
            index
                .make_room(dir, &name, length, DISK_LIMIT, ENTRY_LIMIT)
                .map_err(|e| e.to_string())?;
            let mut pending = tempfile::Builder::new()
                .prefix(TEMP_PREFIX)
                .tempfile_in(dir)
                .map_err(|e| e.to_string())?;
            let mut checksum = crc32fast::Hasher::new();
            checksum.update(&metadata);
            checksum.update(&record.bytes);
            for bytes in [
                &(metadata.len() as u32).to_le_bytes()[..],
                &checksum.finalize().to_le_bytes(),
                &metadata,
                &record.bytes,
            ] {
                pending.write_all(bytes).map_err(|e| e.to_string())?;
            }
            pending.as_file().sync_all().map_err(|e| e.to_string())?;
            if *stop.borrow() {
                return Err("Tile request cancelled".into());
            }
            pending
                .persist(dir.join(&name))
                .map_err(|e| e.to_string())?;
            index.insert(name, length, SystemTime::now());
            Ok(())
        })
        .await
    }
}

fn filename(k: TileKey) -> String {
    format!("{}-{}-{}.tile", k.z, k.x, k.y)
}

fn read_record(mut file: File, size: u64, key: TileKey) -> io::Result<Record> {
    let invalid = |msg| io::Error::new(ErrorKind::InvalidData, msg);
    if !(9..=MAX_FILE).contains(&size) || file.metadata()?.len() != size {
        return Err(invalid("Invalid tile length"));
    }
    let mut header = [0; 8];
    file.read_exact(&mut header)?;
    let len = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    if len > METADATA_LIMIT || 8 + len >= size as usize || size as usize - 8 - len > BODY_LIMIT {
        return Err(invalid("Invalid tile record"));
    }
    let mut meta = vec![0; len];
    let mut bytes = vec![0; size as usize - 8 - len];
    file.read_exact(&mut meta)
        .and_then(|()| file.read_exact(&mut bytes))?;
    let mut checksum = crc32fast::Hasher::new();
    checksum.update(&meta);
    checksum.update(&bytes);
    if checksum.finalize().to_le_bytes() != header[4..] {
        return Err(invalid("Tile checksum mismatch"));
    }
    let metadata = serde_json::from_slice::<Metadata>(&meta)?;
    if metadata.key != key.coordinates() || !metadata.store {
        return Err(invalid("Invalid cached identity"));
    }
    Ok(Record { metadata, bytes })
}

impl Index {
    fn open(dir: &Path, stop: &watch::Receiver<bool>) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        if !dir.symlink_metadata()?.is_dir() {
            return Err(std::io::Error::other("Invalid tile cache directory"));
        }
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(".lock"))?;
        lock.try_lock().map_err(std::io::Error::other)?;
        let mut index = Self {
            _lock: lock,
            entries: HashMap::new(),
            bytes: 0,
        };
        for entry in fs::read_dir(dir)? {
            if *stop.borrow() {
                return Err(std::io::Error::other("Tile request cancelled"));
            }
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let kind = entry.file_type()?;
            if name.starts_with(TEMP_PREFIX) && !kind.is_dir() {
                fs::remove_file(entry.path())?;
                continue;
            }
            if !name.ends_with(".tile") {
                continue;
            }
            if !kind.is_file() {
                return Err(std::io::Error::other("Unexpected tile cache entry type"));
            }
            let meta = entry.metadata()?;
            if meta.len() > MAX_FILE || name.len() > 32 {
                fs::remove_file(entry.path())?;
                continue;
            }
            index.make_room(dir, name, meta.len(), DISK_LIMIT, ENTRY_LIMIT)?;
            index.insert(
                name.to_owned(),
                meta.len(),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            );
        }
        Ok(index)
    }
    fn insert(&mut self, name: String, len: u64, at: SystemTime) {
        if let Some((old, _)) = self.entries.insert(name, (len, at)) {
            self.bytes -= old;
        }
        self.bytes += len;
    }
    fn remove(&mut self, dir: &Path, name: &str) -> std::io::Result<()> {
        if self.entries.contains_key(name) {
            match fs::remove_file(dir.join(name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            self.bytes -= self.entries.remove(name).unwrap().0;
        }
        Ok(())
    }
    fn make_room(
        &mut self,
        dir: &Path,
        name: &str,
        len: u64,
        limit: u64,
        count: usize,
    ) -> std::io::Result<()> {
        while self.bytes + len > limit
            || (self.entries.len() >= count && !self.entries.contains_key(name))
        {
            let oldest = self
                .entries
                .iter()
                .filter(|(k, _)| k.as_str() != name)
                .min_by_key(|(_, (_, at))| *at)
                .map(|(key, _)| key.clone())
                .ok_or_else(|| {
                    std::io::Error::other("Insufficient tile cache space for atomic replacement")
                })?;
            self.remove(dir, &oldest)?;
        }
        Ok(())
    }
}
