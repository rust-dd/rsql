use std::{collections::BTreeMap, sync::Arc, time::Duration};

use deadpool_postgres::{
    Manager as PgManager, ManagerConfig, Pool, RecyclingMethod, Runtime, Timeouts,
};

use crate::AppState;
use crate::common::enums::{AppError, ProjectConnectionStatus};
use crate::drivers::pgsql::get_pool;

use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use tauri::{AppHandle, Manager, Result};
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
    use_ssl: bool,
    max_size: usize,
) -> std::result::Result<Pool, AppError> {
    let manager_config = ManagerConfig {
        recycling_method: RecyclingMethod::Custom("ROLLBACK".into()),
    };
    let timeouts = Timeouts {
        wait: Some(POOL_WAIT),
        ..Timeouts::default()
    };

    // deadpool needs to know which runtime drives its timers; without this the
    // builder rejects any pool that sets a timeout.
    if use_ssl {
        let tls_connector = TlsConnector::builder()
            .build()
            .map_err(|e| AppError::ConnectionFailed(e.to_string()))?;
        let tls = MakeTlsConnector::new(tls_connector);
        let manager = PgManager::from_config(cfg.clone(), tls, manager_config);
        Pool::builder(manager)
            .max_size(max_size)
            .timeouts(timeouts)
            .runtime(Runtime::Tokio1)
            .build()
            .map_err(|e| AppError::ConnectionFailed(e.to_string()))
    } else {
        let manager = PgManager::from_config(cfg.clone(), NoTls, manager_config);
        Pool::builder(manager)
            .max_size(max_size)
            .timeouts(timeouts)
            .runtime(Runtime::Tokio1)
            .build()
            .map_err(|e| AppError::ConnectionFailed(e.to_string()))
    }
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

#[tauri::command(rename_all = "snake_case")]
pub async fn pgsql_test_connection(key: [&str; 6]) -> Result<String> {
    let user = key[0];
    let password = key[1];
    let database = key[2];
    let host = key[3];
    let port: u16 = key[4].parse().unwrap_or(5432);
    let use_ssl = key[5] == "true";

    let mut cfg = Config::new();
    cfg.user(user)
        .password(password)
        .dbname(database)
        .host(host)
        .port(port);

    let pool = create_pg_pool(&cfg, use_ssl, 1)?;
    let client = pool
        .get()
        .await
        .map_err(|e| AppError::ConnectionFailed(full_error_chain(&e)))?;

    let row = client
        .query_one("SELECT version()", &[])
        .await
        .map_err(|e| AppError::ConnectionFailed(e.to_string()))?;

    let version: String = row.get(0);
    Ok(version)
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

    let (user, password, database, host, port_str, use_ssl) = match key {
        Some(key) => (
            key[0].to_string(),
            key[1].to_string(),
            key[2].to_string(),
            key[3].to_string(),
            key[4].to_string(),
            key[5] == "true",
        ),
        None => {
            let conn = app_state
                .local_db
                .connect()
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
            let mut rows = conn
                .query(
                    "SELECT username, password, database, host, port, ssl FROM projects WHERE id = ?1",
                    libsql::params![project_id],
                )
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
            let row = rows
                .next()
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?
                .ok_or_else(|| AppError::ProjectNotFound(project_id.to_string()))?;
            (
                row.get::<String>(0).unwrap_or_default(),
                row.get::<String>(1).unwrap_or_default(),
                row.get::<String>(2).unwrap_or_default(),
                row.get::<String>(3).unwrap_or_default(),
                row.get::<String>(4).unwrap_or_default(),
                row.get::<String>(5).map(|s| s == "true").unwrap_or(false),
            )
        }
    };

    let (effective_host, effective_port_str) = if let Some(ref ssh_params) = ssh {
        // ssh_params: [ssh_host, ssh_port, ssh_user, ssh_password, ssh_key_path]
        if ssh_params.len() >= 3 && !ssh_params[0].is_empty() {
            let ssh_host = &ssh_params[0];
            let ssh_port: u16 = ssh_params[1].parse().unwrap_or(22);
            let ssh_user = &ssh_params[2];
            let ssh_password = ssh_params
                .get(3)
                .filter(|s| !s.is_empty())
                .map(|s| s.as_str());
            let ssh_key_path = ssh_params
                .get(4)
                .filter(|s| !s.is_empty())
                .map(|s| s.as_str());

            app_state.ssh_tunnels.lock().await.remove(project_id);

            let conn = app_state
                .local_db
                .connect()
                .map_err(|error| AppError::DatabaseError(error.to_string()))?;
            let expected =
                crate::ssh::host_keys::known_fingerprint(&conn, ssh_host, ssh_port).await?;
            let connection = crate::ssh::SshConnection {
                host: ssh_host,
                port: ssh_port,
                user: ssh_user,
                password: ssh_password,
                key_path: ssh_key_path,
                expected_fingerprint: expected.clone(),
            };
            let tunnel = match crate::ssh::start_tunnel(
                &connection,
                &host,
                port_str.parse().unwrap_or(5432),
            )
            .await
            {
                Ok(tunnel) => tunnel,
                Err(crate::ssh::TunnelError::HostKey(fingerprint)) => {
                    let challenge = app_state
                        .ssh_host_keys
                        .challenge(ssh_host, ssh_port, fingerprint, expected)
                        .await;
                    let json = serde_json::to_string(&challenge)
                        .map_err(|error| AppError::SerializationError(error.to_string()))?;
                    return Err(AppError::ConnectionFailed(format!("SSH_HOST_KEY:{json}")).into());
                }
                Err(error) => return Err(AppError::ConnectionFailed(error.to_string()).into()),
            };

            let local_port = tunnel.local_port;
            app_state
                .ssh_tunnels
                .lock()
                .await
                .insert(project_id.to_string(), tunnel);

            ("127.0.0.1".to_string(), local_port.to_string())
        } else {
            (host.clone(), port_str.clone())
        }
    } else {
        (host.clone(), port_str.clone())
    };

    let port: u16 = effective_port_str.parse().unwrap_or(5432);
    let mut cfg = Config::new();
    cfg.user(&user)
        .password(&password)
        .dbname(&database)
        .host(&effective_host)
        .port(port);

    let query_pool = match create_pg_pool(&cfg, use_ssl, QUERY_POOL_SIZE) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            tracing::error!("Query pool creation failed: {:?}", e);
            return Err(AppError::ConnectionFailed(full_error_chain(&e)).into());
        }
    };
    let meta_pool = match create_pg_pool(&cfg, use_ssl, META_POOL_SIZE) {
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
        client_ssl.insert(project_id.to_string(), use_ssl);
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
        assert!(create_pg_pool(&cfg, false, QUERY_POOL_SIZE).is_ok());
    }

    #[test]
    fn tls_pools_build_too() {
        let cfg = Config::new();
        assert!(create_pg_pool(&cfg, true, META_POOL_SIZE).is_ok());
    }
}
