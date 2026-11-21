use crate::storage::ConnectionRef;
use std::collections::BTreeMap;

/// Column order of the details vector exchanged with the frontend, followed by `SERVER_ALIAS`.
/// `select_projects` appends `legacy_id`, which only the id migration writes.
const DETAIL_COLUMNS: [&str; 14] = [
    "driver",
    "username",
    "password",
    "database",
    "host",
    "port",
    "ssl",
    "ssh_enabled",
    "ssh_host",
    "ssh_port",
    "ssh_user",
    "ssh_password",
    "ssh_key_path",
    "name",
];

/// Set when a project is created and by `set_server_alias`; editing a connection leaves it alone.
const SERVER_ALIAS: &str = "server_alias";

pub async fn select_projects<'a>(
    conn: impl Into<ConnectionRef<'a>>,
) -> libsql::Result<BTreeMap<String, Vec<String>>> {
    let conn = conn.into().0;
    let sql = format!(
        "SELECT id, {}, {SERVER_ALIAS}, legacy_id FROM projects ORDER BY id",
        DETAIL_COLUMNS.join(", ")
    );
    let mut rows = conn.query(&sql, ()).await?;

    let mut projects = BTreeMap::new();
    while let Some(row) = rows.next().await? {
        let id = row.get::<String>(0)?;
        let mut details = Vec::with_capacity(DETAIL_COLUMNS.len() + 2);
        for idx in 1..=DETAIL_COLUMNS.len() as i32 + 2 {
            details.push(row.get::<String>(idx)?);
        }
        projects.insert(id, details);
    }
    Ok(projects)
}

pub async fn select_project<'a>(
    conn: impl Into<ConnectionRef<'a>>,
    id: &str,
) -> libsql::Result<Option<Vec<String>>> {
    let conn = conn.into().0;
    let sql = format!(
        "SELECT {}, {SERVER_ALIAS} FROM projects WHERE id = ?1",
        DETAIL_COLUMNS.join(", ")
    );
    let mut rows = conn.query(&sql, [id]).await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    (0..=DETAIL_COLUMNS.len())
        .map(|i| row.get::<String>(i as i32))
        .collect::<libsql::Result<_>>()
        .map(Some)
}

pub async fn upsert_project<'a>(
    conn: impl Into<ConnectionRef<'a>>,
    project_id: &str,
    details: &[String],
) -> libsql::Result<()> {
    let conn = conn.into().0;
    let placeholders = (1..=DETAIL_COLUMNS.len() + 2)
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>();
    let updates = DETAIL_COLUMNS
        .iter()
        .map(|column| format!("{column} = excluded.{column}"))
        .collect::<Vec<_>>();
    let sql = format!(
        "INSERT INTO projects (id, {}, {SERVER_ALIAS}) VALUES ({}) ON CONFLICT(id) DO UPDATE SET {}",
        DETAIL_COLUMNS.join(", "),
        placeholders.join(", "),
        updates.join(", ")
    );
    let mut values = vec![project_id.to_string()];
    for (idx, column) in DETAIL_COLUMNS.iter().enumerate() {
        let fallback = if *column == "ssl" { "false" } else { "" };
        values.push(
            details
                .get(idx)
                .map_or(fallback, String::as_str)
                .to_string(),
        );
    }
    values.push(
        details
            .get(DETAIL_COLUMNS.len())
            .cloned()
            .unwrap_or_default(),
    );
    conn.execute(&sql, libsql::params_from_iter(values)).await?;
    Ok(())
}

pub async fn set_server_alias<'a>(
    conn: impl Into<ConnectionRef<'a>>,
    project_ids: &[String],
    alias: &str,
) -> libsql::Result<()> {
    let conn = conn.into().0;
    let tx = conn.transaction().await?;
    for id in project_ids {
        tx.execute(
            &format!("UPDATE projects SET {SERVER_ALIAS} = ?1 WHERE id = ?2"),
            [alias, id.as_str()],
        )
        .await?;
    }
    tx.commit().await
}

pub async fn delete_project<'a>(
    conn: impl Into<ConnectionRef<'a>>,
    project_id: &str,
) -> libsql::Result<()> {
    let conn = conn.into().0;
    conn.execute("DELETE FROM projects WHERE id = ?1", [project_id])
        .await?;
    super::history::clear(conn, Some(project_id)).await
}

pub async fn delete_projects<'a>(
    conn: impl Into<ConnectionRef<'a>>,
    ids: &[String],
) -> libsql::Result<()> {
    let tx = conn.into().0.transaction().await?;
    for id in ids {
        delete_project(&tx, id).await?;
    }
    tx.commit().await
}
