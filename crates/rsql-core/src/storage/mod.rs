//! Local database storage for RSQL profiles.

use libsql::Database;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod history;
mod local;
pub use local::{ConnectionRef, LocalConnection, LocalDatabase};
mod migrations;
pub mod project;
mod project_ids;
pub mod query;
mod schema;
pub mod workspace;

/// Opens the profile database and initializes its required tables.
pub async fn open(path: impl AsRef<Path>) -> libsql::Result<Database> {
    let path = path.as_ref();
    let db = libsql::Builder::new_local(path).build().await?;
    let conn = db.connect()?;
    schema::create_local_schema(&conn).await?;

    if let Err(error) = migrations::cleanup_legacy_snapshots(&conn).await {
        tracing::warn!(%error, "Legacy result snapshot cleanup failed");
    }

    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let backup_path = format!("{}.{started}.bak", path.display());
    match project_ids::migrate_project_ids(&conn, Some(&backup_path)).await {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, %backup_path, "Moved projects to UUID keys"),
        Err(error) => tracing::error!(%error, "Project key migration failed"),
    }

    Ok(db)
}

pub(crate) async fn initialize_staged(
    conn: &libsql::Connection,
    backup_path: &str,
) -> Result<usize, crate::migration::SnapshotError> {
    use crate::migration::SnapshotError;
    schema::create_local_schema(conn)
        .await
        .map_err(|_| SnapshotError::Database("schema initialization"))?;
    migrations::cleanup_legacy_snapshots(conn)
        .await
        .map_err(|_| SnapshotError::Database("obsolete result cleanup"))?;
    let migrated = project_ids::migrate_project_ids(conn, Some(backup_path))
        .await
        .map_err(|_| SnapshotError::Database("project ID migration"))?;
    crate::ssh::host_keys::initialize(conn)
        .await
        .map_err(|_| SnapshotError::Database("SSH trust initialization"))?;
    Ok(migrated)
}

/// Existing objects with a conflicting layout can make CREATE TABLE IF NOT EXISTS succeed.
/// Prepare service reads and the SSH approval upsert on the read-only staged connection.
pub(crate) async fn verify_staged_schema(
    conn: &libsql::Connection,
) -> Result<(), crate::migration::SnapshotError> {
    for sql in [
        "SELECT id, sql FROM queries LIMIT 0",
        "SELECT name, tabs FROM workspaces LIMIT 0",
        "SELECT key, value FROM settings LIMIT 0",
        "SELECT id, project_id, database, sql, sql_truncated, executed_at, duration_ms, row_count, success, error FROM query_history LIMIT 0",
        crate::ssh::host_keys::UPSERT_KNOWN_HOST,
    ] {
        conn.prepare(sql)
            .await
            .map_err(|_| crate::migration::SnapshotError::Database("checking migrated schema"))?;
    }
    Ok(())
}
