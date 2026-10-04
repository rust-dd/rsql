use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::watch;
use tokio_postgres::Config;
use tracing_subscriber::util::SubscriberInitExt;

use super::super::pool_connection::create_pg_pool;
use super::run_on_client;
use crate::common::enums::query_failed;

#[derive(Clone, Default)]
struct QueryLog(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for QueryLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for QueryLog {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl QueryLog {
    fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
        tracing_subscriber::fmt()
            .with_writer(self.clone())
            .with_ansi(false)
            .without_time()
            .finish()
    }

    fn assert_no_cleanup_warnings(&self) -> String {
        let output = String::from_utf8(self.0.lock().unwrap().clone()).unwrap();
        assert!(
            !output.contains("there is no transaction in progress"),
            "{output}"
        );
        assert!(!output.contains("SET LOCAL can only be used"), "{output}");
        output
    }
}

fn pool() -> deadpool_postgres::Pool {
    let config = std::env::var("RSQL_TEST_DATABASE_URL")
        .expect("set RSQL_TEST_DATABASE_URL")
        .parse::<Config>()
        .unwrap();
    create_pg_pool(&config, false, 2).unwrap()
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn cleanup_is_quiet_and_preserves_user_warnings_and_session_settings() {
    let log = QueryLog::default();
    let _subscriber = log.subscriber().set_default();
    let pool = pool();
    let client = pool.get().await.unwrap();
    client
        .batch_execute("SET client_min_messages = warning; CREATE TEMP TABLE cleanup_probe (n int)")
        .await
        .unwrap();
    let original_pid = client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get::<_, i32>(0);
    drop(client);

    let (_sender, mut cancellation) = watch::channel(false);
    for sql in [
        "DO $$ BEGIN RAISE WARNING 'rsql user warning'; END $$; SELECT 1",
        "BEGIN; INSERT INTO cleanup_probe VALUES (1)",
        "BEGIN; INSERT INTO cleanup_probe VALUES (1); SELECT 1/0",
        "SELECT 1/0",
        "BEGIN; SELECT 1; COMMIT",
    ] {
        let result = run_on_client(
            pool.get().await.unwrap(),
            1000,
            &mut cancellation,
            false,
            async |client| client.batch_execute(sql).await.map_err(query_failed),
        )
        .await;
        assert_eq!(result.is_err(), sql.contains("1/0"));
        let client = pool.get().await.unwrap();
        let row = client
            .query_one(
                "SELECT pg_backend_pid(), current_setting('client_min_messages'), \
                 current_setting('statement_timeout'), (SELECT count(*) FROM cleanup_probe)",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, i32>(0), original_pid);
        assert_eq!(row.get::<_, String>(1), "warning");
        assert_eq!(row.get::<_, String>(2), "0");
        assert_eq!(row.get::<_, i64>(3), 0);
    }
    let output = log.assert_no_cleanup_warnings();
    assert!(output.contains("rsql user warning"), "{output}");
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn pool_recycling_cleans_unfinished_transactions_without_idle_warnings() {
    let log = QueryLog::default();
    let _subscriber = log.subscriber().set_default();
    let pool = pool();
    let client = pool.get().await.unwrap();
    client
        .batch_execute("SET client_min_messages = warning; CREATE TEMP TABLE recycle_probe (n int)")
        .await
        .unwrap();
    drop(client);

    for sql in [
        "SELECT 1",
        "BEGIN; INSERT INTO recycle_probe VALUES (1)",
        "BEGIN; INSERT INTO recycle_probe VALUES (1); SELECT 1/0",
    ] {
        let client = pool.get().await.unwrap();
        let result = client.batch_execute(sql).await;
        assert_eq!(result.is_err(), sql.contains("1/0"));
        drop(client);
        let client = pool.get().await.unwrap();
        let row = client
            .query_one(
                "SELECT count(*), current_setting('client_min_messages') FROM recycle_probe",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, i64>(0), 0);
        assert_eq!(row.get::<_, String>(1), "warning");
    }
    log.assert_no_cleanup_warnings();
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn timeouts_and_aborted_transactions_do_not_leak_to_the_next_query() {
    let pool = pool();
    let (_sender, mut cancellation) = watch::channel(false);
    let result = run_on_client(
        pool.get().await.unwrap(),
        25,
        &mut cancellation,
        false,
        async |client| {
            client
                .batch_execute("BEGIN; SELECT pg_sleep(5)")
                .await
                .map_err(query_failed)
        },
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("statement timeout")
    );
    let value = run_on_client(
        pool.get().await.unwrap(),
        0,
        &mut cancellation,
        false,
        async |client| {
            let row = client
                .query_one("SELECT current_setting('statement_timeout')", &[])
                .await
                .map_err(query_failed)?;
            Ok(row.get::<_, String>(0))
        },
    )
    .await
    .unwrap();
    assert_eq!(value, "0");
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn an_invalid_session_cannot_run_sql_without_its_timeout() {
    let pool = pool();
    let client = pool.get().await.unwrap();
    client.batch_execute("BEGIN; SELECT 1/0").await.unwrap_err();
    let (_sender, mut cancellation) = watch::channel(false);
    let ran = Arc::new(AtomicBool::new(false));
    let result = run_on_client(client, 25, &mut cancellation, false, async |_client| {
        ran.store(true, Ordering::SeqCst);
        Ok(())
    })
    .await;
    assert!(result.is_err());
    assert!(!ran.load(Ordering::SeqCst));
    let row = pool
        .get()
        .await
        .unwrap()
        .query_one("SELECT 1", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, i32>(0), 1);
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn cancelling_one_query_preserves_other_queries_and_discards_its_connection() {
    let pool = pool();
    let client = pool.get().await.unwrap();
    let old_pid = client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get::<_, i32>(0);
    let (sender, mut cancellation) = watch::channel(false);
    let (started, ready) = tokio::sync::oneshot::channel();
    let query = run_on_client(client, 0, &mut cancellation, false, async |client| {
        let _ = started.send(());
        client
            .batch_execute("SELECT pg_sleep(5)")
            .await
            .map_err(query_failed)
    });
    let cancel = async {
        ready.await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let other = pool.get().await.unwrap();
        assert_eq!(
            other
                .query_one("SELECT 2", &[])
                .await
                .unwrap()
                .get::<_, i32>(0),
            2
        );
        sender.send_replace(true);
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(query, cancel)
    })
    .await
    .unwrap();
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    let next_pid = pool
        .get()
        .await
        .unwrap()
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get::<_, i32>(0);
    assert_ne!(old_pid, next_pid);
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn a_failed_reset_discards_the_connection() {
    let pool = pool();
    let (_sender, mut cancellation) = watch::channel(false);
    let result = run_on_client(
        pool.get().await.unwrap(),
        25,
        &mut cancellation,
        false,
        async |client| {
            client
                .batch_execute("SELECT pg_terminate_backend(pg_backend_pid())")
                .await
                .map_err(query_failed)
        },
    )
    .await;
    assert!(result.is_err());
    let row = pool
        .get()
        .await
        .unwrap()
        .query_one("SELECT 1", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, i32>(0), 1);
}
