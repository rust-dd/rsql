use super::{EditId, Entry, StructureDraft};
use crate::postgres::metadata_schema::ColumnMetadata;

pub const PG_COMMON_TYPES: &[&str] = &[
    "integer",
    "bigint",
    "smallint",
    "serial",
    "bigserial",
    "text",
    "varchar",
    "char",
    "boolean",
    "date",
    "timestamp",
    "timestamptz",
    "time",
    "timetz",
    "interval",
    "numeric",
    "real",
    "double precision",
    "money",
    "uuid",
    "json",
    "jsonb",
    "bytea",
    "inet",
    "cidr",
    "macaddr",
    "point",
    "line",
    "polygon",
    "box",
    "xml",
    "tsquery",
    "tsvector",
    "int[]",
    "text[]",
    "jsonb[]",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<std::sync::Arc<str>>,
}

impl Column {
    pub(super) fn removes_default_before_cast(&self, old: &Column) -> bool {
        old.data_type != self.data_type && old.default.is_some() && old.default != self.default
    }
}

impl From<&ColumnMetadata> for Column {
    fn from(column: &ColumnMetadata) -> Self {
        Self {
            name: column.identity.name.clone(),
            data_type: column.formatted_type.clone(),
            nullable: column.nullable,
            default: column.default_value.as_deref().map(Into::into),
        }
    }
}

impl Entry<Column> {
    pub(super) fn column_changes(&self) -> usize {
        match (&self.original, &self.current) {
            (Some(old), Some(new)) => {
                usize::from(old.name != new.name)
                    + usize::from(old.data_type != new.data_type)
                    + usize::from(old.nullable != new.nullable)
                    + usize::from(old.default != new.default)
                    + usize::from(new.removes_default_before_cast(old) && new.default.is_some())
            }
            (None, None) => 0,
            _ => 1,
        }
    }
}

impl StructureDraft {
    pub fn columns(&self) -> &super::DraftRows<Entry<Column>> {
        &self.columns
    }

    pub fn add_column(&mut self, column: Column) -> EditId {
        let id = self.allocate();
        self.columns.push(Entry::added(id, column));
        id
    }

    pub fn column_mut(&mut self, id: EditId) -> Option<&mut Column> {
        let index = self.columns.iter().position(|c| c.id == id)?;
        self.columns
            .edit(index)
            .current
            .as_mut()
            .map(std::sync::Arc::make_mut)
    }
}
