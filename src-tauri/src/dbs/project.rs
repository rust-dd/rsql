use crate::AppState;
use crate::common::BTreeVecStore;
use crate::common::enums::AppError;
use std::collections::BTreeMap;
use tauri::{Result, State};

#[tauri::command(rename_all = "snake_case")]
pub async fn project_db_select(app_state: State<'_, AppState>) -> Result<BTreeVecStore> {
    let conn = app_state
        .local_db
        .connect()
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let mut rows = conn
        .query(
            "SELECT id, driver, username, password, database, host, port, ssl, ssh_enabled, ssh_host, ssh_port, ssh_user, ssh_password, ssh_key_path FROM projects ORDER BY id",
            (),
        )
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let mut projects = BTreeMap::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
    {
        let id: String = row
            .get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let driver: String = row
            .get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let username: String = row
            .get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let password: String = row
            .get(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let database: String = row
            .get(4)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let host: String = row
            .get(5)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let port: String = row
            .get(6)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let ssl: String = row
            .get(7)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let ssh_enabled: String = row.get::<String>(8).unwrap_or_default();
        let ssh_host: String = row.get::<String>(9).unwrap_or_default();
        let ssh_port: String = row.get::<String>(10).unwrap_or_default();
        let ssh_user: String = row.get::<String>(11).unwrap_or_default();
        let ssh_password: String = row.get::<String>(12).unwrap_or_default();
        let ssh_key_path: String = row.get::<String>(13).unwrap_or_default();
        projects.insert(
            id,
            vec![
                driver,
                username,
                password,
                database,
                host,
                port,
                ssl,
                ssh_enabled,
                ssh_host,
                ssh_port,
                ssh_user,
                ssh_password,
                ssh_key_path,
            ],
        );
    }
    Ok(projects)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn project_db_insert(
    project_id: &str,
    project_details: Vec<String>,
    app_state: State<'_, AppState>,
) -> Result<()> {
    let conn = app_state
        .local_db
        .connect()
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let driver = project_details.first().cloned().unwrap_or_default();
    let username = project_details.get(1).cloned().unwrap_or_default();
    let password = project_details.get(2).cloned().unwrap_or_default();
    let database = project_details.get(3).cloned().unwrap_or_default();
    let host = project_details.get(4).cloned().unwrap_or_default();
    let port = project_details.get(5).cloned().unwrap_or_default();
    let ssl = project_details
        .get(6)
        .cloned()
        .unwrap_or("false".to_string());
    let ssh_enabled = project_details.get(7).cloned().unwrap_or_default();
    let ssh_host = project_details.get(8).cloned().unwrap_or_default();
    let ssh_port = project_details.get(9).cloned().unwrap_or_default();
    let ssh_user = project_details.get(10).cloned().unwrap_or_default();
    let ssh_password = project_details.get(11).cloned().unwrap_or_default();
    let ssh_key_path = project_details.get(12).cloned().unwrap_or_default();

    conn.execute(
        "INSERT OR REPLACE INTO projects (id, driver, username, password, database, host, port, ssl, ssh_enabled, ssh_host, ssh_port, ssh_user, ssh_password, ssh_key_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        libsql::params![project_id, driver, username, password, database, host, port, ssl, ssh_enabled, ssh_host, ssh_port, ssh_user, ssh_password, ssh_key_path],
    )
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn project_db_delete(project_id: &str, app_state: State<'_, AppState>) -> Result<()> {
    let conn = app_state
        .local_db
        .connect()
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    conn.execute(
        "DELETE FROM projects WHERE id = ?1",
        libsql::params![project_id],
    )
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    // Best-effort cleanup for in-memory connection state.
    app_state.clients.lock().await.remove(project_id);
    app_state.meta_clients.lock().await.remove(project_id);
    app_state.client_ssl.lock().await.remove(project_id);

    app_state.executions.cancel_project(project_id);

    // A LISTEN task and an SSH tunnel outlive the project otherwise: the task
    // keeps polling a connection that is gone, and the tunnel holds its port.
    // Listener handles are keyed "<project>:<channel>", so every channel of
    // this project has to go.
    {
        let mut handles = app_state.notify_handles.lock().await;
        let prefix = format!("{}:", project_id);
        let owned: Vec<String> = handles
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .cloned()
            .collect();
        for key in owned {
            if let Some(handle) = handles.remove(&key) {
                handle.abort();
            }
        }
    }
    app_state.ssh_tunnels.lock().await.remove(project_id);

    Ok(())
}
