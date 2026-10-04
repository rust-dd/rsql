use crate::common::enums::{AppError, error_chain, query_failed};
use crate::drivers::pgsql::mutation::quote_ident;
use futures_util::{SinkExt, pin_mut};

#[cfg(test)]
#[path = "csv_import_tests.rs"]
mod integration_tests;

pub async fn parse_csv_preview(
    file_path: &str,
    max_rows: usize,
) -> Result<(Vec<String>, Vec<Vec<String>>), AppError> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(file_path)
        .map_err(|e| AppError::QueryFailed(format!("Failed to read CSV: {}", e)))?;

    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| AppError::QueryFailed(format!("Failed to parse CSV headers: {}", e)))?
        .iter()
        .map(|h| h.to_string())
        .collect();

    let mut rows = Vec::new();
    for result in rdr.records().take(max_rows) {
        let record =
            result.map_err(|e| AppError::QueryFailed(format!("CSV parse error: {}", e)))?;
        rows.push(record.iter().map(|f| f.to_string()).collect());
    }

    Ok((headers, rows))
}

/// Build the INSERT, casting each text parameter to the column's own type.
///
/// CSV fields are text, and binding them straight at a non-text column made
/// tokio-postgres refuse to serialize the parameter, so importing into a table
/// with any numeric, date or boolean column failed outright. Casting on the
/// server side is the same recipe the row-mutation builder uses.
fn build_insert(schema: &str, table: &str, columns: &[(String, String)]) -> String {
    let assignments: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, (_, ty))| format!("${}::text::{}", i + 1, ty))
        .collect();

    format!(
        "INSERT INTO {}.{} ({}) VALUES ({})",
        quote_ident(schema),
        quote_ident(table),
        columns
            .iter()
            .map(|(name, _)| quote_ident(name))
            .collect::<Vec<_>>()
            .join(", "),
        assignments.join(", "),
    )
}

async fn column_types(
    client: &deadpool_postgres::Client,
    schema: &str,
    table: &str,
    wanted: &[(usize, String)],
) -> Result<Vec<(String, String)>, AppError> {
    let rows = client
        .query(
            "SELECT attname::text, format_type(atttypid, atttypmod)
             FROM pg_attribute
             WHERE attrelid = format('%I.%I', $1::text, $2::text)::regclass
               AND attnum > 0
               AND NOT attisdropped",
            &[&schema, &table],
        )
        .await
        .map_err(query_failed)?;

    let known: std::collections::BTreeMap<String, String> = rows
        .into_iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
        .collect();

    wanted
        .iter()
        .map(|(_, name)| {
            known
                .get(name)
                .map(|ty| (name.clone(), ty.clone()))
                .ok_or_else(|| AppError::QueryFailed(format!("Unknown column \"{}\"", name)))
        })
        .collect()
}

pub async fn import_csv_to_table(
    client: &mut deadpool_postgres::Client,
    file_path: &str,
    schema: &str,
    table: &str,
    column_mapping: &[(usize, String)],
) -> Result<usize, AppError> {
    if column_mapping.is_empty() {
        return Err(AppError::QueryFailed(
            "No column mapping provided".to_string(),
        ));
    }

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(file_path)
        .map_err(|e| AppError::QueryFailed(format!("Failed to read CSV: {}", e)))?;

    let columns = column_types(client, schema, table, column_mapping).await?;
    let names = columns
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    let can_copy = client
        .query_one(
            "SELECT NOT row_security_active(c.oid)
                    AND c.relkind IN ('r', 'p') AND NOT c.relhasrules
                    AND NOT EXISTS (
                        SELECT 1 FROM pg_attribute a WHERE a.attrelid = c.oid
                          AND a.attidentity = 'a' AND a.attname = ANY($3::text[])
                    )
             FROM pg_class c WHERE c.oid = format('%I.%I', $1::text, $2::text)::regclass",
            &[&schema, &table, &names],
        )
        .await
        .map_err(query_failed)?
        .get::<_, bool>(0);
    if can_copy {
        return import_copy(client, &mut rdr, schema, table, &columns, column_mapping).await;
    }
    let insert_sql = build_insert(schema, table, &columns);

    // A real transaction rather than a bare BEGIN: a parse error midway used to
    // return without rolling back, handing a connection back to the pool with a
    // transaction still open.
    let tx = client.transaction().await.map_err(query_failed)?;

    let statement = tx.prepare(&insert_sql).await.map_err(|e| {
        AppError::QueryFailed(format!("Failed to prepare statement: {}", error_chain(&e)))
    })?;

    let mut imported = 0usize;
    for result in rdr.records() {
        let record = result.map_err(|e| {
            AppError::QueryFailed(format!("CSV parse error at row {}: {}", imported + 1, e))
        })?;

        // An absent or empty field becomes NULL rather than an empty string,
        // which is what a numeric or date column needs and what an empty CSV
        // field conventionally means.
        let values: Vec<Option<String>> = column_mapping
            .iter()
            .map(|(idx, _)| match record.get(*idx) {
                Some("") | None => None,
                Some(value) => Some(value.to_string()),
            })
            .collect();

        let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = values
            .iter()
            .map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        tx.execute(&statement, &params).await.map_err(|e| {
            AppError::QueryFailed(format!(
                "Import failed at row {}: {}",
                imported + 1,
                error_chain(&e)
            ))
        })?;
        imported += 1;
    }

    tx.commit()
        .await
        .map_err(|e| AppError::QueryFailed(format!("Failed to commit: {}", error_chain(&e))))?;

    Ok(imported)
}

fn copy_error(error: tokio_postgres::Error) -> AppError {
    let context = error
        .as_db_error()
        .and_then(|error| error.where_())
        .unwrap_or_default();
    AppError::QueryFailed(format!(
        "CSV import failed: {}. {}",
        error_chain(&error),
        context
    ))
}

async fn import_copy(
    client: &mut deadpool_postgres::Client,
    reader: &mut csv::Reader<std::fs::File>,
    schema: &str,
    table: &str,
    columns: &[(String, String)],
    mapping: &[(usize, String)],
) -> Result<usize, AppError> {
    let sql = format!(
        "COPY {}.{} ({}) FROM STDIN WITH (FORMAT text)",
        quote_ident(schema),
        quote_ident(table),
        columns
            .iter()
            .map(|(column, _)| quote_ident(column))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let tx = client.transaction().await.map_err(query_failed)?;
    let sink = tx.copy_in(&sql).await.map_err(copy_error)?;
    pin_mut!(sink);
    let mut buffer = String::with_capacity(64 * 1024);
    let mut imported = 0usize;
    for record in reader.records() {
        let record = record.map_err(|error| {
            AppError::QueryFailed(format!("CSV parse error at row {}: {error}", imported + 1))
        })?;
        for (column, (index, _)) in mapping.iter().enumerate() {
            if column > 0 {
                buffer.push('\t');
            }
            match record.get(*index) {
                None | Some("") => buffer.push_str("\\N"),
                Some(value) => {
                    for char in value.chars() {
                        match char {
                            '\\' => buffer.push_str("\\\\"),
                            '\n' => buffer.push_str("\\n"),
                            '\r' => buffer.push_str("\\r"),
                            '\t' => buffer.push_str("\\t"),
                            other => buffer.push(other),
                        }
                    }
                }
            }
        }
        buffer.push('\n');
        imported += 1;
        if buffer.len() >= 64 * 1024 {
            sink.send(std::io::Cursor::new(
                std::mem::take(&mut buffer).into_bytes(),
            ))
            .await
            .map_err(copy_error)?;
        }
    }
    if !buffer.is_empty() {
        sink.send(std::io::Cursor::new(buffer.into_bytes()))
            .await
            .map_err(copy_error)?;
    }
    sink.as_mut().finish().await.map_err(copy_error)?;
    tx.commit().await.map_err(query_failed)?;
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Vec<(String, String)> {
        vec![
            ("id".to_string(), "integer".to_string()),
            ("name".to_string(), "text".to_string()),
        ]
    }

    #[test]
    fn each_parameter_is_cast_to_its_column_type() {
        let sql = build_insert("public", "t", &columns());
        assert_eq!(
            sql,
            "INSERT INTO \"public\".\"t\" (\"id\", \"name\") \
             VALUES ($1::text::integer, $2::text::text)"
        );
    }

    #[test]
    fn identifiers_are_escaped() {
        let cols = vec![("we\"ird".to_string(), "text".to_string())];
        let sql = build_insert("my schema", "my\"table", &cols);
        assert!(sql.contains("\"my schema\".\"my\"\"table\""));
        assert!(sql.contains("\"we\"\"ird\""));
    }
}
