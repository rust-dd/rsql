use std::{collections::BTreeMap, sync::Arc, time::Duration};

use deadpool_postgres::{Manager as PgManager, ManagerConfig, RecyclingMethod, Runtime, Timeouts};

use super::connection_route::{DbTarget, SshTarget, open_route, test_connection};
use super::session_cleanup::reset_session;
use super::{
    ProjectConnectionStatus,
    pooled::{Client, Pool},
};
use crate::ApplicationServices;
use crate::Error;
#[cfg(feature = "structure")]
use deadpool::managed::HookError;
#[cfg(feature = "structure")]
type Hook = deadpool::managed::Hook<super::pooled::Manager>;
#[cfg(not(feature = "structure"))]
use deadpool_postgres::{Hook, HookError};

use crate::services::tls::PgSslMode;

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

/// Bounds a full connection checkout before user SQL starts.
/// Query duration itself stays governed by `statement_timeout`.
pub(super) const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn create_pg_pool(
    cfg: &Config,
    ssl: PgSslMode,
    max_size: usize,
) -> std::result::Result<Pool, Error> {
    let manager_config = ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    };
    let timeouts = Timeouts {
        wait: Some(CONNECTION_TIMEOUT),
        create: Some(CONNECTION_TIMEOUT),
        recycle: Some(CONNECTION_TIMEOUT),
    };

    let mut cfg = cfg.clone();
    ssl.apply(&mut cfg);
    #[cfg(not(feature = "structure"))]
    let manager = match ssl.connector()? {
        Some(tls) => PgManager::from_config(cfg, tls, manager_config),
        None => PgManager::from_config(cfg, NoTls, manager_config),
    };

    #[cfg(feature = "structure")]
    let manager = {
        use super::command_notices::ObservedConnect;
        let original = match ssl.connector()? {
            Some(tls) => PgManager::from_connect(cfg, ObservedConnect(tls), manager_config),
            None => PgManager::from_connect(cfg, ObservedConnect(NoTls), manager_config),
        };
        super::pooled::Manager(original)
    };

    // deadpool needs to know which runtime drives its timers; without this the
    // builder rejects any pool that sets a timeout.
    Pool::builder(manager)
        .max_size(max_size)
        .timeouts(timeouts)
        .runtime(Runtime::Tokio1)
        .post_recycle(Hook::async_fn(|client, _| {
            Box::pin(async move {
                // deadpool's recycle timeout does not cover hooks. A stalled idle
                // connection must be discarded before the user's SQL is sent.
                tokio::time::timeout(CONNECTION_TIMEOUT, reset_session(client))
                    .await
                    .map_err(|_| HookError::message("Session reset timed out"))?
                    .map_err(HookError::Backend)
            })
        }))
        .build()
        .map_err(|e| Error::ConnectionFailed(e.to_string()))
}

pub async fn acquire_client(
    pools_mutex: &tokio::sync::Mutex<BTreeMap<String, Arc<Pool>>>,
    project_id: &str,
) -> std::result::Result<Client, Error> {
    let pool = project_pool(pools_mutex, project_id).await?;
    checked_out(&pool).await
}

pub(super) async fn project_pool(
    pools: &tokio::sync::Mutex<BTreeMap<String, Arc<Pool>>>,
    project: &str,
) -> Result<Arc<Pool>, Error> {
    pools
        .lock()
        .await
        .get(project)
        .cloned()
        .ok_or_else(|| Error::ClientNotConnected(project.to_owned()))
}

pub(super) async fn checked_out(pool: &Pool) -> Result<Client, Error> {
    tokio::time::timeout(CONNECTION_TIMEOUT, pool.get())
        .await
        .map_err(|_| {
            Error::ConnectionFailed(
                "Could not acquire a database connection: checkout timed out".into(),
            )
        })?
        .map_err(|e| {
            Error::ConnectionFailed(format!(
                "Could not acquire a database connection: {}",
                full_error_chain(&e)
            ))
        })
}

fn database_error(error: libsql::Error) -> Error {
    Error::DatabaseError(error.to_string())
}

async fn saved_key(
    conn: &libsql::Connection,
    project_id: &str,
) -> std::result::Result<[String; 6], Error> {
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
        .ok_or_else(|| Error::ProjectNotFound(project_id.to_string()))?;
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

impl ApplicationServices {
    pub async fn test_connection(
        &self,
        key: [&str; 6],
        ssh: Option<Vec<String>>,
    ) -> Result<String, Error> {
        let mut stopped = self.stop_signal();
        if *stopped.borrow() {
            return Err(super::closed());
        }
        let key = key.map(str::to_string);
        let ssl = PgSslMode::from_stored(&key[5])?;
        let ssh = ssh.as_deref().and_then(SshTarget::from_params);
        let local_db = self.local_db.connect().map_err(database_error)?;
        let approvals = &self.ssh_host_keys;
        let target = db_target(&key);
        tokio::select! {
            biased;
            _ = stopped.changed() => Err(super::closed()),
            result = test_connection(&local_db.connection, approvals, &target, ssl, ssh.as_ref()) => result,
        }
    }

    pub async fn connect(
        &self,
        project_id: &str,
        key: Option<[&str; 6]>,
        ssh: Option<Vec<String>>,
    ) -> Result<ProjectConnectionStatus, Error> {
        let ticket = self.project_ticket(project_id).await?;
        self.connect_with_ticket(&ticket, key, ssh).await
    }

    pub async fn connect_with_ticket(
        &self,
        owner: &super::ProjectTicket,
        key: Option<[&str; 6]>,
        ssh: Option<Vec<String>>,
    ) -> Result<ProjectConnectionStatus, Error> {
        let project_id = owner.1.as_str();
        let mut ticket = owner.0.clone();
        let _guard = self.lifecycle.lock().await;
        if !self.owns_ticket(owner) {
            return Err(super::closed());
        }
        if self.clients.lock().await.contains_key(project_id) {
            return Ok(ProjectConnectionStatus::Connected);
        }
        drop(_guard);
        let prepare = async {
            let local_db = self.local_db.connect().map_err(database_error)?;
            let key = match key {
                Some(key) => key.map(str::to_string),
                None => saved_key(&local_db.connection, project_id).await?,
            };
            let ssl = PgSslMode::from_stored(&key[5])?;
            let ssh = ssh.as_deref().and_then(SshTarget::from_params);
            let (mut cfg, tunnel) = open_route(
                &local_db.connection,
                &self.ssh_host_keys,
                &db_target(&key),
                ssh.as_ref(),
            )
            .await?;

            let query_pool = match create_pg_pool(&cfg, ssl, QUERY_POOL_SIZE) {
                Ok(p) => Arc::new(p),
                Err(e) => {
                    tracing::error!("Query pool creation failed: {:?}", e);
                    return Err(Error::ConnectionFailed(full_error_chain(&e)));
                }
            };
            let meta_pool = match create_pg_pool(&cfg, ssl, META_POOL_SIZE) {
                Ok(p) => Arc::new(p),
                Err(e) => {
                    tracing::error!("Meta pool creation failed: {:?}", e);
                    return Err(Error::ConnectionFailed(full_error_chain(&e)));
                }
            };

            // Validate connectivity eagerly so connector keeps previous fail/connected behavior.
            drop(checked_out(&query_pool).await?);
            drop(checked_out(&meta_pool).await?);

            ssl.apply(&mut cfg);
            Ok::<_, Error>((query_pool, meta_pool, (cfg, ssl), tunnel))
        };
        let (query_pool, meta_pool, route, tunnel) = tokio::select! {
            biased;
            _ = ticket.changed() => return Err(super::closed()),
            result = prepare => result?,
        };
        let _guard = self.lifecycle.lock().await;
        if !self.owns_ticket(owner) {
            return Err(super::closed());
        }
        let mut clients = self.clients.lock().await;
        if clients.contains_key(project_id) {
            return Ok(ProjectConnectionStatus::Connected);
        }
        let mut metadata = self.meta_clients.lock().await;
        let mut routes = self.routes.lock().await;
        let mut tunnels = self.ssh_tunnels.lock().await;
        clients.insert(project_id.to_owned(), query_pool);
        metadata.insert(project_id.to_owned(), meta_pool);
        routes.insert(project_id.to_owned(), route);
        if let Some(tunnel) = tunnel {
            tunnels.insert(project_id.to_owned(), tunnel);
        }
        Ok(ProjectConnectionStatus::Connected)
    }
}
