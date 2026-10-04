use crate::AppState;
use crate::common::enums::AppError;
use crate::drivers::pgsql::{
    close_virtual, execute_query, execute_query_packed, execute_query_streamed, execute_virtual,
    fetch_virtual_page,
};

use tauri::ipc::Response;
use tauri::{AppHandle, Manager, Result, State};

use super::query_session::run_user_query;

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_run_query(
    project_id: &str,
    sql: &str,
    exec_id: &str,
    app_state: State<'_, AppState>,
) -> Result<Response> {
    let result = run_user_query(&app_state, project_id, exec_id, None, async |client| {
        execute_query(client, sql).await
    })
    .await?;
    let json = sonic_rs::to_string(&result).map_err(|e| AppError::QueryFailed(e.to_string()))?;
    Ok(Response::new(json))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_cancel_query(exec_id: &str, app_state: State<'_, AppState>) -> Result<bool> {
    Ok(app_state.executions.cancel(exec_id))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_run_query_packed(
    project_id: &str,
    sql: &str,
    exec_id: &str,
    timeout_ms: Option<u32>,
    app_state: State<'_, AppState>,
) -> Result<Response> {
    let result = run_user_query(
        &app_state,
        project_id,
        exec_id,
        timeout_ms,
        async |client| execute_query_packed(client, sql).await,
    )
    .await?;
    let json = sonic_rs::to_string(&result).map_err(|e| AppError::QueryFailed(e.to_string()))?;
    Ok(Response::new(json))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_run_query_streamed(
    project_id: &str,
    sql: &str,
    stream_id: &str,
    exec_id: &str,
    app: AppHandle,
) -> Result<()> {
    let app_state = app.state::<AppState>();
    run_user_query(&app_state, project_id, exec_id, None, async |client| {
        execute_query_streamed(client, sql, stream_id, &app).await
    })
    .await
    .map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_execute_virtual(
    project_id: &str,
    sql: &str,
    query_id: &str,
    exec_id: &str,
    page_size: usize,
    timeout_ms: Option<u32>,
    app_state: State<'_, AppState>,
) -> Result<Response> {
    let result = run_user_query(
        &app_state,
        project_id,
        exec_id,
        timeout_ms,
        async |client| {
            execute_virtual(client, &app_state.virtual_cache, sql, query_id, page_size).await
        },
    )
    .await;
    if result.is_err() {
        close_virtual(&app_state.virtual_cache, query_id).await?;
    }
    let result = result?;
    let json = sonic_rs::to_string(&result).map_err(|e| AppError::QueryFailed(e.to_string()))?;
    Ok(Response::new(json))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_fetch_page(
    query_id: &str,
    col_count: usize,
    offset: usize,
    limit: usize,
    app_state: State<'_, AppState>,
) -> Result<Response> {
    let packed =
        fetch_virtual_page(&app_state.virtual_cache, query_id, col_count, offset, limit).await?;
    let json = sonic_rs::to_string(&packed).map_err(|e| AppError::QueryFailed(e.to_string()))?;
    Ok(Response::new(json))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_close_virtual(query_id: &str, app_state: State<'_, AppState>) -> Result<()> {
    close_virtual(&app_state.virtual_cache, query_id).await?;
    Ok(())
}
