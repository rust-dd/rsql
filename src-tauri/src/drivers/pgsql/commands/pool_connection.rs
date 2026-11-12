use std::{collections::BTreeMap, sync::Arc, time::Duration};

use deadpool_postgres::{
    Hook, HookError, Manager as PgManager, ManagerConfig, Pool, RecyclingMethod, Runtime, Timeouts,
};

use super::connection_route::{DbTarget, SshTarget, open_route, test_connection};
use super::session_cleanup::reset_session;
use crate::AppState;
use crate::common::enums::{AppError, ProjectConnectionStatus};
use crate::drivers::pgsql::get_pool;
use crate::drivers::pgsql::tls::PgSslMode;

use tauri::{AppHandle, Manager, Result, State};
use tokio_postgres::{Config, NoTls};

pub(crate) fn full_error_chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(cause) = src {
        msg.push_str(": ");
        msg.push_str(&cause.to_string());
        src = cause.source();
    }
    msg
}

/// Connections held per project for user queries and for metadata lookups.
/// Several projects can be open at once, so these are kept modest rather than
/// letting one window monopolize a shared server's connection budget.
pub(crate) const QUERY_POOL_SIZE: usize = 8;
pub(crate) const META_POOL_SIZE: usize = 4;

/// Waiting for a free connection is bounded: an exhausted pool used to block
/// the caller indefinitely, which showed up as a window that had simply stopped
/// responding. Query duration itself stays governed by `statement_timeout`.
const POOL_WAIT: Duration = Duration::from_secs(10);

pub(crate) fn create_pg_pool(
    cfg: &Config,
    ssl: PgSslMode,
    max_size: usize,
) -> std::result::Result<Pool, AppError> {
    let manager_config = ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    };
    let timeouts = Timeouts {
        wait: Some(POOL_WAIT),
        ..Timeouts::default()
    };

    let mut cfg = cfg.clone();
    ssl.apply(&mut cfg);
    let manager = match ssl.connector()? {
        Some(tls) => PgManager::from_config(cfg, tls, manager_config),
        None => PgManager::from_config(cfg, NoTls, manager_config),
    };

    // deadpool needs to know which runtime drives its timers; without this the
    // builder rejects any pool that sets a timeout.
    Pool::builder(manager)
        .max_size(max_size)
        .timeouts(timeouts)
        .runtime(Runtime::Tokio1)
        .post_recycle(Hook::async_fn(|client, _| {
            Box::pin(async move { reset_session(client).await.map_err(HookError::Backend) })
        }))
        .build()
        .map_err(|e| AppError::ConnectionFailed(e.to_string()))
}

pub(crate) async fn acquire_client(
    pools_mutex: &tokio::sync::Mutex<BTreeMap<String, Arc<Pool>>>,
    project_id: &str,
) -> std::result::Result<deadpool_postgres::Client, AppError> {
    let pool = {
        let pools = pools_mutex.lock().await;
        get_pool(&pools, project_id)?
    };

    pool.get().await.map_err(|e| {
        AppError::ConnectionFailed(format!(
            "No connection available within {}s: {}",
            POOL_WAIT.as_secs(),
            e
        ))
    })
}

fn database_error(error: libsql::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

async fn saved_key(
    conn: &libsql::Connection,
    project_id: &str,
) -> std::result::Result<[String; 6], AppError> {
    let mut rows = conn
        .query(
            "SELECT username, password, database, host, port, ssl FROM projects WHERE id = ?1",
            libsql::params![project_id],
        )
        .await
        .map_err(database_error)?;
    let row = rows
        .next()
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::ProjectNotFound(project_id.to_string()))?;
    Ok(std::array::from_fn(|index| {
        row.get::<String>(index as i32).unwrap_or_default()
    }))
}

fn db_target(key: &[String; 6]) -> DbTarget<'_> {
    DbTarget {
        user: &key[0],
        password: &key[1],
        database: &key[2],
        host: &key[3],
        port: key[4].parse().unwrap_or(5432),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_test_connection(
    key: [&str; 6],
    ssh: Option<Vec<String>>,
    app_state: State<'_, AppState>,
) -> Result<String> {
    let key = key.map(str::to_string);
    let ssl = PgSslMode::from_stored(&key[5])?;
    let ssh = ssh.as_deref().and_then(SshTarget::from_params);
    let local_db = app_state.local_db.connect().map_err(database_error)?;
    let approvals = &app_state.ssh_host_keys;
    Ok(test_connection(&local_db, approvals, &db_target(&key), ssl, ssh.as_ref()).await?)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_connector(
    project_id: &str,
    key: Option<[&str; 6]>,
    ssh: Option<Vec<String>>,
    app: AppHandle,
) -> Result<ProjectConnectionStatus> {
    let app_state = app.state::<AppState>();
    {
        let clients = app_state.clients.lock().await;
        if clients.contains_key(project_id) {
            return Ok(ProjectConnectionStatus::Connected);
        }
    }

    let local_db = app_state.local_db.connect().map_err(database_error)?;
    let key = match key {
        Some(key) => key.map(str::to_string),
        None => saved_key(&local_db, project_id).await?,
    };
    let ssl = PgSslMode::from_stored(&key[5])?;
    let ssh = ssh.as_deref().and_then(SshTarget::from_params);
    if ssh.is_some() {
        app_state.ssh_tunnels.lock().await.remove(project_id);
    }
    let (cfg, tunnel) = open_route(
        &local_db,
        &app_state.ssh_host_keys,
        &db_target(&key),
        ssh.as_ref(),
    )
    .await?;

    let query_pool = match create_pg_pool(&cfg, ssl, QUERY_POOL_SIZE) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            tracing::error!("Query pool creation failed: {:?}", e);
            return Err(AppError::ConnectionFailed(full_error_chain(&e)).into());
        }
    };
    let meta_pool = match create_pg_pool(&cfg, ssl, META_POOL_SIZE) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            tracing::error!("Meta pool creation failed: {:?}", e);
            return Err(AppError::ConnectionFailed(full_error_chain(&e)).into());
        }
    };

    // Validate connectivity eagerly so connector keeps previous fail/connected behavior.
    if let Err(e) = query_pool.get().await {
        tracing::error!("Query pool initial connection failed: {:?}", e);
        return Err(AppError::ConnectionFailed(full_error_chain(&e)).into());
    }
    if let Err(e) = meta_pool.get().await {
        tracing::error!("Meta pool initial connection failed: {:?}", e);
        return Err(AppError::ConnectionFailed(full_error_chain(&e)).into());
    }

    {
        let mut clients = app_state.clients.lock().await;
        clients.insert(project_id.to_string(), Arc::clone(&query_pool));
    }
    {
        let mut meta_clients = app_state.meta_clients.lock().await;
        meta_clients.insert(project_id.to_string(), Arc::clone(&meta_pool));
    }
    {
        let mut client_ssl = app_state.client_ssl.lock().await;
        client_ssl.insert(project_id.to_string(), ssl);
    }
    if let Some(tunnel) = tunnel {
        app_state
            .ssh_tunnels
            .lock()
            .await
            .insert(project_id.to_string(), tunnel);
    }

    Ok(ProjectConnectionStatus::Connected)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Building a pool only happens on connect, which no other test reaches.
    /// A misconfigured builder therefore surfaced as a runtime connection
    /// failure rather than a compile or test error.
    #[test]
    fn pools_build_with_their_timeouts_configured() {
        let cfg = Config::new();
        assert!(create_pg_pool(&cfg, PgSslMode::Disable, QUERY_POOL_SIZE).is_ok());
    }

    #[test]
    fn tls_pools_build_too() {
        let cfg = Config::new();
        assert!(create_pg_pool(&cfg, PgSslMode::VerifyFull, META_POOL_SIZE).is_ok());
    }
}
