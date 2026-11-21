use super::provenance::{Relation, load_relation};
use super::roles_schema_objects::ImportTable;
use crate::mutation::quote_ident;
use std::sync::Arc;
use tokio_postgres::{Client, GenericClient, Transaction, types::ToSql};

use crate::error::{Error, query_failed};
use crate::mutation::{ColumnTypes, MutationKind, RowMutation, build_statement};

#[derive(Debug, serde::Serialize)]
pub struct MutationReport {
    pub updated: usize,
    pub deleted: usize,
}

pub struct EditBinding {
    table: ImportTable,
    relation: Relation,
    name: String,
}

impl EditBinding {
    /// Confirm the bound portal against a fresh catalog snapshot, including when
    /// the writer uses repeatable-read isolation.
    pub async fn is_current(&self, observer: &Client) -> Result<bool, Error> {
        Ok(
            load_relation(observer, &self.name).await?.as_ref() == Some(&self.relation)
                && self.table.binding_current(observer).await?,
        )
    }
}

/// Column types straight from the catalog. Used both to cast parameters and to
/// reject column names the table does not actually have.
async fn load_column_types(
    client: &impl GenericClient,
    schema: &str,
    table: &str,
) -> Result<ColumnTypes, Error> {
    let rows = client
        .query(
            "SELECT attname::text, pg_catalog.format_type(atttypid, atttypmod)
             FROM pg_catalog.pg_attribute
             WHERE attrelid = pg_catalog.format('%I.%I', $1::text, $2::text)::regclass
               AND attnum > 0
               AND NOT attisdropped",
            &[&schema, &table],
        )
        .await
        .map_err(query_failed)?;

    if rows.is_empty() {
        return Err(Error::QueryFailed(format!(
            "Table {}.{} has no readable columns",
            schema, table
        )));
    }

    Ok(rows
        .into_iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
        .collect())
}

/// Render a row key for error messages, e.g. `id=7, tenant=acme`.
fn describe_key(key: &[(String, Option<String>)]) -> String {
    key.iter()
        .map(|(column, value)| match value {
            Some(v) => format!("{}={}", column, v),
            None => format!("{}=NULL", column),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Apply grid row edits as parameterized statements in a single transaction.
///
/// Every statement must affect exactly one row. Zero means the row is gone or
/// its key changed underneath the grid; more than one means the supplied key is
/// not unique. Either aborts the whole transaction, so a partial apply is not
/// possible and a silent no-op is reported as an error instead of success.
pub async fn apply_row_mutations(
    client: &mut Client,
    schema: &str,
    table: &str,
    mutations: &[RowMutation],
    timeout_ms: Option<u32>,
) -> Result<MutationReport, Error> {
    if mutations.is_empty() {
        return Ok(MutationReport {
            updated: 0,
            deleted: 0,
        });
    }

    let tx = client.transaction().await.map_err(query_failed)?;
    if let Some(ms) = timeout_ms.filter(|ms| *ms > 0) {
        tx.batch_execute(&format!("SET LOCAL statement_timeout = {ms}"))
            .await
            .map_err(query_failed)?;
    }
    let types = load_column_types(&tx, schema, table).await?;
    let mut report = MutationReport {
        updated: 0,
        deleted: 0,
    };
    let statements = mutations
        .iter()
        .map(|mutation| build_statement(schema, table, mutation, &types))
        .collect::<Result<Vec<_>, _>>()?;
    for (mutation, stmt) in mutations.iter().zip(statements) {
        let params = stmt
            .params
            .iter()
            .map(|v| v as &(dyn ToSql + Sync))
            .collect::<Vec<_>>();
        let affected = tx
            .execute(stmt.sql.as_str(), &params)
            .await
            .map_err(query_failed)?;
        report.record(mutation, affected)?;
    }
    tx.commit().await.map_err(query_failed)?;
    Ok(report)
}

/// Lock before resolving the target again. Relation DDL and changes to traditional
/// inheritance must settle before an old key can be used in a mutation.
pub async fn apply_proven_mutations<F: Future<Output = Result<bool, Error>> + Send>(
    tx: &Transaction<'_>,
    expected: &Relation,
    mapped: &[Option<i16>],
    current_relation: &str,
    mutations: &[RowMutation],
    confirm: impl Fn(Arc<EditBinding>) -> F + Sync,
) -> Result<MutationReport, Error> {
    let name = format!(
        "{}.{}",
        quote_ident(&expected.schema),
        quote_ident(&expected.table)
    );
    let stale = || {
        Error::QueryFailed(
            "The table, primary key, or mapped columns changed. Run the query again.".into(),
        )
    };
    let table = ImportTable::load(tx, &expected.schema, &expected.table).await?;
    if table.oid != expected.oid {
        return Err(stale());
    }
    let single_update = matches!(mutations, [m] if m.kind == MutationKind::Update);
    if single_update {
        let key = expected
            .attributes
            .iter()
            .find(|a| Some(&a.number) == expected.primary_key.first())
            .ok_or_else(stale)?;
        let lock = tx
            .prepare(&format!(
                "SELECT {} FROM ONLY {name} FOR UPDATE",
                quote_ident(&key.name)
            ))
            .await
            .map_err(query_failed)?;
        if !matches!(lock.columns(), [c] if c.table_oid()==Some(expected.oid) && c.column_id()==Some(key.number))
        {
            return Err(stale());
        }
    } else {
        tx.batch_execute(&format!("LOCK TABLE {name} IN SHARE ROW EXCLUSIVE MODE")).await.map_err(|e| {
            if mutations.len() > 1 && e.code()==Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE) {
                Error::QueryFailed("Multi-row editing requires a table-level write grant to position every original row. Apply one update at a time.".into())
            } else {
                query_failed(e)
            }
        })?;
    }
    let live = load_relation(tx, &name).await?.ok_or_else(stale)?;
    if tx
        .query_one(
            "SELECT pg_catalog.to_regclass($1)::oid",
            &[&current_relation],
        )
        .await
        .map_err(query_failed)?
        .get::<_, Option<u32>>(0)
        != Some(expected.oid)
        || live.oid != expected.oid
        || !live.descendant_safe
        || !expected.descendant_safe
        || live.primary_key.is_empty()
        || live.primary_key != expected.primary_key
        || live
            .primary_key
            .iter()
            .any(|key| mapped.iter().filter(|n| **n == Some(*key)).count() != 1)
        || mapped
            .iter()
            .flatten()
            .chain(&expected.primary_key)
            .any(|n| {
                let old = expected.attributes.iter().find(|a| a.number == *n);
                old.is_none() || old != live.attributes.iter().find(|a| a.number == *n)
            })
    {
        return Err(stale());
    }
    if !live.lossless_key() {
        return Err(Error::QueryFailed(
            "Primary key text cannot reliably identify the original row for this type.".into(),
        ));
    }
    let keys: Vec<_> = live
        .primary_key
        .iter()
        .map(|n| {
            live.attributes
                .iter()
                .find(|a| a.number == *n)
                .map(|a| a.name.as_str())
        })
        .collect();
    for m in mutations {
        if m.kind == MutationKind::Update && m.set.is_empty() {
            return Err(Error::QueryFailed("No assignments to apply.".into()));
        }
        if !m
            .pk
            .iter()
            .map(|(name, value)| value.as_ref().map(|_| name.as_str()))
            .eq(keys.iter().copied())
            || m.set.iter().any(|(name, _)| {
                !live
                    .attributes
                    .iter()
                    .any(|a| a.writable && a.name == *name && mapped.contains(&Some(a.number)))
            })
        {
            return Err(stale());
        }
    }
    if table != ImportTable::load(tx, &live.schema, &live.table).await? {
        return Err(stale());
    }
    let positions = positions::open(tx, &live, mutations, single_update).await?;
    let binding = Arc::new(EditBinding {
        table,
        relation: live,
        name,
    });
    let mut report = MutationReport {
        updated: 0,
        deleted: 0,
    };
    for (i, (mutation, position)) in mutations.iter().zip(positions).enumerate() {
        if i > 0 {
            position.check_count(tx, mutation).await?;
        }
        let affected = positioned_write(tx, mutation, &position, &binding, &confirm).await?;
        if affected == 0 {
            return Err(Error::QueryFailed(format!(
                "Expected 1 row but matched 0. Original row changed or the write was refused by a policy or trigger. Row key: {}. Nothing was changed.",
                describe_key(&mutation.pk)
            )));
        }
        report.record(mutation, affected)?;
        tx.batch_execute(&format!("CLOSE {}", quote_ident(&position.cursor)))
            .await
            .map_err(query_failed)?;
    }
    Ok(report)
}

async fn positioned_write<F: Future<Output = Result<bool, Error>> + Send>(
    tx: &Transaction<'_>,
    mutation: &RowMutation,
    position: &positions::Position,
    binding: &Arc<EditBinding>,
    confirm: &(impl Fn(Arc<EditBinding>) -> F + Sync),
) -> Result<u64, Error> {
    use super::roles_schema_objects::import_input::{input_types, parameter_source, record_input};
    use super::text_param::TextParam;
    use futures_util::{TryStreamExt, pin_mut};
    use tokio_postgres::types::Type;
    let relation = &binding.relation;
    let table = positions::target(relation);
    let mut values = Vec::new();
    let mut types = Vec::new();
    let command = match mutation.kind {
        MutationKind::Delete => format!("DELETE FROM {table}"),
        MutationKind::Update => {
            let mut mapped = Vec::new();
            let mut names = Vec::new();
            for (name, value) in &mutation.set {
                let attr = binding
                    .table
                    .columns
                    .iter()
                    .find(|a| a.name == *name)
                    .ok_or_else(|| Error::QueryFailed("Edited column changed.".into()))?;
                mapped.push(attr);
                names.push(quote_ident(name));
                values.push(TextParam(value.as_deref()));
            }
            types = input_types(&mapped);
            let row_type = format!(
                "{}.{}",
                quote_ident(&relation.schema),
                quote_ident(&relation.table)
            );
            let input = record_input(
                &row_type,
                &binding.table,
                &mapped,
                &parameter_source(mapped.len()),
            );
            format!("UPDATE {table} SET ({}) = ({input})", names.join(", "))
        }
    };
    let sql = format!(
        "{command} WHERE CURRENT OF {} RETURNING {}",
        quote_ident(&position.cursor),
        quote_ident(&mutation.pk[0].0)
    );
    let stmt = tx.prepare_typed(&sql, &types).await.map_err(query_failed)?;
    if !matches!(stmt.columns(),[c] if c.table_oid()==Some(relation.oid) && c.column_id()==relation.primary_key.first().copied())
        || !stmt
            .params()
            .iter()
            .map(Type::oid)
            .eq(types.iter().map(Type::oid))
    {
        return Err(Error::QueryFailed(
            "Mutation target or assignment type changed. Run the query again.".into(),
        ));
    }
    let params = values
        .iter()
        .map(|v| v as &(dyn ToSql + Sync))
        .collect::<Vec<_>>();
    let portal = tx.bind(&stmt, &params).await.map_err(query_failed)?;
    if !confirm(binding.clone()).await? {
        return Err(Error::QueryFailed(
            "Mutation target or assignment type changed. Run the query again.".into(),
        ));
    }
    let result = tx
        .query_portal_raw(&portal, 0)
        .await
        .map_err(query_failed)?;
    pin_mut!(result);
    while result
        .as_mut()
        .try_next()
        .await
        .map_err(query_failed)?
        .is_some()
    {}
    Ok(result.rows_affected().unwrap_or(0))
}

impl MutationReport {
    fn record(&mut self, mutation: &RowMutation, affected: u64) -> Result<(), Error> {
        if affected != 1 {
            return Err(cardinality_error(mutation, affected));
        }
        match mutation.kind {
            MutationKind::Update => self.updated += 1,
            MutationKind::Delete => self.deleted += 1,
        }
        Ok(())
    }
}

fn cardinality_error(mutation: &RowMutation, affected: u64) -> Error {
    let key = describe_key(&mutation.pk);
    let action = match mutation.kind {
        MutationKind::Update => "update",
        MutationKind::Delete => "delete",
    };
    Error::QueryFailed(format!(
        "Expected 1 row for {action} but matched {affected}. Row key: {key}. Nothing was changed — refresh the results and try again."
    ))
}

mod positions;
