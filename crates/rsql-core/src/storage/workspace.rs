use crate::storage::ConnectionRef;

pub async fn save_workspace<'a>(
    conn: impl Into<ConnectionRef<'a>>,
    name: &str,
    tabs: &str,
) -> libsql::Result<()> {
    let conn = conn.into().0;
    conn.execute(
        "INSERT OR REPLACE INTO workspaces (name, tabs) VALUES (?1, ?2)",
        [name, tabs],
    )
    .await?;
    Ok(())
}

pub async fn load_workspaces<'a>(
    conn: impl Into<ConnectionRef<'a>>,
) -> libsql::Result<Vec<(String, String)>> {
    let conn = conn.into().0;
    let mut rows = conn
        .query("SELECT name, tabs FROM workspaces ORDER BY name", ())
        .await?;
    let mut workspaces = Vec::new();
    while let Some(row) = rows.next().await? {
        workspaces.push((row.get(0)?, row.get(1)?));
    }
    Ok(workspaces)
}

pub async fn delete_workspace<'a>(
    conn: impl Into<ConnectionRef<'a>>,
    name: &str,
) -> libsql::Result<()> {
    let conn = conn.into().0;
    conn.execute("DELETE FROM workspaces WHERE name = ?1", [name])
        .await?;
    Ok(())
}
