use std::sync::{Arc, Mutex};

pub(crate) mod host_keys;
#[cfg(test)]
mod tests;
use tokio::net::TcpListener;
use tokio::sync::watch;

use russh::client;
use russh::keys::{self, PrivateKeyWithHashAlg};

pub struct SshTunnel {
    pub local_port: u16,
    shutdown_tx: watch::Sender<bool>,
}

impl SshTunnel {
    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

struct Client {
    expected_fingerprint: Option<String>,
    rejected_key: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TunnelError {
    #[error("Untrusted SSH host key: {0}")]
    HostKey(String),
    #[error("{0}")]
    Connection(String),
}

impl From<String> for TunnelError {
    fn from(message: String) -> Self {
        Self::Connection(message)
    }
}

pub(crate) struct SshConnection<'a> {
    pub host: &'a str,
    pub port: u16,
    pub user: &'a str,
    pub password: Option<&'a str>,
    pub key_path: Option<&'a str>,
    pub expected_fingerprint: Option<String>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key
            .fingerprint(keys::HashAlg::Sha256)
            .to_string();
        if self.expected_fingerprint.as_deref() == Some(&fingerprint) {
            Ok(true)
        } else {
            *self.rejected_key.lock().expect("host key lock poisoned") = Some(fingerprint);
            Ok(false)
        }
    }
}

async fn connect_ssh(
    connection: &SshConnection<'_>,
) -> Result<client::Handle<Client>, TunnelError> {
    let config = Arc::new(client::Config::default());
    let rejected_key = Arc::new(Mutex::new(None));
    let handler = Client {
        expected_fingerprint: connection.expected_fingerprint.clone(),
        rejected_key: Arc::clone(&rejected_key),
    };
    let mut handle = client::connect(config, (connection.host, connection.port), handler)
        .await
        .map_err(|error| {
            if let Some(fingerprint) = rejected_key.lock().expect("host key lock poisoned").take() {
                TunnelError::HostKey(fingerprint)
            } else {
                TunnelError::Connection(format!(
                    "SSH connection to {}:{} failed: {}",
                    connection.host, connection.port, error
                ))
            }
        })?;
    let ssh_user = connection.user;
    let ssh_password = connection.password;
    let ssh_key_path = connection.key_path;

    if let Some(key_path) = ssh_key_path
        && !key_path.is_empty()
    {
        match keys::load_secret_key(key_path, ssh_password) {
            Ok(key) => {
                let key = PrivateKeyWithHashAlg::new(Arc::new(key), None);
                let result = handle
                    .authenticate_publickey(ssh_user, key)
                    .await
                    .map_err(|e| format!("SSH key auth failed: {}", e))?;
                if result.success() {
                    return Ok(handle);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load SSH key {}: {}", key_path, e);
            }
        }
    }

    if let Some(password) = ssh_password
        && !password.is_empty()
    {
        let result = handle
            .authenticate_password(ssh_user, password)
            .await
            .map_err(|e| format!("SSH password auth failed: {}", e))?;
        if result.success() {
            return Ok(handle);
        }
    }

    Err("SSH authentication failed: all methods exhausted"
        .to_string()
        .into())
}

pub(crate) async fn start_tunnel(
    connection: &SshConnection<'_>,
    remote_host: &str,
    remote_port: u16,
) -> Result<SshTunnel, TunnelError> {
    let handle = tokio::time::timeout(std::time::Duration::from_secs(30), connect_ssh(connection))
        .await
        .map_err(|_| TunnelError::Connection("SSH connection timed out".into()))??;
    let handle = Arc::new(handle);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Failed to bind local port: {}", e))?;
    let local_port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get local addr: {}", e))?
        .port();

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let remote_host = remote_host.to_string();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((local_stream, _)) => {
                            let handle = handle.clone();
                            let rh = remote_host.clone();
                            tokio::spawn(async move {
                                if let Err(e) = proxy(local_stream, &handle, &rh, remote_port).await {
                                    tracing::error!("SSH proxy error: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("SSH tunnel accept error: {}", e);
                            break;
                        }
                    }
                }
                _ = shutdown_rx.changed() => break,
            }
        }
    });

    Ok(SshTunnel {
        local_port,
        shutdown_tx,
    })
}

async fn proxy(
    mut local: tokio::net::TcpStream,
    handle: &client::Handle<Client>,
    remote_host: &str,
    remote_port: u16,
) -> Result<(), String> {
    let channel = handle
        .channel_open_direct_tcpip(remote_host, remote_port as u32, "127.0.0.1", 0)
        .await
        .map_err(|e| {
            format!(
                "SSH direct-tcpip to {}:{} failed: {}",
                remote_host, remote_port, e
            )
        })?;

    let mut stream = channel.into_stream();
    tokio::io::copy_bidirectional(&mut local, &mut stream)
        .await
        .map_err(|e| format!("SSH proxy IO error: {}", e))?;

    Ok(())
}
