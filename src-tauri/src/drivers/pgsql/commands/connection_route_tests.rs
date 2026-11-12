use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use libsql::{Connection, Database};
use tokio_postgres::Config;
use tokio_postgres::config::Host;

use super::{DbTarget, SshTarget, open_route, test_connection};
use crate::drivers::pgsql::commands::pool_connection::create_pg_pool;
use crate::drivers::pgsql::tls::PgSslMode;
use crate::ssh::host_keys::{self, HostKeyApprovals};
use crate::ssh::test_server::{self, PASSWORD, TestSshServer};

const REMOTE_HOST: &str = "db.internal";

struct TestDatabase {
    user: String,
    password: String,
    database: String,
    address: SocketAddr,
}

async fn database(variable: &str) -> TestDatabase {
    let cfg: Config = std::env::var(variable)
        .unwrap_or_else(|_| panic!("set {variable}"))
        .parse()
        .unwrap();
    let Host::Tcp(host) = &cfg.get_hosts()[0] else {
        panic!("{variable} must use a TCP host");
    };
    let port = cfg.get_ports().first().copied().unwrap_or(5432);
    let address = tokio::net::lookup_host((host.as_str(), port))
        .await
        .unwrap()
        .min_by_key(|address| !address.is_ipv4())
        .unwrap();
    TestDatabase {
        user: cfg.get_user().unwrap().into(),
        password: String::from_utf8(cfg.get_password().unwrap_or_default().to_vec()).unwrap(),
        database: cfg.get_dbname().unwrap_or("postgres").into(),
        address,
    }
}

impl TestDatabase {
    fn target<'a>(&'a self, host: &'a str) -> DbTarget<'a> {
        DbTarget {
            user: &self.user,
            password: &self.password,
            database: &self.database,
            host,
            port: self.address.port(),
        }
    }
}

async fn local_db() -> (Database, Connection) {
    let db = libsql::Builder::new_local(":memory:")
        .build()
        .await
        .unwrap();
    let conn = db.connect().unwrap();
    host_keys::initialize(&conn).await.unwrap();
    (db, conn)
}

async fn trust(conn: &Connection, server: &TestSshServer, fingerprint: &str) {
    conn.execute(
        "INSERT INTO ssh_known_hosts (host, port, fingerprint) VALUES ('127.0.0.1', ?1, ?2)
         ON CONFLICT (host, port) DO UPDATE SET fingerprint = excluded.fingerprint",
        libsql::params![i64::from(server.port), fingerprint],
    )
    .await
    .unwrap();
}

fn ssh(server: &TestSshServer, key: bool) -> SshTarget {
    SshTarget {
        host: "127.0.0.1".into(),
        port: server.port,
        user: "test".into(),
        password: (!key).then(|| PASSWORD.into()),
        key_path: key.then(|| server.key_path.to_string_lossy().into()),
    }
}

fn challenge(error: &str) -> serde_json::Value {
    let json = error.split_once("SSH_HOST_KEY:").expect(error).1;
    serde_json::from_str(json).unwrap()
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn test_connection_uses_the_tunnel_after_the_host_key_is_trusted() {
    let pg = database("RSQL_TEST_DATABASE_URL").await;
    let server = test_server::start(Some(pg.address)).await;
    let (_db, conn) = local_db().await;
    let approvals = HostKeyApprovals::default();
    let db = pg.target(REMOTE_HOST);

    let direct = test_connection(&conn, &approvals, &db, PgSslMode::Disable, None).await;
    assert!(
        direct.is_err(),
        "{REMOTE_HOST} must only resolve through the tunnel"
    );

    for previous in [None, Some("SHA256:previous-key")] {
        if let Some(previous) = previous {
            trust(&conn, &server, previous).await;
        }
        let tunnel = ssh(&server, false);
        let error = test_connection(&conn, &approvals, &db, PgSslMode::Disable, Some(&tunnel))
            .await
            .unwrap_err()
            .to_string();
        let challenge = challenge(&error);
        assert_eq!(challenge["fingerprint"], server.fingerprint.as_str());
        assert_eq!(challenge["previousFingerprint"].as_str(), previous);
        approvals
            .approve(challenge["id"].as_str().unwrap(), &conn)
            .await
            .unwrap();
        let version = test_connection(&conn, &approvals, &db, PgSslMode::Disable, Some(&tunnel))
            .await
            .unwrap();
        assert!(version.starts_with("PostgreSQL"), "{version}");
    }

    let with_key = ssh(&server, true);
    let version = test_connection(&conn, &approvals, &db, PgSslMode::Disable, Some(&with_key))
        .await
        .unwrap();
    assert!(version.starts_with("PostgreSQL"), "{version}");
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn a_closed_route_leaves_no_listener_and_keeps_the_remote_name_for_tls() {
    let pg = database("RSQL_TEST_DATABASE_URL").await;
    let server = test_server::start(Some(pg.address)).await;
    let (_db, conn) = local_db().await;
    trust(&conn, &server, &server.fingerprint).await;
    let approvals = HostKeyApprovals::default();

    let tunnel = ssh(&server, false);
    let (cfg, route) = open_route(&conn, &approvals, &pg.target(REMOTE_HOST), Some(&tunnel))
        .await
        .unwrap();
    let route = route.unwrap();
    let port = route.local_port;
    assert_eq!(cfg.get_hosts(), [Host::Tcp(REMOTE_HOST.into())]);
    assert_eq!(cfg.get_hostaddrs(), [IpAddr::V4(Ipv4Addr::LOCALHOST)]);
    assert_eq!(cfg.get_ports(), [port]);

    let pool = create_pg_pool(&cfg, PgSslMode::Disable, 1).unwrap();
    let row = pool
        .get()
        .await
        .unwrap()
        .query_one("SELECT 1", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, i32>(0), 1);
    drop(pool);

    route.close().await;
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_TLS_DATABASE_URL"]
async fn tls_through_the_tunnel_is_encrypted_and_still_verified() {
    let pg = database("RSQL_TEST_TLS_DATABASE_URL").await;
    let server = test_server::start(Some(pg.address)).await;
    let (_db, conn) = local_db().await;
    trust(&conn, &server, &server.fingerprint).await;
    let approvals = HostKeyApprovals::default();
    let tunnel = ssh(&server, false);
    let (cfg, _route) = open_route(&conn, &approvals, &pg.target(REMOTE_HOST), Some(&tunnel))
        .await
        .unwrap();

    let pool = create_pg_pool(&cfg, PgSslMode::Require, 1).unwrap();
    let row = pool
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0));

    let verified = create_pg_pool(&cfg, PgSslMode::VerifyFull, 1).unwrap();
    assert!(
        verified.get().await.is_err(),
        "the test certificate is self-signed"
    );
}

// CI serves a certificate for db.internal from a CA it adds to the system trust store.
#[tokio::test]
#[ignore = "requires RSQL_TEST_TRUSTED_TLS_DATABASE_URL"]
async fn verify_full_through_the_tunnel_checks_the_database_host_name() {
    if std::env::var("RSQL_TEST_TRUSTED_TLS_DATABASE_URL").is_err() {
        eprintln!("skipped: RSQL_TEST_TRUSTED_TLS_DATABASE_URL is not set");
        return;
    }
    let pg = database("RSQL_TEST_TRUSTED_TLS_DATABASE_URL").await;
    let server = test_server::start(Some(pg.address)).await;
    let (_db, conn) = local_db().await;
    trust(&conn, &server, &server.fingerprint).await;
    let approvals = HostKeyApprovals::default();
    let tunnel = ssh(&server, false);

    for (host, mode, connects) in [
        (REMOTE_HOST, PgSslMode::VerifyFull, true),
        ("127.0.0.1", PgSslMode::VerifyFull, false),
        ("127.0.0.1", PgSslMode::VerifyCa, true),
    ] {
        let target = pg.target(host);
        let result = test_connection(&conn, &approvals, &target, mode, Some(&tunnel)).await;
        assert_eq!(result.is_ok(), connects, "{host} {mode:?}: {result:?}");
    }
}
