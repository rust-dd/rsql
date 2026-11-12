use libsql::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};

mod commands;
mod search;
#[cfg(test)]
mod tests;

pub use commands::*;
pub(crate) use search::{databases, search};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const MAX_SQL_BYTES: usize = 64 * 1024;
const MAX_ERROR_BYTES: usize = 4 * 1024;
const ENABLED_KEY: &str = "history.enabled";
const ENTRY_BYTES: &str = "length(CAST(sql AS BLOB)) + coalesce(length(CAST(error AS BLOB)), 0)";

#[derive(Clone, Copy)]
pub(crate) struct Retention {
    pub max_entries: i64,
    pub max_age_days: i64,
    pub max_bytes: i64,
}

pub(crate) const RETENTION: Retention = Retention {
    max_entries: 10_000,
    max_age_days: 90,
    max_bytes: 32 * 1024 * 1024,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewHistoryEntry {
    pub project_id: String,
    pub database: String,
    pub sql: String,
    pub executed_at: i64,
    pub duration_ms: f64,
    pub row_count: i64,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub project_id: String,
    pub database: String,
    pub sql: String,
    pub sql_truncated: bool,
    pub executed_at: i64,
    pub duration_ms: f64,
    pub row_count: i64,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HistoryFilter {
    pub text: Option<String>,
    pub project_id: Option<String>,
    pub database: Option<String>,
    pub success: Option<bool>,
    pub since: Option<i64>,
    pub before_id: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    pub has_more: bool,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStatus {
    pub enabled: bool,
    pub entries: i64,
    pub bytes: i64,
    pub max_entries: i64,
    pub max_age_days: i64,
    pub max_bytes: i64,
}

pub(crate) async fn create_history_schema(conn: &Connection) -> libsql::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS query_history (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             project_id TEXT NOT NULL,
             database TEXT NOT NULL DEFAULT '',
             sql TEXT NOT NULL,
             sql_truncated INTEGER NOT NULL DEFAULT 0,
             executed_at INTEGER NOT NULL,
             duration_ms REAL NOT NULL DEFAULT 0,
             row_count INTEGER NOT NULL DEFAULT 0,
             success INTEGER NOT NULL,
             error TEXT
         );
         CREATE INDEX IF NOT EXISTS query_history_executed_at ON query_history (executed_at);
         CREATE INDEX IF NOT EXISTS query_history_project ON query_history (project_id, id);",
    )
    .await
    .map(|_| ())
}

// Deleted history should not linger in free pages of the local database file.
async fn erase_deleted_content(conn: &Connection) -> libsql::Result<()> {
    conn.execute_batch("PRAGMA secure_delete = ON")
        .await
        .map(|_| ())
}

fn truncate(text: &str, max_bytes: usize) -> (&str, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

pub(crate) async fn is_enabled(conn: &Connection) -> libsql::Result<bool> {
    let mut rows = conn
        .query("SELECT value FROM settings WHERE key = ?1", [ENABLED_KEY])
        .await?;
    Ok(match rows.next().await? {
        Some(row) => row.get::<String>(0)? != "false",
        None => true,
    })
}

pub(crate) async fn set_enabled(conn: &Connection, enabled: bool) -> libsql::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [ENABLED_KEY, if enabled { "true" } else { "false" }],
    )
    .await
    .map(|_| ())
}

/// Returns false without writing anything while history is disabled.
pub(crate) async fn record(
    conn: &Connection,
    entry: &NewHistoryEntry,
    retention: Retention,
    now_ms: i64,
) -> libsql::Result<bool> {
    if !is_enabled(conn).await? {
        return Ok(false);
    }
    erase_deleted_content(conn).await?;
    let (sql, sql_truncated) = truncate(&entry.sql, MAX_SQL_BYTES);
    let error = entry
        .error
        .as_deref()
        .map(|error| truncate(error, MAX_ERROR_BYTES).0);

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await?;
    tx.execute(
        "INSERT INTO query_history
             (project_id, database, sql, sql_truncated, executed_at, duration_ms, row_count, success, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        libsql::params![
            entry.project_id.as_str(),
            entry.database.as_str(),
            sql,
            sql_truncated,
            entry.executed_at,
            entry.duration_ms,
            entry.row_count,
            entry.success,
            error,
        ],
    )
    .await?;
    prune(&tx, retention, now_ms).await?;
    tx.commit().await?;
    Ok(true)
}

async fn prune(conn: &Connection, retention: Retention, now_ms: i64) -> libsql::Result<()> {
    conn.execute(
        "DELETE FROM query_history WHERE executed_at < ?1",
        [now_ms - retention.max_age_days * DAY_MS],
    )
    .await?;
    conn.execute(
        "DELETE FROM query_history WHERE id <=
             (SELECT id FROM query_history ORDER BY id DESC LIMIT 1 OFFSET ?1)",
        [retention.max_entries],
    )
    .await?;
    conn.execute(
        &format!(
            "DELETE FROM query_history WHERE id <= (
                 SELECT id FROM (
                     SELECT id, sum({ENTRY_BYTES}) OVER (ORDER BY id DESC) AS total
                     FROM query_history
                 ) WHERE total > ?1 ORDER BY id DESC LIMIT 1)"
        ),
        [retention.max_bytes],
    )
    .await?;
    Ok(())
}

pub(crate) async fn status(
    conn: &Connection,
    retention: Retention,
) -> libsql::Result<HistoryStatus> {
    let mut rows = conn
        .query(
            &format!("SELECT count(*), coalesce(sum({ENTRY_BYTES}), 0) FROM query_history"),
            (),
        )
        .await?;
    let row = rows
        .next()
        .await?
        .ok_or(libsql::Error::QueryReturnedNoRows)?;
    Ok(HistoryStatus {
        enabled: is_enabled(conn).await?,
        entries: row.get(0)?,
        bytes: row.get(1)?,
        max_entries: retention.max_entries,
        max_age_days: retention.max_age_days,
        max_bytes: retention.max_bytes,
    })
}

/// Clears one connection's history, or all of it when `project_id` is `None`.
pub(crate) async fn clear(conn: &Connection, project_id: Option<&str>) -> libsql::Result<()> {
    erase_deleted_content(conn).await?;
    conn.execute(
        "DELETE FROM query_history WHERE ?1 IS NULL OR project_id = ?1",
        [project_id],
    )
    .await
    .map(|_| ())
}
