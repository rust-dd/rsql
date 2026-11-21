//! One-shot catalog snapshot of a schema, for the editor's language features.
//!
//! Completion used to ask the server per table and per schema while the user
//! typed, which put an IPC round trip on the keystroke path. The index is
//! fetched once per schema instead, so completion reads memory.

use serde::Serialize;
use tokio_postgres::Client;

use crate::error::{Error, query_failed};

/// Columns beyond this are not sent; the frontend falls back to loading a
/// table's columns on demand. Keeps a very wide schema from stalling the load.
pub const COLUMN_CAP: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RelationKind {
    Table,
    View,
    MaterializedView,
}

impl RelationKind {
    /// `relkind` values from `pg_class`.
    fn from_relkind(value: &str) -> Option<Self> {
        match value {
            "r" | "p" | "f" => Some(Self::Table),
            "v" => Some(Self::View),
            "m" => Some(Self::MaterializedView),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexedColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub is_primary_key: bool,
    /// Target of a single-column foreign key: (schema, table, column).
    pub references: Option<(String, String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexedRelation {
    #[serde(skip)]
    pub identity: Option<super::metadata_schema::RelationIdentity>,
    pub name: String,
    pub kind: RelationKind,
    pub comment: Option<String>,
    pub columns: Vec<IndexedColumn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineKind {
    Function,
    Aggregate,
    Window,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexedFunction {
    pub name: String,
    #[serde(skip)]
    pub kind: RoutineKind,
    #[serde(skip)]
    pub identity: Option<super::metadata_views_functions::RoutineIdentity>,
    pub signature: String,
    pub return_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaIndex {
    pub schema: String,
    pub relations: Vec<IndexedRelation>,
    pub functions: Vec<IndexedFunction>,
    /// Columns were omitted because the schema exceeds `COLUMN_CAP`.
    pub truncated: bool,
}

const RELATION_SQL: &str = include_str!("sql/relations.sql");

/// Columns with their type, nullability, default, primary-key flag and, for
/// single-column foreign keys, the target. One row per column of the schema.
const COLUMN_SQL: &str = include_str!("sql/columns.sql");

const FUNCTION_SQL: &str = include_str!("sql/functions.sql");

pub async fn load_schema_index(client: &Client, schema: &str) -> Result<SchemaIndex, Error> {
    let relation_rows = client
        .query(RELATION_SQL, &[&schema])
        .await
        .map_err(query_failed)?;

    let mut relations: Vec<IndexedRelation> = relation_rows
        .iter()
        .filter_map(|row| {
            let kind = RelationKind::from_relkind(row.get::<_, String>(1).as_str())?;
            Some(IndexedRelation {
                identity: Some(super::metadata_schema::RelationIdentity {
                    oid: row.get(3),
                    schema: schema.into(),
                    name: row.get(0),
                    kind: super::metadata_schema::RelationKind::from_catalog(row.get(1)).ok()?,
                }),
                name: row.get(0),
                kind,
                comment: row.get(2),
                columns: Vec::new(),
            })
        })
        .collect();

    // One extra row is the signal that the schema is wider than the cap, which
    // avoids a separate counting query.
    let limit = COLUMN_CAP as i64 + 1;
    let column_rows = client
        .query(COLUMN_SQL, &[&schema, &limit])
        .await
        .map_err(query_failed)?;

    let truncated = column_rows.len() > COLUMN_CAP;
    if !truncated {
        for row in &column_rows {
            let relation_name: String = row.get(0);
            let Some(relation) = relations.iter_mut().find(|r| r.name == relation_name) else {
                continue;
            };
            let target_schema: Option<String> = row.get(6);
            let target_table: Option<String> = row.get(7);
            let target_column: Option<String> = row.get(8);
            relation.columns.push(IndexedColumn {
                name: row.get(1),
                data_type: row.get(2),
                nullable: row.get(3),
                default_value: row.get(4),
                is_primary_key: row.get(5),
                references: match (target_schema, target_table, target_column) {
                    (Some(s), Some(t), Some(c)) => Some((s, t, c)),
                    _ => None,
                },
            });
        }
    }

    let function_rows = client
        .query(FUNCTION_SQL, &[&schema])
        .await
        .map_err(query_failed)?;

    let functions = function_rows
        .iter()
        .map(|row| IndexedFunction {
            name: row.get(0),
            kind: match row.get::<_, &str>(3) {
                "a" => RoutineKind::Aggregate,
                "w" => RoutineKind::Window,
                _ => RoutineKind::Function,
            },
            identity: match row.get::<_, &str>(3) {
                "f" | "w" => Some(super::metadata_views_functions::RoutineIdentity {
                    oid: row.get(4),
                    schema: schema.into(),
                    name: row.get(0),
                    kind: if row.get::<_, &str>(3) == "w" {
                        super::metadata_views_functions::RoutineKind::Window
                    } else {
                        super::metadata_views_functions::RoutineKind::Function
                    },
                    argument_types: row.get(5),
                }),
                _ => None,
            },
            signature: row.get(1),
            return_type: row.get(2),
        })
        .collect();

    Ok(SchemaIndex {
        schema: schema.to_string(),
        relations,
        functions,
        truncated,
    })
}
