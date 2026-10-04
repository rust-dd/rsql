use super::*;
use russh::keys::ssh_key::rand_core::OsRng;
use russh::server::{self, Server as _};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
struct TestServer(Arc<AtomicUsize>);

impl server::Server for TestServer {
    type Handler = Self;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for TestServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, _: &str, _: &str) -> Result<server::Auth, Self::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(server::Auth::Accept)
    }

    async fn auth_publickey(
        &mut self,
        _: &str,
        _: &keys::PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(server::Auth::Accept)
    }
}

#[tokio::test]
#[ignore = "requires a local TCP listener"]
async fn unknown_and_changed_keys_are_rejected_before_password_or_key_authentication() {
    let host_key = keys::PrivateKey::random(&mut OsRng, keys::Algorithm::Ed25519).unwrap();
    let fingerprint = host_key
        .public_key()
        .fingerprint(keys::HashAlg::Sha256)
        .to_string();
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let authentications = Arc::new(AtomicUsize::new(0));
    let mut server = TestServer(Arc::clone(&authentications));
    let config = Arc::new(server::Config {
        keys: vec![host_key],
        auth_rejection_time: std::time::Duration::ZERO,
        ..Default::default()
    });
    let task = tokio::spawn(async move { server.run_on_socket(config, &socket).await });
    let user_key = keys::PrivateKey::random(&mut OsRng, keys::Algorithm::Ed25519).unwrap();
    let key_path =
        std::env::temp_dir().join(format!("rsql-ssh-test-{}-{port}", std::process::id()));
    user_key
        .write_openssh_file(&key_path, keys::ssh_key::LineEnding::LF)
        .unwrap();

    for private_key in [false, true] {
        let mut connection = SshConnection {
            host: "127.0.0.1",
            port,
            user: "test",
            password: if private_key {
                None
            } else {
                Some("test-password")
            },
            key_path: if private_key { key_path.to_str() } else { None },
            expected_fingerprint: None,
        };
        for expected in [None, Some("SHA256:previous-key".into())] {
            connection.expected_fingerprint = expected;
            let before = authentications.load(Ordering::SeqCst);
            let result = connect_ssh(&connection).await;
            assert!(matches!(result, Err(TunnelError::HostKey(ref key)) if key == &fingerprint));
            assert_eq!(authentications.load(Ordering::SeqCst), before);
        }
        connection.expected_fingerprint = Some(fingerprint.clone());
        let before = authentications.load(Ordering::SeqCst);
        let handle = connect_ssh(&connection).await.unwrap();
        assert_eq!(authentications.load(Ordering::SeqCst), before + 1);
        handle
            .disconnect(russh::Disconnect::ByApplication, "test complete", "")
            .await
            .unwrap();
    }
    std::fs::remove_file(key_path).unwrap();
    task.abort();
}
