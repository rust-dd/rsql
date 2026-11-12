use libsql::{Connection, Database};

use super::{
    HistoryFilter, NewHistoryEntry, RETENTION, Retention, clear, databases, is_enabled, record,
    search, set_enabled, status,
};
use crate::dbs::schema::create_local_schema;

const NOW: i64 = 1_800_000_000_000;

async fn open(path: &str) -> (Database, Connection) {
    let db = libsql::Builder::new_local(path).build().await.unwrap();
    let conn = db.connect().unwrap();
    create_local_schema(&conn).await.unwrap();
    (db, conn)
}

fn entry(project_id: &str, database: &str, sql: &str, executed_at: i64) -> NewHistoryEntry {
    NewHistoryEntry {
        project_id: project_id.into(),
        database: database.into(),
        sql: sql.into(),
        executed_at,
        duration_ms: 12.5,
        row_count: 3,
        success: true,
        error: None,
    }
}

async fn sqls(conn: &Connection, filter: HistoryFilter) -> Vec<String> {
    let page = search(conn, &filter).await.unwrap();
    page.entries.into_iter().map(|entry| entry.sql).collect()
}

#[tokio::test]
async fn search_filters_by_text_connection_database_status_and_time() {
    let (_db, conn) = open(":memory:").await;
    let failed = NewHistoryEntry {
        success: false,
        error: Some("permission denied".into()),
        ..entry(
            "p1",
            "orders",
            "DELETE FROM orders WHERE id = 1",
            NOW - 2_000,
        )
    };
    for entry in [
        entry("p1", "orders", "SELECT * FROM orders", NOW - 3_000),
        failed,
        entry("p2", "billing", "SELECT 100% AS all_rows", NOW - 1_000),
    ] {
        assert!(record(&conn, &entry, RETENTION, NOW).await.unwrap());
    }

    let all = sqls(&conn, HistoryFilter::default()).await;
    assert_eq!(all[0], "SELECT 100% AS all_rows");
    assert_eq!(all.len(), 3);
    let text = |text: &str| HistoryFilter {
        text: Some(text.into()),
        ..HistoryFilter::default()
    };
    assert_eq!(sqls(&conn, text(" ORDERS ")).await.len(), 2);
    assert_eq!(sqls(&conn, text("100%")).await, ["SELECT 100% AS all_rows"]);
    assert_eq!(sqls(&conn, text("l_r")).await, ["SELECT 100% AS all_rows"]);
    assert!(sqls(&conn, text("0%A")).await.is_empty());

    let by_project = HistoryFilter {
        project_id: Some("p1".into()),
        ..HistoryFilter::default()
    };
    assert_eq!(sqls(&conn, by_project).await.len(), 2);
    let by_database = HistoryFilter {
        database: Some("billing".into()),
        ..HistoryFilter::default()
    };
    assert_eq!(sqls(&conn, by_database).await.len(), 1);
    let failures = HistoryFilter {
        success: Some(false),
        ..HistoryFilter::default()
    };
    let page = search(&conn, &failures).await.unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].error.as_deref(), Some("permission denied"));
    let recent = HistoryFilter {
        since: Some(NOW - 2_000),
        ..HistoryFilter::default()
    };
    assert_eq!(sqls(&conn, recent).await.len(), 2);

    assert_eq!(databases(&conn, None).await.unwrap(), ["billing", "orders"]);
    assert_eq!(databases(&conn, Some("p2")).await.unwrap(), ["billing"]);
}

#[tokio::test]
async fn pages_continue_before_the_last_entry() {
    let (_db, conn) = open(":memory:").await;
    for n in 0..5 {
        let sql = format!("SELECT {n}");
        record(&conn, &entry("p1", "db", &sql, NOW), RETENTION, NOW)
            .await
            .unwrap();
    }
    let first = search(
        &conn,
        &HistoryFilter {
            limit: Some(3),
            ..HistoryFilter::default()
        },
    )
    .await
    .unwrap();
    assert!(first.has_more);
    let rest = HistoryFilter {
        limit: Some(3),
        before_id: first.entries.last().map(|entry| entry.id),
        ..HistoryFilter::default()
    };
    assert_eq!(sqls(&conn, rest).await, ["SELECT 1", "SELECT 0"]);
}

#[tokio::test]
async fn retention_drops_expired_excess_and_over_budget_history() {
    let (_db, conn) = open(":memory:").await;
    let retention = Retention {
        max_entries: 3,
        max_age_days: 1,
        max_bytes: 1_000,
    };
    let day = 24 * 60 * 60 * 1000;
    record(
        &conn,
        &entry("p1", "db", "expired", NOW - 2 * day),
        retention,
        NOW,
    )
    .await
    .unwrap();
    assert!(sqls(&conn, HistoryFilter::default()).await.is_empty());

    for n in 0..5 {
        let sql = format!("SELECT {n}");
        record(&conn, &entry("p1", "db", &sql, NOW), retention, NOW)
            .await
            .unwrap();
    }
    let kept = ["SELECT 4", "SELECT 3", "SELECT 2"];
    assert_eq!(sqls(&conn, HistoryFilter::default()).await, kept);

    for n in 0..3 {
        let sql = format!("{n}{}", "x".repeat(399));
        record(&conn, &entry("p1", "db", &sql, NOW), retention, NOW)
            .await
            .unwrap();
    }
    let kept = sqls(&conn, HistoryFilter::default()).await;
    assert_eq!(kept.len(), 2);
    assert!(kept[0].starts_with('2') && kept[1].starts_with('1'));
    assert!(status(&conn, retention).await.unwrap().bytes <= 1_000);
}

#[tokio::test]
async fn oversized_sql_is_truncated_on_a_character_boundary() {
    let (_db, conn) = open(":memory:").await;
    let sql = format!("SELECT '{}'", "é".repeat(40 * 1024));
    record(&conn, &entry("p1", "db", &sql, NOW), RETENTION, NOW)
        .await
        .unwrap();
    let page = search(&conn, &HistoryFilter::default()).await.unwrap();
    let stored = &page.entries[0];
    assert!(stored.sql_truncated);
    assert!(stored.sql.len() <= 64 * 1024);
    assert!(sql.starts_with(&stored.sql));
}

#[tokio::test]
async fn disabling_stops_recording_and_both_survive_a_restart() {
    let path = std::env::temp_dir().join(format!("rsql-history-{}.db", uuid::Uuid::new_v4()));
    let path = path.to_string_lossy().to_string();
    {
        let (_db, conn) = open(&path).await;
        assert!(is_enabled(&conn).await.unwrap());
        record(
            &conn,
            &entry("p1", "db", "SELECT 'kept'", NOW),
            RETENTION,
            NOW,
        )
        .await
        .unwrap();
        set_enabled(&conn, false).await.unwrap();
        let skipped = entry("p1", "db", "SELECT 'skipped'", NOW);
        assert!(!record(&conn, &skipped, RETENTION, NOW).await.unwrap());
    }

    let (_db, conn) = open(&path).await;
    assert!(!is_enabled(&conn).await.unwrap());
    assert_eq!(
        sqls(&conn, HistoryFilter::default()).await,
        ["SELECT 'kept'"]
    );
    set_enabled(&conn, true).await.unwrap();
    let resumed = entry("p1", "db", "SELECT 'resumed'", NOW);
    assert!(record(&conn, &resumed, RETENTION, NOW).await.unwrap());
    drop(conn);
    for suffix in ["", "-journal", "-wal", "-shm"] {
        std::fs::remove_file(format!("{path}{suffix}")).ok();
    }
}

#[tokio::test]
async fn clearing_history_keeps_other_connections_and_local_data() {
    let (_db, conn) = open(":memory:").await;
    conn.execute_batch(
        "INSERT INTO projects (id) VALUES ('p1');
         INSERT INTO queries (id, sql) VALUES ('q1', 'SELECT 1');
         INSERT INTO workspaces (name, tabs) VALUES ('w1', '[]');",
    )
    .await
    .unwrap();
    for project_id in ["p1", "p2"] {
        record(
            &conn,
            &entry(project_id, "db", project_id, NOW),
            RETENTION,
            NOW,
        )
        .await
        .unwrap();
    }
    create_local_schema(&conn).await.unwrap();

    clear(&conn, Some("p1")).await.unwrap();
    assert_eq!(sqls(&conn, HistoryFilter::default()).await, ["p2"]);
    clear(&conn, None).await.unwrap();
    let status = status(&conn, RETENTION).await.unwrap();
    assert_eq!((status.entries, status.bytes), (0, 0));

    let mut rows = conn
        .query(
            "SELECT (SELECT count(*) FROM projects) + (SELECT count(*) FROM queries) \
             + (SELECT count(*) FROM workspaces)",
            (),
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    assert_eq!(row.get::<i64>(0).unwrap(), 3);
}

#[tokio::test]
async fn cleared_sql_does_not_remain_in_the_database_file() {
    let path = std::env::temp_dir().join(format!("rsql-history-{}.db", uuid::Uuid::new_v4()));
    let path = path.to_string_lossy().to_string();
    let needle = "SELECT 'rsql-secret-needle'";
    let (db, conn) = open(&path).await;
    record(&conn, &entry("p1", "db", needle, NOW), RETENTION, NOW)
        .await
        .unwrap();
    clear(&conn, None).await.unwrap();
    drop((conn, db));

    for suffix in ["", "-journal", "-wal"] {
        let file = format!("{path}{suffix}");
        if let Ok(bytes) = std::fs::read(&file) {
            let found = bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
            assert!(!found, "{file} still contains the cleared SQL");
        }
        std::fs::remove_file(&file).ok();
    }
}
