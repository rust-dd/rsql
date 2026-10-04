use libsql::{Connection, TransactionBehavior};

#[cfg(test)]
#[path = "migrations_benchmark.rs"]
mod benchmark;

/// Remove unreachable legacy result snapshots and reclaim their disk space once.
pub(crate) async fn cleanup_legacy_snapshots(conn: &Connection) -> libsql::Result<bool> {
    let has_snapshots = {
        let mut rows = conn
            .query(
                "SELECT 1 FROM sqlite_schema
                 WHERE type = 'table'
                   AND name IN ('virtual_query_pages', 'virtual_query_snapshots')
                 LIMIT 1",
                (),
            )
            .await?;
        rows.next().await?.is_some()
    };
    if !has_snapshots {
        return Ok(false);
    }

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await?;
    tx.execute("DROP TABLE IF EXISTS virtual_query_pages", ())
        .await?;
    tx.execute("DROP TABLE IF EXISTS virtual_query_snapshots", ())
        .await?;
    tx.commit().await?;

    conn.execute("VACUUM", ()).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::cleanup_legacy_snapshots;

    #[tokio::test]
    async fn fresh_database_skips_vacuum() {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();

        // VACUUM would fail inside this transaction if the cleanup attempted it.
        let tx = conn.transaction().await.unwrap();
        assert!(!cleanup_legacy_snapshots(&tx).await.unwrap());
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_cleanup_preserves_user_data_and_skips_subsequent_vacuum() {
        for tables in [
            &["virtual_query_pages"][..],
            &["virtual_query_snapshots"][..],
            &["virtual_query_pages", "virtual_query_snapshots"][..],
        ] {
            let db = libsql::Builder::new_local(":memory:")
                .build()
                .await
                .unwrap();
            let conn = db.connect().unwrap();
            conn.execute_batch(
                "CREATE TABLE projects (id TEXT PRIMARY KEY);
                 INSERT INTO projects VALUES ('saved-project');
                 CREATE TABLE queries (sql TEXT);
                 INSERT INTO queries VALUES ('SELECT 1');
                 CREATE TABLE workspaces (tabs TEXT);
                 INSERT INTO workspaces VALUES ('[\"saved-tab\"]');",
            )
            .await
            .unwrap();
            for table in tables {
                conn.execute(&format!("CREATE TABLE {table} (payload TEXT)"), ())
                    .await
                    .unwrap();
                conn.execute(&format!("INSERT INTO {table} VALUES ('old-result')"), ())
                    .await
                    .unwrap();
            }

            assert!(cleanup_legacy_snapshots(&conn).await.unwrap());

            let tx = conn.transaction().await.unwrap();
            assert!(!cleanup_legacy_snapshots(&tx).await.unwrap());
            tx.rollback().await.unwrap();

            let mut rows = conn
                .query(
                    "SELECT id, sql, tabs FROM projects CROSS JOIN queries CROSS JOIN workspaces",
                    (),
                )
                .await
                .unwrap();
            let row = rows.next().await.unwrap().unwrap();
            assert_eq!(row.get::<String>(0).unwrap(), "saved-project");
            assert_eq!(row.get::<String>(1).unwrap(), "SELECT 1");
            assert_eq!(row.get::<String>(2).unwrap(), "[\"saved-tab\"]");
            assert!(rows.next().await.unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn a_failed_drop_rolls_back_the_cleanup() {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        conn.execute_batch(
            "CREATE TABLE virtual_query_pages (payload TEXT);
             INSERT INTO virtual_query_pages VALUES ('old-result');
             CREATE VIEW virtual_query_snapshots AS SELECT 1;",
        )
        .await
        .unwrap();

        assert!(cleanup_legacy_snapshots(&conn).await.is_err());

        let mut rows = conn
            .query("SELECT payload FROM virtual_query_pages", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "old-result");
        assert!(conn.is_autocommit());
    }
}
