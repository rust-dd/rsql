use super::test_server::{self, PASSWORD};
use super::*;
use std::sync::atomic::Ordering;

#[tokio::test]
#[ignore = "requires a local TCP listener"]
async fn unknown_and_changed_keys_are_rejected_before_password_or_key_authentication() {
    let server = test_server::start(None).await;
    let authentications = &server.authentications;

    for private_key in [false, true] {
        let mut connection = SshConnection {
            host: "127.0.0.1",
            port: server.port,
            user: "test",
            password: if private_key { None } else { Some(PASSWORD) },
            key_path: if private_key {
                server.key_path.to_str()
            } else {
                None
            },
            expected_fingerprint: None,
        };
        for expected in [None, Some("SHA256:previous-key".into())] {
            connection.expected_fingerprint = expected;
            let before = authentications.load(Ordering::SeqCst);
            let result = connect_ssh(&connection).await;
            assert!(
                matches!(result, Err(TunnelError::HostKey(ref key)) if key == &server.fingerprint)
            );
            assert_eq!(authentications.load(Ordering::SeqCst), before);
        }
        connection.expected_fingerprint = Some(server.fingerprint.clone());
        let before = authentications.load(Ordering::SeqCst);
        let handle = connect_ssh(&connection).await.unwrap();
        assert_eq!(authentications.load(Ordering::SeqCst), before + 1);
        handle
            .disconnect(russh::Disconnect::ByApplication, "test complete", "")
            .await
            .unwrap();
    }
}
