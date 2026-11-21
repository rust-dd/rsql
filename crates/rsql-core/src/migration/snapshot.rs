use super::{
    DATABASE_FILENAME, DatabaseOperation, ProfileLease, Sha256Digest,
    files::FileIdentity,
    operation::{Result, SnapshotError},
    stage::Stage,
};
use libsql::{Connection, OpenFlags};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};
use tokio::sync::Mutex;

pub(super) async fn connect(path: &Path, flags: OpenFlags) -> Result<Connection> {
    libsql::Builder::new_local(path)
        .flags(flags)
        .build()
        .await
        .and_then(|db| db.connect())
        .map_err(|_| SnapshotError::Database("database open"))
}

pub(super) async fn integrity(conn: &Connection) -> Result<()> {
    let mut rows = conn
        .query("PRAGMA main.integrity_check", ())
        .await
        .map_err(|_| SnapshotError::Integrity)?;
    let first = rows
        .next()
        .await
        .map_err(|_| SnapshotError::Integrity)?
        .ok_or(SnapshotError::Integrity)?;
    if first
        .get::<String>(0)
        .map_err(|_| SnapshotError::Integrity)?
        != "ok"
        || rows
            .next()
            .await
            .map_err(|_| SnapshotError::Integrity)?
            .is_some()
    {
        return Err(SnapshotError::Integrity);
    }
    Ok(())
}

async fn version(conn: &Connection) -> Result<i64> {
    let error = || SnapshotError::Database("source generation check");
    conn.query("PRAGMA main.data_version", ())
        .await
        .map_err(|_| error())?
        .next()
        .await
        .map_err(|_| error())?
        .ok_or_else(error)?
        .get::<i64>(0)
        .map_err(|_| error())
}

/// Only this observer's read-only connection can validate a generation from its lifetime.
pub struct SourceObserver {
    connection: Mutex<Connection>,
    file: FileIdentity,
    path: PathBuf,
    lease: Arc<ProfileLease>,
}

/// Process-local observation of committed source data. The bridge must separately guard
/// write admission and UI revisions through its final handoff.
#[derive(Clone)]
pub struct SourceGeneration {
    owner: Weak<SourceObserver>,
    version: i64,
}

impl SourceObserver {
    pub fn profile_lease(&self) -> &ProfileLease {
        &self.lease
    }

    /// Opens the source database read-only and leaves committed data and journal mode intact.
    /// In WAL mode SQLite may need directory write access to create `rsql.db-wal` and
    /// `rsql.db-shm`. These shared files belong to SQLite and may outlive this observer.
    pub async fn open(lease: Arc<ProfileLease>) -> Result<Arc<Self>> {
        DatabaseOperation::start(move |_| async move {
            let path = lease.directory().join(DATABASE_FILENAME);
            let file = FileIdentity::capture(&path)?;
            let connection = connect(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).await?;
            version(&connection).await?;
            if !file.matches(&path)? {
                return Err(SnapshotError::SourceChanged);
            }
            Ok(Arc::new(Self {
                lease,
                connection: Mutex::new(connection),
                file,
                path,
            }))
        })
        .join()
        .await
    }

    pub async fn revalidate(self: &Arc<Self>, token: &SourceGeneration) -> Result<bool> {
        if !Weak::ptr_eq(&token.owner, &Arc::downgrade(self)) {
            return Ok(false);
        }
        let source = self.clone();
        let expected = token.version;
        DatabaseOperation::start(move |_| async move {
            let conn = source.connection.lock().await;
            Ok(source.file.matches(&source.path)? && version(&conn).await? == expected)
        })
        .join()
        .await
    }
}

pub struct DatabaseSnapshot {
    stage: Stage,
    bytes: u64,
    digest: Sha256Digest,
    generation: SourceGeneration,
}

impl DatabaseSnapshot {
    /// Finish and publish both validated files under a new sibling name. Run on a blocking worker.
    pub fn publish(mut self, manifest: &super::MigrationManifest) -> Result<PathBuf> {
        if (manifest.database_byte_len(), manifest.database_sha256()) != (self.bytes, self.digest) {
            return Err(SnapshotError::ContentMismatch);
        }
        let bytes = manifest
            .encode()
            .map_err(|_| SnapshotError::ManifestTooLarge)?;
        self.stage.write_manifest(&bytes)?;
        self.stage.publish(manifest)
    }

    /// Borrow the fixed database file while this owner is alive.
    pub fn path(&self) -> &Path {
        self.stage.path()
    }

    /// Transfer the completed private directory and its cleanup to the caller.
    pub fn into_directory(self) -> PathBuf {
        self.stage.into_directory()
    }

    pub fn byte_len(&self) -> u64 {
        self.bytes
    }

    pub fn sha256(&self) -> Sha256Digest {
        self.digest
    }

    pub fn generation(&self) -> &SourceGeneration {
        &self.generation
    }
}

/// Export committed SQLite data into a new private directory in a caller-owned parent.
/// SQLite manages the source WAL and shared memory throughout export and observer retirement.
/// Call from a Tokio runtime. Dropping the snapshot removes its files unless ownership was transferred.
pub fn export_database(
    source: &Arc<SourceObserver>,
    staging: impl AsRef<Path>,
) -> DatabaseOperation<DatabaseSnapshot> {
    let source = Arc::clone(source);
    let staging = staging.as_ref().to_owned();
    DatabaseOperation::start(move |cancel| async move {
        let conn = source.connection.lock().await;
        cancel.check()?;
        if !source.file.matches(&source.path)? {
            return Err(SnapshotError::SourceChanged);
        }
        let before = version(&conn).await?;
        integrity(&conn).await?;
        let stage = Stage::create(&staging, source.lease.directory())?;
        cancel.check()?;
        let path = stage.path().to_str().ok_or(SnapshotError::InvalidStage)?;
        conn.execute("VACUUM INTO ?1", [path])
            .await
            .map_err(|_| SnapshotError::Database("snapshot export"))?;
        if version(&conn).await? != before || !source.file.matches(&source.path)? {
            return Err(SnapshotError::SourceChanged);
        }
        drop(conn);
        cancel.check()?;
        let output = connect(stage.path(), OpenFlags::SQLITE_OPEN_READ_ONLY).await?;
        integrity(&output).await?;
        drop(output);
        let (bytes, digest) = stage.finish(&cancel)?;
        Ok(DatabaseSnapshot {
            stage,
            bytes,
            digest,
            generation: SourceGeneration {
                owner: Arc::downgrade(&source),
                version: before,
            },
        })
    })
}
