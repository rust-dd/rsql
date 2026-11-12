use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use russh::Channel;
use russh::keys::{self, ssh_key::LineEnding};
use russh::server::{self, ChannelOpenHandle, Msg, Server as _, Session};
use tokio::net::{TcpListener, TcpStream};

pub(crate) const PASSWORD: &str = "test-password";

#[derive(Clone)]
struct Handler {
    authentications: Arc<AtomicUsize>,
    forward_to: Option<SocketAddr>,
}

impl server::Server for Handler {
    type Handler = Self;

    fn new_client(&mut self, _: Option<SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for Handler {
    type Error = russh::Error;

    async fn auth_password(&mut self, _: &str, _: &str) -> Result<server::Auth, Self::Error> {
        self.authentications.fetch_add(1, Ordering::SeqCst);
        Ok(server::Auth::Accept)
    }

    async fn auth_publickey(
        &mut self,
        _: &str,
        _: server::VerifiedPublicKey<'_>,
    ) -> Result<server::Auth, Self::Error> {
        self.authentications.fetch_add(1, Ordering::SeqCst);
        Ok(server::Auth::Accept)
    }

    // Every requested host goes to `forward_to`, so a name only the server knows still works.
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _: &str,
        _: u32,
        _: &str,
        _: u32,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(target) = self.forward_to else {
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            if let Ok(mut upstream) = TcpStream::connect(target).await {
                let mut stream = channel.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
            }
        });
        Ok(())
    }
}

/// An SSH server on 127.0.0.1 that accepts any credentials.
pub(crate) struct TestSshServer {
    pub port: u16,
    pub fingerprint: String,
    pub key_path: PathBuf,
    pub authentications: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestSshServer {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.key_path);
    }
}

pub(crate) async fn start(forward_to: Option<SocketAddr>) -> TestSshServer {
    let host_key = keys::PrivateKey::random(&mut rand::rng(), keys::Algorithm::Ed25519).unwrap();
    let fingerprint = host_key
        .public_key()
        .fingerprint(keys::HashAlg::Sha256)
        .to_string();
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let authentications = Arc::new(AtomicUsize::new(0));
    let mut handler = Handler {
        authentications: Arc::clone(&authentications),
        forward_to,
    };
    let config = Arc::new(server::Config {
        keys: vec![host_key],
        auth_rejection_time: std::time::Duration::ZERO,
        ..Default::default()
    });
    let task = tokio::spawn(async move {
        let _ = handler.run_on_socket(config, &socket).await;
    });

    let user_key = keys::PrivateKey::random(&mut rand::rng(), keys::Algorithm::Ed25519).unwrap();
    let key_path =
        std::env::temp_dir().join(format!("rsql-ssh-test-{}-{port}", std::process::id()));
    user_key
        .write_openssh_file(&key_path, LineEnding::LF)
        .unwrap();

    TestSshServer {
        port,
        fingerprint,
        key_path,
        authentications,
        task,
    }
}
