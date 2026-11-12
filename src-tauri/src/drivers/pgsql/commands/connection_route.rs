use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use libsql::Connection;
use tokio_postgres::Config;

use super::pool_connection::{create_pg_pool, full_error_chain};
use crate::common::enums::AppError;
use crate::drivers::pgsql::tls::PgSslMode;
use crate::ssh::host_keys::{self, HostKeyApprovals};
use crate::ssh::{self, SshConnection, SshTunnel, TunnelError};

#[cfg(test)]
#[path = "connection_route_tests.rs"]
mod tests;

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct DbTarget<'a> {
    pub user: &'a str,
    pub password: &'a str,
    pub database: &'a str,
    pub host: &'a str,
    pub port: u16,
}

pub(crate) struct SshTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub key_path: Option<String>,
}

impl SshTarget {
    /// `params` is `[host, port, user, password, key_path]`; without a host there is no tunnel.
    pub fn from_params(params: &[String]) -> Option<Self> {
        let host = params.first().filter(|host| !host.is_empty())?;
        let user = params.get(2)?;
        let optional = |index: usize| params.get(index).filter(|value| !value.is_empty()).cloned();
        Some(Self {
            host: host.clone(),
            port: params
                .get(1)
                .and_then(|port| port.parse().ok())
                .unwrap_or(22),
            user: user.clone(),
            password: optional(3),
            key_path: optional(4),
        })
    }
}

/// The settings to reach `db` with, through a new SSH tunnel when `ssh` is set.
pub(crate) async fn open_route(
    local_db: &Connection,
    approvals: &HostKeyApprovals,
    db: &DbTarget<'_>,
    ssh: Option<&SshTarget>,
) -> Result<(Config, Option<SshTunnel>), AppError> {
    let mut cfg = Config::new();
    cfg.user(db.user)
        .password(db.password)
        .dbname(db.database)
        .host(db.host);
    let Some(ssh) = ssh else {
        cfg.port(db.port);
        return Ok((cfg, None));
    };

    let expected = host_keys::known_fingerprint(local_db, &ssh.host, ssh.port).await?;
    let connection = SshConnection {
        host: &ssh.host,
        port: ssh.port,
        user: &ssh.user,
        password: ssh.password.as_deref(),
        key_path: ssh.key_path.as_deref(),
        expected_fingerprint: expected.clone(),
    };
    let tunnel = match ssh::start_tunnel(&connection, db.host, db.port).await {
        Ok(tunnel) => tunnel,
        Err(TunnelError::HostKey(fingerprint)) => {
            let challenge = approvals
                .challenge(&ssh.host, ssh.port, fingerprint, expected)
                .await;
            let json = serde_json::to_string(&challenge)
                .map_err(|error| AppError::SerializationError(error.to_string()))?;
            return Err(AppError::ConnectionFailed(format!("SSH_HOST_KEY:{json}")));
        }
        Err(error) => return Err(AppError::ConnectionFailed(error.to_string())),
    };
    // The socket goes to the tunnel, while TLS still verifies the database's own host name.
    cfg.hostaddr(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .port(tunnel.local_port);
    Ok((cfg, Some(tunnel)))
}

/// Connects over the route Connect would use, then closes the pool and the tunnel.
pub(crate) async fn test_connection(
    local_db: &Connection,
    approvals: &HostKeyApprovals,
    db: &DbTarget<'_>,
    ssl: PgSslMode,
    ssh: Option<&SshTarget>,
) -> Result<String, AppError> {
    let (cfg, tunnel) = open_route(local_db, approvals, db, ssh).await?;
    let version = tokio::time::timeout(TEST_TIMEOUT, server_version(&cfg, ssl))
        .await
        .unwrap_or_else(|_| {
            Err(AppError::ConnectionFailed(
                "Connection test timed out".into(),
            ))
        });
    if let Some(tunnel) = tunnel {
        tunnel.close().await;
    }
    version
}

async fn server_version(cfg: &Config, ssl: PgSslMode) -> Result<String, AppError> {
    let pool = create_pg_pool(cfg, ssl, 1)?;
    let client = pool
        .get()
        .await
        .map_err(|error| AppError::ConnectionFailed(full_error_chain(&error)))?;
    let row = client
        .query_one("SELECT version()", &[])
        .await
        .map_err(|error| AppError::ConnectionFailed(error.to_string()))?;
    Ok(row.get(0))
}
