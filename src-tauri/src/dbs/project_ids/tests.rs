use super::migrate_project_ids;
use crate::dbs::schema::create_local_schema;
use libsql::{Connection, Database};
use serde_json::Value;
use uuid::Uuid;

async fn local_db() -> (Database, Connection) {
    let db = libsql::Builder::new_local(":memory:")
        .build()
        .await
        .unwrap();
    let conn = db.connect().unwrap();
    create_local_schema(&conn).await.unwrap();
    (db, conn)
}

async fn single_string(conn: &Connection, sql: &str, param: &str) -> String {
    let mut rows = conn.query(sql, [param]).await.unwrap();
    let row = rows.next().await.unwrap().expect("row exists");
    row.get::<String>(0).unwrap()
}

async fn id_for_legacy(conn: &Connection, legacy_id: &str) -> String {
    single_string(
        conn,
        "SELECT id FROM projects WHERE legacy_id = ?1",
        legacy_id,
    )
    .await
}

async fn query_ids(conn: &Connection) -> Vec<String> {
    let mut rows = conn
        .query("SELECT id FROM queries ORDER BY id", ())
        .await
        .unwrap();
    let mut ids = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        ids.push(row.get::<String>(0).unwrap());
    }
    ids
}

#[tokio::test]
async fn legacy_project_gets_uuid_key_and_keeps_its_name() {
    let (_db, conn) = local_db().await;
    conn.execute("INSERT INTO projects (id) VALUES ('prod')", ())
        .await
        .unwrap();

    assert_eq!(migrate_project_ids(&conn, None).await.unwrap(), 1);

    let id = id_for_legacy(&conn, "prod").await;
    assert!(Uuid::parse_str(&id).is_ok(), "{id} is not a UUID");
    let name = single_string(&conn, "SELECT name FROM projects WHERE id = ?1", &id).await;
    assert_eq!(name, "prod");
}

#[tokio::test]
async fn saved_queries_follow_their_project_even_when_its_name_has_colons() {
    let (_db, conn) = local_db().await;
    conn.execute_batch(
        "INSERT INTO projects (id) VALUES ('prod'), ('prod:eu');
         INSERT INTO queries (id, sql) VALUES
           ('prod:app:PGSQL:daily', 'SELECT 1'),
           ('prod:eu:app:PGSQL:a:b', 'SELECT 2'),
           ('gone:app:PGSQL:orphan', 'SELECT 3');",
    )
    .await
    .unwrap();

    migrate_project_ids(&conn, None).await.unwrap();

    let prod = id_for_legacy(&conn, "prod").await;
    let eu = id_for_legacy(&conn, "prod:eu").await;
    let mut expected = vec![
        format!("{prod}:app:PGSQL:daily"),
        format!("{eu}:app:PGSQL:a:b"),
        "gone:app:PGSQL:orphan".to_string(),
    ];
    expected.sort();
    assert_eq!(query_ids(&conn).await, expected);
    let sql = single_string(
        &conn,
        "SELECT sql FROM queries WHERE id = ?1",
        &format!("{eu}:app:PGSQL:a:b"),
    )
    .await;
    assert_eq!(sql, "SELECT 2");
}

#[tokio::test]
async fn workspace_tabs_point_at_the_new_project_key() {
    let (_db, conn) = local_db().await;
    conn.execute_batch(
        r#"INSERT INTO projects (id) VALUES ('prod');
           INSERT INTO workspaces (name, tabs) VALUES ('ws', '[
             {"title":"Q1","editorValue":"SELECT 1","projectId":"prod","type":"query"},
             {"title":"Q2","editorValue":"","type":"query"},
             {"title":"Q3","editorValue":"","projectId":"gone","type":"query"}
           ]');"#,
    )
    .await
    .unwrap();

    migrate_project_ids(&conn, None).await.unwrap();

    let prod = id_for_legacy(&conn, "prod").await;
    let tabs: Value = serde_json::from_str(
        &single_string(&conn, "SELECT tabs FROM workspaces WHERE name = ?1", "ws").await,
    )
    .unwrap();
    assert_eq!(tabs[0]["projectId"], prod.as_str());
    assert_eq!(tabs[0]["editorValue"], "SELECT 1");
    assert!(tabs[1].get("projectId").is_none());
    assert_eq!(tabs[2]["projectId"], "gone");
}

#[tokio::test]
async fn second_run_changes_nothing() {
    let (_db, conn) = local_db().await;
    conn.execute("INSERT INTO projects (id) VALUES ('prod')", ())
        .await
        .unwrap();
    migrate_project_ids(&conn, None).await.unwrap();
    let first = id_for_legacy(&conn, "prod").await;

    assert_eq!(migrate_project_ids(&conn, None).await.unwrap(), 0);
    assert_eq!(id_for_legacy(&conn, "prod").await, first);
}

#[tokio::test]
async fn uuid_keyed_row_without_name_keeps_its_key() {
    let (_db, conn) = local_db().await;
    let key = Uuid::new_v4().to_string();
    conn.execute("INSERT INTO projects (id) VALUES (?1)", [key.as_str()])
        .await
        .unwrap();
    let query = format!("{key}:app:PGSQL:q");
    conn.execute("INSERT INTO queries (id) VALUES (?1)", [query.as_str()])
        .await
        .unwrap();

    assert_eq!(migrate_project_ids(&conn, None).await.unwrap(), 1);

    let name = single_string(&conn, "SELECT name FROM projects WHERE id = ?1", &key).await;
    assert_eq!(name, key);
    assert_eq!(query_ids(&conn).await, vec![query]);
}

#[tokio::test]
async fn backup_is_written_only_when_projects_need_migrating() {
    let path = std::env::temp_dir().join(format!("rsql-backup-{}.db", Uuid::new_v4()));
    let backup = path.to_str().unwrap();
    let (_db, conn) = local_db().await;
    let key = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO projects (id, name) VALUES (?1, 'already')",
        [key.as_str()],
    )
    .await
    .unwrap();

    assert_eq!(migrate_project_ids(&conn, Some(backup)).await.unwrap(), 0);
    assert!(!path.exists());

    conn.execute("INSERT INTO projects (id) VALUES ('prod')", ())
        .await
        .unwrap();
    assert_eq!(migrate_project_ids(&conn, Some(backup)).await.unwrap(), 1);

    let saved = libsql::Builder::new_local(backup).build().await.unwrap();
    let saved_conn = saved.connect().unwrap();
    let name = single_string(&saved_conn, "SELECT id FROM projects WHERE id = ?1", "prod").await;
    assert_eq!(name, "prod");
    drop(saved_conn);
    drop(saved);
    std::fs::remove_file(&path).unwrap();
}

#[tokio::test]
async fn failure_leaves_every_key_untouched() {
    let (_db, conn) = local_db().await;
    conn.execute_batch("INSERT INTO projects (id) VALUES ('prod'); DROP TABLE queries;")
        .await
        .unwrap();

    assert!(migrate_project_ids(&conn, None).await.is_err());

    let name = single_string(&conn, "SELECT name FROM projects WHERE id = ?1", "prod").await;
    assert_eq!(name, "");
    assert!(conn.is_autocommit());
}
