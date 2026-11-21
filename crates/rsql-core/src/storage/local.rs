use crate::migration::{DATABASE_FILENAME, ProfileLease};
use libsql::{Connection, Database, params::IntoParams};
use std::{sync::Arc, time::Duration};

/// The service's connection factory. Its file-backed profile stays leased through every clone
/// of every issued connection, including clones sent to interrupt blocking SQLite work.
pub struct LocalDatabase {
    database: Database,
    lease: Option<Arc<ProfileLease>>,
}

impl LocalDatabase {
    pub(crate) fn from_database(database: Database) -> Self {
        Self {
            database,
            lease: None,
        }
    }

    pub(crate) async fn open(lease: Arc<ProfileLease>) -> libsql::Result<Self> {
        let database = super::open(lease.directory().join(DATABASE_FILENAME)).await?;
        Ok(Self {
            database,
            lease: Some(lease),
        })
    }

    /// Clone the existing owner when starting a source observer on this profile.
    pub fn profile_lease(&self) -> Option<Arc<ProfileLease>> {
        self.lease.clone()
    }

    pub fn connect(&self) -> libsql::Result<LocalConnection> {
        let connection = self.database.connect()?;
        // Startup reads overlap profile writes; without a timeout they fail at once as busy.
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(LocalConnection {
            connection,
            _lease: self.lease.clone(),
        })
    }
}

#[derive(Clone)]
pub struct LocalConnection {
    pub(crate) connection: Connection,
    _lease: Option<Arc<ProfileLease>>,
}

/// Shared storage routines accept either a leased connection or the caller's own SQLite
/// connection. Only core storage can access the borrowed raw handle.
pub struct ConnectionRef<'a>(pub(super) &'a Connection);

impl<'a> From<&'a LocalConnection> for ConnectionRef<'a> {
    fn from(conn: &'a LocalConnection) -> Self {
        Self(&conn.connection)
    }
}

impl<'a> From<&'a Connection> for ConnectionRef<'a> {
    fn from(conn: &'a Connection) -> Self {
        Self(conn)
    }
}

impl<'a> From<&'a libsql::Transaction> for ConnectionRef<'a> {
    fn from(transaction: &'a libsql::Transaction) -> Self {
        Self(transaction)
    }
}

impl LocalConnection {
    pub fn interrupt(&self) -> libsql::Result<()> {
        self.connection.interrupt()
    }

    pub fn busy_timeout(&self, duration: Duration) -> libsql::Result<()> {
        self.connection.busy_timeout(duration)
    }

    pub async fn execute(&self, sql: &str, params: impl IntoParams) -> libsql::Result<u64> {
        self.connection.execute(sql, params).await
    }

    pub async fn execute_batch(&self, sql: &str) -> libsql::Result<()> {
        self.connection.execute_batch(sql).await.map(drop)
    }

    pub fn on_update(&self, notify: impl Fn() + Send + Sync + 'static) -> libsql::Result<()> {
        self.connection
            .add_update_hook(Box::new(move |_, _, _, _| notify()))
    }
}
