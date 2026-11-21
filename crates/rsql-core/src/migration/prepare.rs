use super::{
    DATABASE_FILENAME, DatabaseOperation, Sha256Digest,
    files::{check_file, file_options, hash, stream},
    operation::{Cancellation, Result, SnapshotError},
    snapshot::{connect, integrity},
    stage::Stage,
};
use crate::storage;
use libsql::{Connection, OpenFlags};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, Write},
    path::Path,
};

/// An integrity-checked, closed standalone file and the project rows read back from it.
/// This value covers the database only. Its private files exist for this owner's lifetime.
pub struct PreparedDatabase {
    stage: Stage,
    bytes: u64,
    digest: Sha256Digest,
    projects: BTreeMap<String, Vec<String>>,
}

impl PreparedDatabase {
    pub fn path(&self) -> &Path {
        self.stage.path()
    }

    /// Transfer the completed private directory and its cleanup to the caller.
    pub fn into_directory(self) -> std::path::PathBuf {
        self.stage.into_directory()
    }

    /// Store one of the native import's fixed artifacts under the same identity-checked owner.
    pub fn write_import_file(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        self.stage.write_import_file(name, bytes)
    }

    pub fn verify_stage_identity(&self) -> Result<()> {
        self.stage.verify_import_entries()
    }

    pub fn byte_len(&self) -> u64 {
        self.bytes
    }

    pub fn sha256(&self) -> Sha256Digest {
        self.digest
    }

    /// Exactly the rows consumed by the native Connections::from_rows converter.
    pub fn project_rows(&self) -> &BTreeMap<String, Vec<String>> {
        &self.projects
    }
}

/// Verify and copy the bundle's standalone fixed database before opening a private copy
/// for migration. Call from a Tokio runtime and retain the source bundle until the worker joins.
pub fn prepare_database(
    bundle: impl AsRef<Path>,
    expected_len: u64,
    expected_digest: Sha256Digest,
    staging: impl AsRef<Path>,
) -> DatabaseOperation<PreparedDatabase> {
    let bundle = bundle.as_ref().to_owned();
    let staging = staging.as_ref().to_owned();
    DatabaseOperation::start(move |cancel| async move {
        prepare_with(
            &bundle,
            expected_len,
            expected_digest,
            &staging,
            &cancel,
            |_| Ok(()),
        )
        .await
    })
}

/// Run the same strict preparation inside an already-owned blocking worker with a Tokio handle.
pub fn prepare_database_blocking(
    bundle: &Path,
    expected_len: u64,
    expected_digest: Sha256Digest,
    staging: &Path,
) -> Result<PreparedDatabase> {
    tokio::runtime::Handle::current().block_on(prepare_with(
        bundle,
        expected_len,
        expected_digest,
        staging,
        &Cancellation::default(),
        |_| Ok(()),
    ))
}

async fn prepare_with(
    bundle: &Path,
    expected_len: u64,
    expected_digest: Sha256Digest,
    staging: &Path,
    cancel: &Cancellation,
    boundary: impl Fn(&'static str) -> Result<()>,
) -> Result<PreparedDatabase> {
    let directory = bundle.canonicalize()?;
    let path = directory.join(DATABASE_FILENAME);
    let metadata = check_file(&path)?;
    if expected_len == 0 || metadata.len() != expected_len {
        return Err(SnapshotError::ContentMismatch);
    }
    let mut source = file_options().open(&path)?;
    let expected = (expected_len, expected_digest);
    if hash(
        &mut Read::by_ref(&mut source).take(expected_len.saturating_add(1)),
        cancel,
    )? != expected
    {
        return Err(SnapshotError::ContentMismatch);
    }
    source.rewind()?;
    let mut stage = Stage::create(staging, &directory)?;
    boundary("stage created")?;
    copy_verified(&mut source, &stage, expected, cancel)?;
    drop(source);
    boundary("database copied")?;
    let conn = connect(stage.path(), OpenFlags::SQLITE_OPEN_READ_WRITE).await?;
    integrity(&conn).await?;
    let backup = stage.reserve_backup()?;
    let migrated =
        storage::initialize_staged(&conn, backup.to_str().ok_or(SnapshotError::InvalidStage)?)
            .await?;
    if migrated == 0 {
        stage.drop_unused_backup()?;
    }
    boundary("strict migrations finished")?;
    cancel.check()?;
    finish_journal(&conn).await?;
    drop(conn);
    let reader = connect(stage.path(), OpenFlags::SQLITE_OPEN_READ_ONLY).await?;
    integrity(&reader).await?;
    storage::verify_staged_schema(&reader).await?;
    let projects = storage::project::select_projects(&reader)
        .await
        .map_err(|_| SnapshotError::Database("reading migrated project rows"))?;
    drop(reader);
    let (bytes, digest) = stage.finish(cancel)?;
    Ok(PreparedDatabase {
        stage,
        bytes,
        digest,
        projects,
    })
}

fn copy_verified(
    source: &mut File,
    stage: &Stage,
    expected: (u64, Sha256Digest),
    cancel: &Cancellation,
) -> Result<()> {
    let mut copy = file_options().write(true).open(stage.path())?;
    let mut bounded = Read::by_ref(source).take(expected.0.saturating_add(1));
    stream(&mut bounded, cancel, |chunk| copy.write_all(chunk))?;
    copy.flush()?;
    copy.rewind()?;
    if hash(&mut copy, cancel)? != expected {
        return Err(SnapshotError::ContentMismatch);
    }
    Ok(())
}

async fn finish_journal(conn: &Connection) -> Result<()> {
    let mut rows = conn
        .query("PRAGMA main.wal_checkpoint(TRUNCATE)", ())
        .await
        .map_err(|_| SnapshotError::UnfinishedJournal)?;
    let row = rows
        .next()
        .await
        .map_err(|_| SnapshotError::UnfinishedJournal)?
        .ok_or(SnapshotError::UnfinishedJournal)?;
    if row
        .get::<i64>(0)
        .map_err(|_| SnapshotError::UnfinishedJournal)?
        != 0
    {
        return Err(SnapshotError::UnfinishedJournal);
    }
    drop(row);
    drop(rows);
    let mut rows = conn
        .query("PRAGMA main.journal_mode=DELETE", ())
        .await
        .map_err(|_| SnapshotError::UnfinishedJournal)?;
    let row = rows
        .next()
        .await
        .map_err(|_| SnapshotError::UnfinishedJournal)?
        .ok_or(SnapshotError::UnfinishedJournal)?;
    if row
        .get::<String>(0)
        .map_err(|_| SnapshotError::UnfinishedJournal)?
        != "delete"
    {
        return Err(SnapshotError::UnfinishedJournal);
    }
    Ok(())
}
