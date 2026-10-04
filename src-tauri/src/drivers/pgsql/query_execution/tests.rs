//! Query-result semantics against a disposable PostgreSQL database.

use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls};

use super::super::VirtualCache;
use super::super::wire::{Cell, ROW_SEP, pack_columns, pack_rows};
use super::{
    close_virtual, execute_query, execute_query_packed, execute_virtual, fetch_virtual_page,
};

async fn connect() -> Client {
    let url = std::env::var("RSQL_TEST_DATABASE_URL").expect("set RSQL_TEST_DATABASE_URL");
    let (client, connection) = tokio_postgres::connect(&url, NoTls).await.unwrap();
    tokio::spawn(async move {
        connection.await.expect("PostgreSQL connection failed");
    });
    client
}

async fn assert_result(client: &Client, sql: &str, columns: &[&str], rows: Vec<Vec<Cell>>) {
    let columns = columns
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let (actual_columns, actual_rows, _) = execute_query(client, sql).await.unwrap();
    assert_eq!(actual_columns, columns);
    assert_eq!(actual_rows, rows);

    let header = pack_columns(&columns);
    let body = pack_rows(&rows);
    let expected_packed = if rows.is_empty() {
        header.clone()
    } else {
        format!("{header}{ROW_SEP}{body}")
    };
    let (packed, _) = execute_query_packed(client, sql).await.unwrap();
    assert_eq!(packed, expected_packed);

    let cache = Mutex::new(VirtualCache::new());
    let (actual_header, count, page, _, capped) =
        execute_virtual(client, &cache, sql, "result", 2_000)
            .await
            .unwrap();
    assert_eq!(actual_header, header);
    assert_eq!(count, rows.len());
    assert_eq!(page, body);
    assert!(!capped);
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn empty_select_retains_columns() {
    let client = connect().await;
    assert_result(&client, "SELECT 1 AS value WHERE false", &["value"], vec![]).await;
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn a_single_empty_cell_is_a_row() {
    let client = connect().await;
    assert_result(
        &client,
        "SELECT '' AS value",
        &["value"],
        vec![vec![Some(String::new())]],
    )
    .await;
    let (packed, _) = execute_query_packed(&client, "SELECT '' AS value")
        .await
        .unwrap();
    assert_eq!(packed, "value\x1e\x1dE");
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn the_last_empty_rowset_replaces_an_earlier_rowset() {
    let client = connect().await;
    assert_result(
        &client,
        "SELECT 1 AS earlier; SELECT 2 AS final WHERE false",
        &["final"],
        vec![],
    )
    .await;
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn commands_after_a_rowset_preserve_its_metadata_and_execute_once() {
    let client = connect().await;
    client
        .batch_execute("CREATE TEMP TABLE query_writes (id int)")
        .await
        .unwrap();
    let sql = "SELECT 1 AS value WHERE false; INSERT INTO query_writes VALUES (1)";
    assert_result(&client, sql, &["value"], vec![]).await;
    let count = client
        .query_one("SELECT count(*) FROM query_writes", &[])
        .await
        .unwrap();
    assert_eq!(count.get::<_, i64>(0), 3);
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn commands_without_rowsets_report_affected_rows() {
    let client = connect().await;
    client
        .batch_execute("CREATE TEMP TABLE query_writes (id int)")
        .await
        .unwrap();
    let sql = "INSERT INTO query_writes VALUES (1), (2)";
    let (columns, rows, _) = execute_query(&client, sql).await.unwrap();
    assert_eq!(columns, ["Result"]);
    assert_eq!(rows, vec![vec![Some("2 rows affected".to_string())]]);

    let (packed, _) = execute_query_packed(&client, sql).await.unwrap();
    assert_eq!(packed, "Result\x1e2 rows affected");
    let cache = Mutex::new(VirtualCache::new());
    let (columns, count, packed, _, _) = execute_virtual(&client, &cache, sql, "writes", 2)
        .await
        .unwrap();
    assert!(columns.is_empty());
    assert_eq!(count, 0);
    assert_eq!(packed, "Result\x1e2 rows affected");
    assert!(cache.lock().await.is_empty());
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn virtual_pages_preserve_nulls_escapes_and_an_empty_final_cell() {
    let client = connect().await;
    let cache = Mutex::new(VirtualCache::new());
    let sql = "SELECT value FROM (VALUES
        (1, NULL::text), (2, 'null'), (3, ''),
        (4, 'árvíz 🦀' || chr(29) || chr(30) || chr(31)), (5, '')
        ) AS data(position, value) ORDER BY position";
    let expected = ["\x1dN\x1enull", "\x1dE\x1eárvíz 🦀\x1dC\x1dB\x1dA", "\x1dE"];
    let (columns, count, first, _, capped) = execute_virtual(&client, &cache, sql, "pages", 2)
        .await
        .unwrap();
    assert_eq!(columns, "value");
    assert_eq!(count, 5);
    assert_eq!(first, expected[0]);
    assert!(!capped);
    for (index, expected_page) in expected.iter().enumerate() {
        assert_eq!(
            fetch_virtual_page(&cache, "pages", 1, index * 2, 2)
                .await
                .unwrap(),
            *expected_page
        );
    }
    close_virtual(&cache, "pages").await.unwrap();
    assert!(cache.lock().await.is_empty());
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn an_error_does_not_publish_a_partial_result() {
    let client = connect().await;
    let cache = Mutex::new(VirtualCache::new());
    let sql = "SELECT 1 AS value; SELECT 1 / 0";
    assert!(execute_query(&client, sql).await.is_err());
    assert!(execute_query_packed(&client, sql).await.is_err());
    assert!(
        execute_virtual(&client, &cache, sql, "failed", 2)
            .await
            .is_err()
    );
    assert!(cache.lock().await.is_empty());
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn empty_cursor_fetch_retains_columns() {
    let client = connect().await;
    client
        .batch_execute("BEGIN; DECLARE result CURSOR FOR SELECT 1 AS value WHERE false")
        .await
        .unwrap();
    let messages = client.simple_query("FETCH 10 FROM result").await.unwrap();
    let (columns, rows) = super::helpers::process_simple_messages(messages);
    assert_eq!(columns, ["value"]);
    assert!(rows.is_empty());
    client.batch_execute("ROLLBACK").await.unwrap();
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn concurrent_results_share_the_budget_and_closing_releases_it() {
    let first = connect().await;
    let second = connect().await;
    let cache = Mutex::new(VirtualCache::with_budget(128));
    let sql = "SELECT repeat('x', 80) AS value FROM generate_series(1, 20)";
    let (a, b) = tokio::join!(
        execute_virtual(&first, &cache, sql, "a", 2),
        execute_virtual(&second, &cache, sql, "b", 2),
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert!(a.4 && b.4);
    assert_eq!(a.1 + b.1, 1);
    assert!(cache.lock().await.budget.used() <= 128);
    close_virtual(&cache, "a").await.unwrap();
    close_virtual(&cache, "b").await.unwrap();
    assert_eq!(cache.lock().await.budget.used(), 0);
}

#[tokio::test]
#[ignore = "requires RSQL_TEST_DATABASE_URL"]
async fn oversized_rows_and_query_errors_cannot_leave_reserved_memory() {
    let client = connect().await;
    let cache = Mutex::new(VirtualCache::with_budget(128));
    let result = execute_virtual(
        &client,
        &cache,
        "SELECT repeat(chr(29), 100) AS huge",
        "huge",
        2,
    )
    .await
    .unwrap();
    assert_eq!(result.0, "huge");
    assert_eq!(result.1, 0);
    assert!(result.4);
    assert_eq!(cache.lock().await.budget.used(), 0);
    assert!(
        execute_virtual(&client, &cache, "SELECT 'value'; SELECT 1/0", "error", 2)
            .await
            .is_err()
    );
    assert_eq!(cache.lock().await.budget.used(), 0);
}
