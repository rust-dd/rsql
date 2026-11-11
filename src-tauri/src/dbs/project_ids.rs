use libsql::{Connection, Transaction, TransactionBehavior};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

#[cfg(test)]
mod tests;

/// Re-keys name-keyed projects onto UUIDs, moving the saved queries and workspace tabs that embed the key.
/// `backup_path` receives a copy of the database taken before anything changes.
pub(crate) async fn migrate_project_ids(
    conn: &Connection,
    backup_path: Option<&str>,
) -> libsql::Result<usize> {
    let unnamed = string_column(conn, "SELECT id FROM projects WHERE name = ''").await?;
    if unnamed.is_empty() {
        return Ok(0);
    }
    if let Some(path) = backup_path {
        conn.execute("VACUUM INTO ?1", [path]).await?;
    }

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await?;
    let mut rekeyed = BTreeMap::new();
    for old in &unnamed {
        if Uuid::parse_str(old).is_ok() {
            // An older release rewrote this row and cleared its name; the key is already stable.
            tx.execute(
                "UPDATE projects SET name = id WHERE id = ?1",
                [old.as_str()],
            )
            .await?;
        } else {
            let new = Uuid::new_v4().to_string();
            tx.execute(
                "UPDATE projects SET id = ?1, name = ?2, legacy_id = ?2 WHERE id = ?2",
                [new.as_str(), old.as_str()],
            )
            .await?;
            rekeyed.insert(old.clone(), new);
        }
    }
    if !rekeyed.is_empty() {
        rekey_saved_queries(&tx, &rekeyed).await?;
        rekey_workspace_tabs(&tx, &rekeyed).await?;
    }
    tx.commit().await?;
    Ok(unnamed.len())
}

async fn string_column(conn: &Connection, sql: &str) -> libsql::Result<Vec<String>> {
    let mut rows = conn.query(sql, ()).await?;
    let mut values = Vec::new();
    while let Some(row) = rows.next().await? {
        values.push(row.get::<String>(0)?);
    }
    Ok(values)
}

async fn rekey_saved_queries(
    tx: &Transaction,
    rekeyed: &BTreeMap<String, String>,
) -> libsql::Result<()> {
    for id in string_column(tx, "SELECT id FROM queries").await? {
        // Longest key wins, so "prod:eu" is not read as project "prod" with database "eu".
        let owner = rekeyed
            .iter()
            .filter(|(old, _)| {
                id.strip_prefix(old.as_str())
                    .is_some_and(|rest| rest.starts_with(':'))
            })
            .max_by_key(|(old, _)| old.len());
        if let Some((old, new)) = owner {
            let moved = format!("{new}{}", &id[old.len()..]);
            tx.execute(
                "UPDATE queries SET id = ?1 WHERE id = ?2",
                [moved.as_str(), id.as_str()],
            )
            .await?;
        }
    }
    Ok(())
}

async fn rekey_workspace_tabs(
    tx: &Transaction,
    rekeyed: &BTreeMap<String, String>,
) -> libsql::Result<()> {
    let mut rows = tx.query("SELECT name, tabs FROM workspaces", ()).await?;
    let mut workspaces = Vec::new();
    while let Some(row) = rows.next().await? {
        workspaces.push((row.get::<String>(0)?, row.get::<String>(1)?));
    }

    for (name, tabs) in workspaces {
        let Ok(Value::Array(mut tabs)) = serde_json::from_str::<Value>(&tabs) else {
            continue;
        };
        let mut changed = false;
        for tab in &mut tabs {
            let Some(slot) = tab.get_mut("projectId") else {
                continue;
            };
            if let Some(new) = slot.as_str().and_then(|old| rekeyed.get(old)).cloned() {
                *slot = Value::String(new);
                changed = true;
            }
        }
        if changed {
            let tabs = Value::Array(tabs).to_string();
            tx.execute(
                "UPDATE workspaces SET tabs = ?1 WHERE name = ?2",
                [tabs.as_str(), name.as_str()],
            )
            .await?;
        }
    }
    Ok(())
}
