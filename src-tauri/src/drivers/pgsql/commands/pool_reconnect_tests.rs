use std::time::Duration;

use deadpool_postgres::{PoolError, TimeoutType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_postgres::Config;

use super::create_pg_pool;
use crate::drivers::pgsql::tls::PgSslMode;

async fn frame(socket: &mut TcpStream) -> Vec<u8> {
    let size = socket.read_u32().await.unwrap();
    let mut bytes = vec![0; (size - 4) as usize];
    socket.read_exact(&mut bytes).await.unwrap();
    bytes
}

async fn authenticate(socket: &mut TcpStream) {
    frame(socket).await;
    socket
        .write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I")
        .await
        .unwrap();
}

fn config(listener: &TcpListener) -> Config {
    let mut config = Config::new();
    config
        .host("127.0.0.1")
        .port(listener.local_addr().unwrap().port())
        .user("test");
    config
}

#[tokio::test]
async fn an_unresponsive_server_cannot_stall_connection_creation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pool = create_pg_pool(&config(&listener), PgSslMode::Disable, 1).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        frame(&mut socket).await;
        std::future::pending::<()>().await;
    });

    let result = tokio::time::timeout(Duration::from_secs(12), pool.get()).await;
    server.abort();
    assert!(
        matches!(result, Ok(Err(PoolError::Timeout(TimeoutType::Create)))),
        "connection creation must return a timeout, got {result:?}"
    );
}

#[tokio::test]
async fn a_stalled_idle_connection_is_replaced_before_running_the_next_query() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pool = create_pg_pool(&config(&listener), PgSslMode::Disable, 1).unwrap();
    let server = tokio::spawn(async move {
        let (mut stale, _) = listener.accept().await.unwrap();
        authenticate(&mut stale).await;
        assert_eq!(stale.read_u8().await.unwrap(), b'Q');
        let reset = String::from_utf8(frame(&mut stale).await).unwrap();
        assert!(reset.contains("DISCARD TEMP"), "{reset}");

        let (mut replacement, _) = listener.accept().await.unwrap();
        authenticate(&mut replacement).await;
        assert_eq!(replacement.read_u8().await.unwrap(), b'Q');
        assert_eq!(frame(&mut replacement).await, b"SELECT 1\0");
        replacement
            .write_all(b"C\0\0\0\x0dSELECT 1\0Z\0\0\0\x05I")
            .await
            .unwrap();
    });

    drop(pool.get().await.unwrap());
    let result = tokio::time::timeout(Duration::from_secs(12), async {
        let client = pool.get().await.unwrap();
        client.batch_execute("SELECT 1").await.unwrap();
    })
    .await;
    if result.is_err() {
        server.abort();
    }
    assert!(result.is_ok(), "the old connection prevented reconnection");
    server.await.unwrap();
}
