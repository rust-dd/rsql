use libsql::{Connection, TransactionBehavior};

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
