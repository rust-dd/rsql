use super::PropertyText;
use rsql_core::postgres::metadata_schema::{IndexEntry, IndexMetadata, VisibleColumn};

/// Stands in for a column without a default; PostgreSQL never renders an expression this way.
pub const NO_DEFAULT: &str = "—";

pub struct ColumnRow {
    pub number: i16,
    pub name: String,
    pub ty: String,
    pub nullable: bool,
    pub primary: bool,
    pub default: PropertyText,
}

pub struct IndexRow {
    pub oid: u32,
    pub relation_oid: u32,
    pub name: String,
    pub primary: bool,
    pub unique: bool,
    pub method: String,
    pub entries: Vec<IndexEntry>,
    pub columns: PropertyText,
    pub definition: PropertyText,
}

impl IndexRow {
    pub fn role(&self) -> &'static str {
        if self.primary {
            "PRIMARY"
        } else if self.unique {
            "UNIQUE"
        } else {
            "INDEX"
        }
    }
}

impl From<IndexMetadata> for IndexRow {
    fn from(i: IndexMetadata) -> Self {
        let columns = i
            .entries
            .iter()
            .map(|entry| {
                if entry.included {
                    format!("INCLUDE ({})", entry.definition)
                } else {
                    entry.definition.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
            .into();
        Self {
            oid: i.oid,
            relation_oid: i.relation_oid,
            name: i.name,
            primary: i.primary,
            unique: i.unique,
            method: i.method,
            entries: i.entries,
            columns,
            definition: i.definition.into(),
        }
    }
}

impl From<VisibleColumn> for ColumnRow {
    fn from(c: VisibleColumn) -> Self {
        let default = if let Some(mode) = c.identity_generation {
            format!("GENERATED {mode} AS IDENTITY")
        } else if let Some(expr) = c.generation_expression {
            format!("GENERATED ({expr})")
        } else {
            c.default_value.unwrap_or_else(|| NO_DEFAULT.into())
        };
        Self {
            number: c.number,
            name: c.name,
            ty: c.formatted_type,
            nullable: c.nullable,
            primary: c.primary,
            default: default.into(),
        }
    }
}
pub fn rule_line((name, event): (String, String)) -> String {
    format!("{name} · {}", rule_event(&event))
}

pub(super) fn rule_event(event: &str) -> &str {
    match event {
        "1" => "SELECT",
        "2" => "UPDATE",
        "3" => "INSERT",
        "4" => "DELETE",
        event => event,
    }
}
pub fn trigger_line((name, event, timing): (String, String, String)) -> String {
    format!("{name} · {timing} {event}")
}
pub fn policy_line((name, mode, command): (String, String, String)) -> String {
    format!("{name} · {mode} · {command}")
}
