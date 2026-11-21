use crate::storage::ConnectionRef;
use std::collections::BTreeMap;

pub async fn select_queries<'a>(
    conn: impl Into<ConnectionRef<'a>>,
) -> libsql::Result<BTreeMap<String, String>> {
    let conn = conn.into().0;
    let mut rows = conn
        .query("SELECT id, sql FROM queries ORDER BY id", ())
        .await?;
    let mut queries = BTreeMap::new();
    while let Some(row) = rows.next().await? {
        queries.insert(row.get(0)?, row.get(1)?);
    }
    Ok(queries)
}

pub async fn upsert_query<'a>(
    conn: impl Into<ConnectionRef<'a>>,
    query_id: &str,
    sql: &str,
) -> libsql::Result<()> {
    let conn = conn.into().0;
    conn.execute(
        "INSERT OR REPLACE INTO queries (id, sql) VALUES (?1, ?2)",
        [query_id, sql],
    )
    .await?;
    Ok(())
}

pub async fn delete_query<'a>(
    conn: impl Into<ConnectionRef<'a>>,
    query_id: &str,
) -> libsql::Result<()> {
    let conn = conn.into().0;
    conn.execute("DELETE FROM queries WHERE id = ?1", [query_id])
        .await?;
    Ok(())
}
