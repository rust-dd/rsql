use super::{ImportColumn, ImportTable};
use crate::{
    Error,
    error::{error_chain, query_failed},
    mutation::quote_ident,
};
use tokio_postgres::{Statement, Transaction, types::Type};

/// The same column-level decision is captured for the native route and aggregated
/// for the legacy COPY command. A mapping uses COPY input only if all columns do.
pub(super) const COPY_COLUMN: &str = "NOT pg_catalog.row_security_active(c.oid)
    AND c.relkind IN ('r','p') AND NOT c.relhasrules AND a.attidentity <> 'a'";

pub(super) async fn prepare_native_insert(
    tx: &Transaction<'_>,
    target: &ImportTable,
    mapped: &[&ImportColumn],
) -> Result<Statement, Error> {
    let rel = format!(
        "{}.{}",
        quote_ident(&target.schema),
        quote_ident(&target.name)
    );
    let columns = mapped
        .iter()
        .map(|c| quote_ident(&c.name))
        .collect::<Vec<_>>()
        .join(", ");
    let copy_input = mapped.iter().all(|column| column.copy_input);
    let types = if copy_input {
        input_types(mapped)
    } else {
        vec![Type::TEXT; mapped.len()]
    };
    let source = parameter_source(mapped.len());
    let input = if copy_input {
        record_input(&rel, target, mapped, &source)
    } else {
        let casts = fallback_types(tx, target, mapped)
            .await?
            .into_iter()
            .enumerate()
            .map(|(i, ty)| format!("csv.v{i}::{ty}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("SELECT {casts} FROM {source}")
    };
    let sql = format!("INSERT INTO {rel} ({columns}) {input}");
    let statement = tx.prepare_typed(&sql, &types).await.map_err(|e| {
        Error::QueryFailed(format!("Failed to prepare statement: {}", error_chain(&e)))
    })?;
    if !statement
        .params()
        .iter()
        .map(Type::oid)
        .eq(types.iter().map(Type::oid))
    {
        return Err(Error::QueryFailed(
            "Import parameter types changed. Reopen the table import.".into(),
        ));
    }
    Ok(statement)
}

pub(in crate::postgres) fn input_types(mapped: &[&ImportColumn]) -> Vec<Type> {
    mapped
        .iter()
        .map(|column| {
            if column.bind_domain {
                // Parse consumes only the OID. Describe returns the server's full parameter types.
                crate::postgres::text_param::captured_type(column.type_oid)
            } else {
                Type::TEXT
            }
        })
        .collect()
}

pub(in crate::postgres) fn parameter_source(count: usize) -> String {
    let params = (0..count)
        .map(|i| format!("${} AS v{i}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    // OFFSET defers SQL conversion expressions to execution. Typed domain parameters use input during Bind.
    format!("(SELECT {params} OFFSET 0) AS csv")
}

pub(in crate::postgres) fn record_input(
    rel: &str,
    target: &ImportTable,
    mapped: &[&ImportColumn],
    source: &str,
) -> String {
    let mut output = Vec::new();
    for (i, col) in mapped.iter().enumerate() {
        let value = format!("csv.v{i}");
        let name = quote_ident(&col.name);
        let field = format!("{value} AS {name}");
        let expr = if col.bind_domain {
            value
        } else if let Some(input) = json_input(col) {
            let json = format!("pg_catalog.{input}({value}::pg_catalog.cstring)");
            if col.type_oid == col.base_oid {
                json
            } else {
                domain_field(target, rel, col, &json)
            }
        } else {
            format!("({}).{name}", populate_fields(rel, &field))
        };
        output.push(expr);
    }
    ordered_fields(source, &output)
}

// A cast to the captured row type coerces just this value to its actual domain.
fn domain_field(target: &ImportTable, rel: &str, column: &ImportColumn, value: &str) -> String {
    let fields = target
        .columns
        .iter()
        .map(|attr| {
            if attr.position == column.position {
                value.to_owned()
            } else {
                format!("(NULL::{rel}).{}", quote_ident(&attr.name))
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("(ROW({fields})::{rel}).{}", quote_ident(&column.name))
}

// Each checked field reads its predecessor. INSERT can compute defaults only after
// the last field has a row, and reading an earlier result cannot repeat its input.
fn ordered_fields(source: &str, expressions: &[String]) -> String {
    let mut steps = vec![format!("raw AS MATERIALIZED (SELECT * FROM {source})")];
    let mut from = "raw AS csv".to_owned();
    let mut output = Vec::new();
    for (i, expr) in expressions.iter().enumerate() {
        steps.push(format!(
            "f{i} AS MATERIALIZED (SELECT {expr} AS v FROM {from})"
        ));
        from = format!("raw AS csv CROSS JOIN f{i}");
        output.push(format!("(SELECT v FROM f{i})"));
    }
    let last = expressions.len() - 1;
    format!(
        "WITH {} SELECT {} FROM f{last}",
        steps.join(", "),
        output.join(", ")
    )
}

fn json_input(column: &ImportColumn) -> Option<&'static str> {
    match column.base_oid {
        oid if oid == Type::JSON.oid() => Some("json_in"),
        oid if oid == Type::JSONB.oid() => Some("jsonb_in"),
        _ => None,
    }
}

fn populate_fields(rel: &str, fields: &str) -> String {
    // Typed NULLs skip unmapped attributes whose domains may require a default.
    format!(
        "pg_catalog.json_populate_record(ROW((NULL::{rel}).*)::{rel},
        (SELECT pg_catalog.row_to_json(fields.*) FROM (SELECT {fields}) AS fields))"
    )
}

async fn fallback_types(
    tx: &Transaction<'_>,
    target: &ImportTable,
    mapped: &[&ImportColumn],
) -> Result<Vec<String>, Error> {
    let positions = mapped.iter().map(|c| c.position).collect::<Vec<_>>();
    let rows = tx
        .query(
            "SELECT pg_catalog.format_type(t.oid,a.atttypmod),
            pg_catalog.format_type(named.oid,NULL), pg_catalog.quote_ident(named.typname),
            pg_catalog.format('%I.%I',n.nspname,named.typname)
        FROM pg_catalog.unnest($2::pg_catalog.int2[]) WITH ORDINALITY AS m(pos,ord)
        JOIN pg_catalog.pg_attribute a ON a.attrelid=$1 AND a.attnum=m.pos
        JOIN pg_catalog.pg_type t ON t.oid=a.atttypid
        LEFT JOIN pg_catalog.pg_type e ON e.oid=t.typelem AND e.typarray=t.oid
        JOIN pg_catalog.pg_type named ON named.oid=COALESCE(e.oid,t.oid)
        JOIN pg_catalog.pg_namespace n ON n.oid=named.typnamespace ORDER BY m.ord",
            &[&target.oid, &positions],
        )
        .await
        .map_err(query_failed)?;
    Ok(rows
        .iter()
        .map(|r| {
            let formatted: String = r.get(0);
            let base: &str = r.get(1);
            let name: &str = r.get(2);
            let qualified: &str = r.get(3);
            if (base == name || formatted == name || formatted.strip_suffix("[]") == Some(name))
                && let Some(suffix) = formatted.strip_prefix(name)
            {
                return format!("{qualified}{suffix}");
            }
            formatted
        })
        .collect())
}
