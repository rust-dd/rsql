use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::watch;
use tokio_postgres::Config;

use super::super::pool_connection::create_pg_pool;
use super::run_on_client;
use crate::common::enums::query_failed;

fn pool() -> deadpool_postgres::Pool {
    let config = std::env::var("RSQL_TEST_DATABASE_URL")
        .expect("set RSQL_TEST_DATABASE_URL")
        .parse::<Config>()
        .unwrap();
    create_pg_pool(&config, false, 2).unwrap()
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
