use deadpool_postgres::Pool;
use tokio::sync::watch;
use tokio_postgres::{Client, Config};

use super::super::pool_connection::create_pg_pool;
use super::super::query_session::run_on_client;
use crate::common::enums::{AppError, query_failed};

#[derive(Debug, PartialEq)]
struct Session {
    current_user: String,
    session_user: String,
    search_path: String,
    time_zone: String,
    tenant: String,
    temp_table: bool,
    advisory_locks: i64,
    prepared_statements: i64,
    cursors: i64,
    channels: i64,
}

const SNAPSHOT: &str = "SELECT pg_backend_pid(), current_user::text, session_user::text, \
    current_setting('search_path'), current_setting('TimeZone'), \
    coalesce(current_setting('rsql.tenant', true), ''), to_regclass('pg_temp.tab_a') IS NOT NULL, \
    (SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND pid = pg_backend_pid()), \
    (SELECT count(*) FROM pg_prepared_statements WHERE from_sql), \
    (SELECT count(*) FROM pg_cursors WHERE name <> ''), \
    (SELECT count(*) FROM pg_listening_channels())";

async fn snapshot(client: &Client) -> Result<(i32, Session), AppError> {
    let row = client
        .query_one(SNAPSHOT, &[])
        .await
        .map_err(query_failed)?;
    let session = Session {
        current_user: row.get(1),
        session_user: row.get(2),
        search_path: row.get(3),
        time_zone: row.get(4),
        tenant: row.get(5),
        temp_table: row.get(6),
        advisory_locks: row.get(7),
        prepared_statements: row.get(8),
        cursors: row.get(9),
        channels: row.get(10),
    };
    Ok((row.get(0), session))
}

fn config() -> Config {
    std::env::var("RSQL_TEST_DATABASE_URL")
        .expect("set RSQL_TEST_DATABASE_URL")
        .parse()
        .unwrap()
}

async fn run(pool: &Pool, sql: &str, timeout_ms: u32) -> Result<(i32, Session), AppError> {
    let (_sender, mut cancellation) = watch::channel(false);
    let client = pool.get().await.unwrap();
    run_on_client(
        client,
        timeout_ms,
        &mut cancellation,
        false,
        async |client| {
            client.batch_execute(sql).await.map_err(query_failed)?;
            snapshot(client).await
        },
    )
    .await
}

async fn observe(pool: &Pool) -> (i32, Session) {
    run(pool, "SELECT 1", 0).await.unwrap()
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn a_tab_cannot_inherit_another_tabs_session_state() {
    let pool = create_pg_pool(&config(), false, 1).unwrap();
    let (pid, baseline) = observe(&pool).await;

    let (tab_a_pid, tab_a) = run(
        &pool,
        "SET search_path = pg_catalog; SET TIME ZONE 'Asia/Tokyo'; \
         SELECT set_config('rsql.tenant', 'a', false); CREATE TEMP TABLE tab_a (n int); \
         SELECT pg_advisory_lock(42); PREPARE \"tab \"\"a\"\"\" AS SELECT 1; \
         DECLARE tab_a_cursor CURSOR WITH HOLD FOR SELECT 1; LISTEN tab_a; \
         SET SESSION AUTHORIZATION pg_read_all_data",
        0,
    )
    .await
    .unwrap();
    assert_eq!(tab_a_pid, pid);
    let changed = Session {
        current_user: "pg_read_all_data".into(),
        session_user: "pg_read_all_data".into(),
        search_path: "pg_catalog".into(),
        time_zone: "Asia/Tokyo".into(),
        tenant: "a".into(),
        temp_table: true,
        advisory_locks: 1,
        prepared_statements: 1,
        cursors: 1,
        channels: 1,
    };
    assert_eq!(tab_a, changed);

    let (tab_b_pid, tab_b) = observe(&pool).await;
    assert_eq!(tab_b_pid, pid);
    assert_eq!(tab_b, baseline);
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn failed_timed_out_and_unfinished_scripts_return_a_clean_session() {
    let pool = create_pg_pool(&config(), false, 1).unwrap();
    let (pid, baseline) = observe(&pool).await;

    for (sql, timeout_ms) in [
        (
            "BEGIN; SET search_path = pg_catalog; COMMIT; SELECT pg_advisory_lock(42); SELECT 1/0",
            0,
        ),
        (
            "BEGIN; SET ROLE pg_read_all_data; COMMIT; \
             BEGIN; CREATE TEMP TABLE tab_a (n int); SELECT pg_advisory_lock(42)",
            0,
        ),
        ("SELECT pg_advisory_lock(42); SELECT pg_sleep(5)", 25),
    ] {
        let result = run(&pool, sql, timeout_ms).await;
        assert_eq!(result.is_err(), sql.contains("1/0") || timeout_ms > 0);
        let (next_pid, next) = observe(&pool).await;
        assert_eq!(next_pid, pid, "{sql}");
        assert_eq!(next, baseline, "{sql}");
    }
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn the_reset_keeps_role_defaults_configured_on_the_server() {
    const ROLE: &str = "rsql_session_defaults";
    let admin = create_pg_pool(&config(), false, 1).unwrap();
    let admin = admin.get().await.unwrap();
    admin
        .batch_execute(&format!(
            "DROP ROLE IF EXISTS {ROLE}; CREATE ROLE {ROLE} LOGIN PASSWORD 'rsql'; \
             GRANT pg_read_all_data TO {ROLE}; \
             ALTER ROLE {ROLE} SET search_path = pg_catalog, public; \
             ALTER ROLE {ROLE} SET role = pg_read_all_data"
        ))
        .await
        .unwrap();

    let mut config = config();
    config.user(ROLE).password("rsql");
    let pool = create_pg_pool(&config, false, 1).unwrap();
    let (_, baseline) = observe(&pool).await;
    assert_eq!(baseline.current_user, "pg_read_all_data");
    assert_eq!(baseline.search_path, "pg_catalog, public");

    let (_, changed) = run(&pool, "SET ROLE NONE; SET search_path = public", 0)
        .await
        .unwrap();
    assert_eq!(changed.current_user, ROLE);
    assert_eq!(changed.search_path, "public");
    let (_, after) = observe(&pool).await;
    assert_eq!(after, baseline);

    pool.close();
    admin
        .batch_execute(&format!("DROP ROLE {ROLE}"))
        .await
        .unwrap();
}
