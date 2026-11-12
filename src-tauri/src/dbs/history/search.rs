use libsql::{Connection, Value, params_from_iter};

use super::{HistoryEntry, HistoryFilter, HistoryPage};

const PAGE_SIZE: u32 = 100;
const COLUMNS: &str = "id, project_id, database, sql, sql_truncated, executed_at, duration_ms, row_count, success, error";

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(crate) async fn search(
    conn: &Connection,
    filter: &HistoryFilter,
) -> libsql::Result<HistoryPage> {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    let mut add = |clause: &str, value: Value| {
        params.push(value);
        clauses.push(clause.replace('?', &format!("?{}", params.len())));
    };
    if let Some(text) = filter
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        add(
            "sql LIKE ? ESCAPE '\\'",
            Value::Text(format!("%{}%", escape_like(text))),
        );
    }
    if let Some(project_id) = &filter.project_id {
        add("project_id = ?", Value::Text(project_id.clone()));
    }
    if let Some(database) = &filter.database {
        add("database = ?", Value::Text(database.clone()));
    }
    if let Some(success) = filter.success {
        add("success = ?", Value::Integer(success.into()));
    }
    if let Some(since) = filter.since {
        add("executed_at >= ?", Value::Integer(since));
    }
    if let Some(before_id) = filter.before_id {
        add("id < ?", Value::Integer(before_id));
    }

    let limit = filter.limit.unwrap_or(PAGE_SIZE).clamp(1, 500);
    let filter = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let mut rows = conn
        .query(
            &format!(
                "SELECT {COLUMNS} FROM query_history {filter} ORDER BY id DESC LIMIT {}",
                limit + 1
            ),
            params_from_iter(params),
        )
        .await?;

    let mut entries = Vec::new();
    while let Some(row) = rows.next().await? {
        entries.push(HistoryEntry {
            id: row.get(0)?,
            project_id: row.get(1)?,
            database: row.get(2)?,
            sql: row.get(3)?,
            sql_truncated: row.get(4)?,
            executed_at: row.get(5)?,
            duration_ms: row.get(6)?,
            row_count: row.get(7)?,
            success: row.get(8)?,
            error: row.get(9)?,
        });
    }
    let has_more = entries.len() > limit as usize;
    entries.truncate(limit as usize);
    Ok(HistoryPage { entries, has_more })
}

pub(crate) async fn databases(
    conn: &Connection,
    project_id: Option<&str>,
) -> libsql::Result<Vec<String>> {
    let mut rows = conn
        .query(
            "SELECT DISTINCT database FROM query_history
             WHERE ?1 IS NULL OR project_id = ?1 ORDER BY database",
            [project_id],
        )
        .await?;
    let mut names = Vec::new();
    while let Some(row) = rows.next().await? {
        names.push(row.get(0)?);
    }
    Ok(names)
}
