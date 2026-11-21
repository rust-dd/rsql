//! Builds parameterized UPDATE and DELETE statements for grid row edits.
//!
//! Values arrive from the frontend as text, because that is how the simple
//! query protocol hands them out. Binding them as text and casting on the
//! Postgres side (`$1::text::<type>`) keeps every value out of the SQL string.
//! The double cast is deliberate: with a bare `$1::int4` Postgres infers the
//! parameter as `int4` while the client binds `&str`, which is a type mismatch.
//! `$1::text` pins the parameter to text and the second cast converts. The
//! compared column stays bare, so the expression folds to a constant at plan
//! time and indexes stay usable.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MutationKind {
    Update,
    Delete,
}

/// One row mutation. `None` values are SQL NULL.
#[derive(Debug, Clone, Deserialize)]
pub struct RowMutation {
    pub kind: MutationKind,
    #[serde(default)]
    pub set: Vec<(String, Option<String>)>,
    pub pk: Vec<(String, Option<String>)>,
}

/// A statement plus its bound parameters, in placeholder order.
#[derive(Debug, PartialEq, Eq)]
pub struct BuiltStatement {
    pub sql: String,
    pub params: Vec<Option<String>>,
}

/// Column name to the type expression returned by `format_type`.
pub type ColumnTypes = BTreeMap<String, String>;

pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Resolve a column's type, rejecting anything the table does not have. This is
/// what makes the statement injection-proof: names are matched against the live
/// catalog rather than merely quoted.
fn column_type<'a>(types: &'a ColumnTypes, column: &str) -> Result<&'a str, Error> {
    types
        .get(column)
        .map(String::as_str)
        .ok_or_else(|| Error::QueryFailed(format!("Unknown column \"{}\"", column)))
}

/// Append the key predicates and their parameters, returning the WHERE body.
fn build_key_predicates(
    key: &[(String, Option<String>)],
    types: &ColumnTypes,
    params: &mut Vec<Option<String>>,
) -> Result<String, Error> {
    if key.is_empty() {
        return Err(Error::QueryFailed(
            "Refusing to build a statement with no key columns".into(),
        ));
    }

    let mut predicates = Vec::with_capacity(key.len());
    for (column, value) in key {
        let ty = column_type(types, column)?;
        match value {
            None => predicates.push(format!("{} IS NULL", quote_ident(column))),
            Some(_) => {
                params.push(value.clone());
                predicates.push(format!(
                    "{} = ${}::text::{}",
                    quote_ident(column),
                    params.len(),
                    ty
                ));
            }
        }
    }

    Ok(predicates.join(" AND "))
}

pub fn build_statement(
    schema: &str,
    table: &str,
    mutation: &RowMutation,
    types: &ColumnTypes,
) -> Result<BuiltStatement, Error> {
    let target = format!("{}.{}", quote_ident(schema), quote_ident(table));
    let mut params = Vec::new();
    let command = match mutation.kind {
        MutationKind::Delete => format!("DELETE FROM {target}"),
        MutationKind::Update => {
            if mutation.set.is_empty() {
                return Err(Error::QueryFailed(
                    "Refusing to build an UPDATE with no assignments".into(),
                ));
            }
            let mut assignments = Vec::with_capacity(mutation.set.len());
            for (column, value) in &mutation.set {
                let ty = column_type(types, column)?;
                params.push(value.clone());
                assignments.push(format!(
                    "{} = ${}::text::{ty}",
                    quote_ident(column),
                    params.len()
                ));
            }
            format!("UPDATE {target} SET {}", assignments.join(", "))
        }
    };
    let predicate = build_key_predicates(&mutation.pk, types, &mut params)?;
    let sql = format!("{command} WHERE {predicate}");
    Ok(BuiltStatement { sql, params })
}
