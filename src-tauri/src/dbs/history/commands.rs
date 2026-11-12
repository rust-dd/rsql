use std::time::{SystemTime, UNIX_EPOCH};

use libsql::Connection;
use tauri::{Result, State};

use super::{HistoryFilter, HistoryPage, HistoryStatus, NewHistoryEntry, RETENTION};
use crate::AppState;
use crate::common::enums::AppError;

fn local_db(app_state: &AppState) -> std::result::Result<Connection, AppError> {
    app_state.local_db.connect().map_err(db_error)
}

fn db_error(error: libsql::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

#[tauri::command(rename_all = "snake_case")]
pub async fn history_record(
    entry: NewHistoryEntry,
    app_state: State<'_, AppState>,
) -> Result<bool> {
    let conn = local_db(&app_state)?;
    Ok(super::record(&conn, &entry, RETENTION, now_ms())
        .await
        .map_err(db_error)?)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn history_search(
    filter: HistoryFilter,
    app_state: State<'_, AppState>,
) -> Result<HistoryPage> {
    let conn = local_db(&app_state)?;
    Ok(super::search(&conn, &filter).await.map_err(db_error)?)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn history_databases(
    project_id: Option<String>,
    app_state: State<'_, AppState>,
) -> Result<Vec<String>> {
    let conn = local_db(&app_state)?;
    Ok(super::databases(&conn, project_id.as_deref())
        .await
        .map_err(db_error)?)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn history_status(app_state: State<'_, AppState>) -> Result<HistoryStatus> {
    let conn = local_db(&app_state)?;
    Ok(super::status(&conn, RETENTION).await.map_err(db_error)?)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn history_set_enabled(enabled: bool, app_state: State<'_, AppState>) -> Result<()> {
    let conn = local_db(&app_state)?;
    Ok(super::set_enabled(&conn, enabled).await.map_err(db_error)?)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn history_clear(app_state: State<'_, AppState>) -> Result<()> {
    let conn = local_db(&app_state)?;
    Ok(super::clear(&conn, None).await.map_err(db_error)?)
}
