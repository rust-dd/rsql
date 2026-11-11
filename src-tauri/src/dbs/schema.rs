use libsql::Connection;

/// Creates the local tables and adds columns that later releases introduced.
pub(crate) async fn create_local_schema(conn: &Connection) -> libsql::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            driver TEXT NOT NULL DEFAULT 'PGSQL',
            username TEXT NOT NULL DEFAULT '',
            password TEXT NOT NULL DEFAULT '',
            database TEXT NOT NULL DEFAULT '',
            host TEXT NOT NULL DEFAULT '',
            port TEXT NOT NULL DEFAULT '',
            ssl TEXT NOT NULL DEFAULT 'false'
        )",
        (),
    )
    .await?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS queries (
            id TEXT PRIMARY KEY,
            sql TEXT NOT NULL DEFAULT ''
        )",
        (),
    )
    .await?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS workspaces (
            name TEXT PRIMARY KEY,
            tabs TEXT NOT NULL DEFAULT '[]'
        )",
        (),
    )
    .await?;

    for col in [
        "ssh_enabled",
        "ssh_host",
        "ssh_port",
        "ssh_user",
        "ssh_password",
        "ssh_key_path",
        "name",
        "legacy_id",
        "server_alias",
    ] {
        conn.execute(
            &format!("ALTER TABLE projects ADD COLUMN {col} TEXT NOT NULL DEFAULT ''"),
            (),
        )
        .await
        .ok(); // Ignore "column already exists" errors
    }

    Ok(())
}
