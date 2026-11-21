use super::{CsvRows, ImportTable};
use crate::error::{Error, error_chain, query_failed};
use crate::mutation::quote_ident;
use crate::postgres::text_param::TextParam;
use futures_util::{SinkExt, TryStreamExt, pin_mut};
use tokio_postgres::{
    GenericClient, Statement, Transaction,
    types::{ToSql, Type},
};

/// Build the INSERT, casting each text parameter to the column's own type.
///
/// CSV fields are text, and binding them straight at a non-text column made
/// tokio-postgres refuse to serialize the parameter, so importing into a table
/// with any numeric, date or boolean column failed outright. Casting on the
/// server side is the same recipe the row-mutation builder uses.
fn build_insert(schema: &str, table: &str, columns: &[(String, String)]) -> String {
    let assignments = columns
        .iter()
        .enumerate()
        .map(|(i, (_, ty))| format!("${}::text::{ty}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {}.{} ({}) VALUES ({assignments})",
        quote_ident(schema),
        quote_ident(table),
        columns
            .iter()
            .map(|(name, _)| quote_ident(name))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

async fn column_types(
    client: &impl GenericClient,
    schema: &str,
    table: &str,
    wanted: &[(usize, String)],
) -> Result<Vec<(String, String)>, Error> {
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
                .ok_or_else(|| Error::QueryFailed(format!("Unknown column \"{}\"", name)))
        })
        .collect()
}

pub async fn import_csv_to_table(
    client: &mut tokio_postgres::Client,
    file_path: &str,
    schema: &str,
    table: &str,
    column_mapping: &[(usize, String)],
) -> Result<usize, Error> {
    if column_mapping.is_empty() {
        return Err(Error::QueryFailed("No column mapping provided".into()));
    }
    let tx = client.transaction().await.map_err(query_failed)?;
    let (mut rows, worker) = CsvRows::open(file_path.into(), None);
    let result = async {
        rows.ready().await?;
        let columns = column_types(&tx, schema, table, column_mapping).await?;
        if can_copy(&tx, schema, table, &columns).await? {
            return import_copy(&tx, &mut rows, schema, table, &columns, column_mapping).await;
        }
        let no_probe = None::<fn() -> std::future::Ready<_>>;
        let statement = tx
            .prepare(&build_insert(schema, table, &columns))
            .await
            .map_err(|e| {
                Error::QueryFailed(format!("Failed to prepare statement: {}", error_chain(&e)))
            })?;
        import_insert(&tx, &mut rows, &statement, column_mapping, no_probe).await
    }
    .await;
    drop(rows);
    worker
        .await
        .map_err(|e| Error::QueryFailed(e.to_string()))?;
    let count = result?;
    tx.commit().await.map_err(query_failed)?;
    Ok(count)
}

pub async fn import_bound_csv<F: Future<Output = Result<(), Error>> + Send>(
    tx: &Transaction<'_>,
    rows: &mut CsvRows,
    target: &ImportTable,
    mapping: &[(usize, String)],
    confirm: impl Fn() -> F + Send + Sync,
) -> Result<usize, Error> {
    if rows
        .width
        .is_some_and(|width| mapping.iter().any(|(i, _)| *i >= width))
    {
        return Err(Error::QueryFailed(
            "CSV mapping is outside the preview headers".into(),
        ));
    }
    let columns = target.mapped_columns(mapping)?;
    target.lock_current(tx).await?;
    let statement = super::import_input::prepare_native_insert(tx, target, &columns).await?;
    import_insert(tx, rows, &statement, mapping, Some(confirm)).await
}

pub(super) async fn import_insert<F: Future<Output = Result<(), Error>> + Send>(
    client: &Transaction<'_>,
    rows: &mut CsvRows,
    statement: &Statement,
    column_mapping: &[(usize, String)],
    confirm: Option<impl Fn() -> F + Send + Sync>,
) -> Result<usize, Error> {
    let mut imported = 0usize;
    let mut position = 0;
    while let Some(record) = rows.next().await? {
        position += 1;
        // An absent or empty field becomes NULL rather than an empty string,
        // which is what a numeric or date column needs and what an empty CSV
        // field conventionally means.
        let values = column_mapping
            .iter()
            .map(|(idx, _)| TextParam(record.get(*idx).filter(|value| !value.is_empty())))
            .collect::<Vec<_>>();
        let params = values
            .iter()
            .zip(statement.params())
            .map(|(value, ty)| {
                if *ty == Type::TEXT {
                    &value.0 as &(dyn ToSql + Sync)
                } else {
                    value
                }
            })
            .collect::<Vec<_>>();
        let row_error = |e: tokio_postgres::Error| {
            Error::QueryFailed(format!(
                "Import failed at row {position}: {}",
                error_chain(&e)
            ))
        };
        let result = if let Some(probe) = &confirm {
            let portal = client.bind(statement, &params).await.map_err(&row_error)?;
            probe().await?;
            async {
                let result = client.query_portal_raw(&portal, 0).await?;
                pin_mut!(result);
                while result.as_mut().try_next().await?.is_some() {}
                Ok::<_, tokio_postgres::Error>(result.rows_affected().unwrap_or(0) as usize)
            }
            .await
        } else {
            client.execute(statement, &params).await.map(|_| 1)
        };
        imported += result.map_err(row_error)?;
    }

    Ok(imported)
}

async fn can_copy(
    client: &Transaction<'_>,
    schema: &str,
    table: &str,
    columns: &[(String, String)],
) -> Result<bool, Error> {
    let names = columns
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    let copy_column = super::import_input::COPY_COLUMN;
    let can_copy = client
        .query_one(
            &*format!(
                "SELECT COALESCE(pg_catalog.bool_and({copy_column}),false)
             FROM pg_catalog.pg_class c JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid
             WHERE c.oid=pg_catalog.format('%I.%I', $1::text,$2::text)::pg_catalog.regclass
               AND a.attnum>0 AND NOT a.attisdropped AND a.attname=ANY($3::text[])"
            ),
            &[&schema, &table, &names],
        )
        .await
        .map_err(query_failed)?
        .get::<_, bool>(0);
    Ok(can_copy)
}

fn copy_error(error: tokio_postgres::Error) -> Error {
    let context = error
        .as_db_error()
        .and_then(|error| error.where_())
        .unwrap_or_default();
    Error::QueryFailed(format!(
        "CSV import failed: {}. {}",
        error_chain(&error),
        context
    ))
}

async fn import_copy(
    tx: &Transaction<'_>,
    reader: &mut CsvRows,
    schema: &str,
    table: &str,
    columns: &[(String, String)],
    mapping: &[(usize, String)],
) -> Result<usize, Error> {
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
    let sink = tx.copy_in(&sql).await.map_err(copy_error)?;
    pin_mut!(sink);
    let mut buffer = String::with_capacity(64 * 1024);
    let mut imported = 0usize;
    while let Some(record) = reader.next().await? {
        for (column, (index, _)) in mapping.iter().enumerate() {
            if column > 0 {
                buffer.push('\t');
            }
            match record.get(*index) {
                None | Some("") => buffer.push_str("\\N"),
                Some(value) => {
                    for char in value.chars() {
                        if buffer.len() >= 64 * 1024 {
                            sink.send(std::io::Cursor::new(
                                std::mem::take(&mut buffer).into_bytes(),
                            ))
                            .await
                            .map_err(copy_error)?;
                        }
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
    Ok(imported)
}
