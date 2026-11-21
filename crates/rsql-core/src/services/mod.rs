#[cfg(feature = "structure")]
mod command_notices;
mod connection_route;
mod executions;
mod metadata_session;
pub mod notifications;
mod pool_connection;
pub mod pooled;
mod query_session;
mod write_session;
pub use write_session::{WriteOutcome, run_guarded_write};
#[cfg(feature = "structure")]
pub mod extension_action;
#[cfg(feature = "structure")]
pub mod object_action;
#[cfg(feature = "structure")]
mod schema_version;
mod session_cleanup;
mod status;
#[cfg(feature = "structure")]
mod structure_apply;
pub mod tls;

pub use executions::QueryExecutions;
pub use pool_connection::acquire_client;
pub(crate) use query_session::run_until_closed;
pub use query_session::{run_guarded_query, run_user_query};
pub use status::ProjectConnectionStatus;

use crate::{
    Error,
    migration::ProfileLease,
    query::{VirtualCache, evict_idle},
    ssh::{SshTunnel, host_keys},
    storage::LocalDatabase,
};
use pooled::Pool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tls::PgSslMode;
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
};

type Pools = Arc<Mutex<BTreeMap<String, Arc<Pool>>>>;

/// Identifies the lifetime of a project connection attempt or listener registration.
#[derive(Clone)]
pub struct ProjectTicket(watch::Receiver<()>, String);

impl ProjectTicket {
    pub fn same_generation(&self, other: &Self) -> bool {
        self.0.same_channel(&other.0)
    }
    pub fn is_current(&self) -> bool {
        self.0.has_changed().is_ok()
    }

    /// Wait for this project's route generation to retire, including prior retirement.
    pub async fn retired(&self) {
        let _ = self.0.clone().changed().await;
    }
}

pub struct ApplicationServices {
    pub local_db: LocalDatabase,
    pub clients: Pools,
    pub meta_clients: Pools,
    pub executions: Arc<QueryExecutions>,
    routes: Mutex<BTreeMap<String, (tokio_postgres::Config, PgSslMode)>>,
    pub virtual_cache: Arc<Mutex<VirtualCache>>,
    notify_handles: notifications::Listeners,
    ssh_tunnels: Mutex<BTreeMap<String, SshTunnel>>,
    ssh_host_keys: host_keys::HostKeyApprovals,
    route_activity: Mutex<BTreeMap<String, watch::Sender<()>>>,
    lifecycle: Mutex<()>,
    tickets: std::sync::Mutex<BTreeMap<String, watch::Sender<()>>>,
    stopped: watch::Sender<bool>,
    maintenance: JoinHandle<()>,
}

fn closed() -> Error {
    Error::ConnectionFailed("Connection closed".into())
}

impl ApplicationServices {
    /// Initialize a caller-managed database, including in-memory stores and isolated fixtures.
    pub async fn new(local_db: libsql::Database) -> Result<Self, Error> {
        Self::initialize(LocalDatabase::from_database(local_db)).await
    }

    /// Acquire the profile before running any local SQLite initialization.
    pub async fn acquire_profile(directory: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let lease =
            ProfileLease::acquire(directory).map_err(|e| Error::DatabaseError(e.to_string()))?;
        Self::open_profile(Arc::new(lease)).await
    }

    pub async fn open_profile(lease: Arc<ProfileLease>) -> Result<Self, Error> {
        lease
            .require_resolved_import()
            .map_err(|e| Error::DatabaseError(e.to_string()))?;
        let db = LocalDatabase::open(lease)
            .await
            .map_err(|e| Error::DatabaseError(e.to_string()))?;
        Self::initialize(db).await
    }

    async fn initialize(local_db: LocalDatabase) -> Result<Self, Error> {
        let db_error = |e: libsql::Error| Error::DatabaseError(e.to_string());
        let conn = local_db.connect().map_err(db_error)?;
        host_keys::initialize(&conn.connection)
            .await
            .map_err(db_error)?;
        let virtual_cache = Arc::new(Mutex::new(VirtualCache::new()));
        let cache = Arc::downgrade(&virtual_cache);
        let (stopped, mut stop) = watch::channel(false);
        let maintenance = tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    biased;
                    _ = stop.changed() => break,
                    _ = timer.tick() => {
                        let Some(cache) = cache.upgrade() else { break };
                        evict_idle(&cache).await;
                    }
                }
            }
        });
        Ok(Self {
            local_db,
            virtual_cache,
            stopped,
            maintenance,
            clients: Default::default(),
            meta_clients: Default::default(),
            executions: Default::default(),
            routes: Default::default(),
            notify_handles: Default::default(),
            ssh_tunnels: Default::default(),
            ssh_host_keys: Default::default(),
            route_activity: Default::default(),
            lifecycle: Default::default(),
            tickets: Default::default(),
        })
    }

    fn stop_signal(&self) -> watch::Receiver<bool> {
        self.stopped.subscribe()
    }

    // Removing the sender invalidates every outstanding attempt for this project.
    pub async fn project_ticket(&self, project: &str) -> Result<ProjectTicket, Error> {
        let _guard = self.lifecycle.lock().await;
        self.capture_project_ticket(project)
    }

    /// Capture a dispatch-time identity without waiting for pool or tunnel cleanup.
    pub fn capture_project_ticket(&self, project: &str) -> Result<ProjectTicket, Error> {
        let mut projects = self.tickets.lock().unwrap();
        if *self.stopped.borrow() {
            return Err(closed());
        }
        Ok(ProjectTicket(
            projects
                .entry(project.to_owned())
                .or_insert_with(|| watch::channel(()).0)
                .subscribe(),
            project.to_owned(),
        ))
    }

    fn owns_ticket(&self, ticket: &ProjectTicket) -> bool {
        self.tickets
            .lock()
            .unwrap()
            .get(&ticket.1)
            .is_some_and(|sender| sender.subscribe().same_channel(&ticket.0))
    }

    /// Returns the published socket route, database hostname, and TLS policy for this project.
    pub async fn active_route(
        &self,
        project: &str,
    ) -> Result<(tokio_postgres::Config, PgSslMode), Error> {
        let _guard = self.lifecycle.lock().await;
        self.published_route(project).await
    }

    async fn published_route(
        &self,
        project: &str,
    ) -> Result<(tokio_postgres::Config, PgSslMode), Error> {
        self.routes
            .lock()
            .await
            .get(project)
            .cloned()
            .ok_or_else(|| Error::ClientNotConnected(project.to_owned()))
    }

    pub async fn trust_host_key(&self, id: &str) -> Result<(), Error> {
        if *self.stopped.borrow() {
            return Err(closed());
        }
        let conn = self
            .local_db
            .connect()
            .map_err(|e| Error::DatabaseError(e.to_string()))?;
        self.ssh_host_keys.approve(id, &conn.connection).await
    }

    pub(super) async fn register_activity(&self, project: &str) -> watch::Receiver<()> {
        self.route_activity
            .lock()
            .await
            .entry(project.into())
            .or_insert_with(|| watch::channel(()).0)
            .subscribe()
    }

    pub(super) async fn drain_activity(&self, project: Option<&str>) {
        let mut active = self.route_activity.lock().await;
        if let Some(project) = project {
            if let Some(route) = active.get(project) {
                route.send_replace(());
                route.closed().await;
            }
            active.remove(project);
        } else {
            for route in active.values() {
                route.send_replace(());
            }
            for route in active.values() {
                route.closed().await;
            }
            active.clear();
        }
    }

    pub async fn disconnect(&self, project: &str) {
        let _guard = self.lifecycle.lock().await;
        self.tickets.lock().unwrap().remove(project);
        self.executions.cancel_project(project);
        self.drain_activity(Some(project)).await;
        for pools in [&self.clients, &self.meta_clients] {
            if let Some(pool) = pools.lock().await.remove(project) {
                pool.close();
            }
        }
        self.routes.lock().await.remove(project);
        self.notify_handles.stop_project(project).await;
        if let Some(tunnel) = self.ssh_tunnels.lock().await.remove(project) {
            tunnel.close().await;
        }
        // Dispatch can capture a ticket while this lock is draining the old route.
        // Invalidate those queued requests before admitting the next connection.
        self.tickets.lock().unwrap().remove(project);
    }

    pub async fn shutdown(&self) {
        let _guard = self.lifecycle.lock().await;
        self.stopped.send_replace(true);
        self.tickets.lock().unwrap().clear();
        self.maintenance.abort();
        self.executions.shutdown();
        self.virtual_cache.lock().await.shutdown();
        self.drain_activity(None).await;
        for pools in [&self.clients, &self.meta_clients] {
            for pool in std::mem::take(&mut *pools.lock().await).into_values() {
                pool.close();
            }
        }
        self.routes.lock().await.clear();
        self.notify_handles.stop_all().await;
        for tunnel in std::mem::take(&mut *self.ssh_tunnels.lock().await).into_values() {
            tunnel.close().await;
        }
    }
}

impl Drop for ApplicationServices {
    fn drop(&mut self) {
        self.maintenance.abort();
        self.executions.shutdown();
    }
}
