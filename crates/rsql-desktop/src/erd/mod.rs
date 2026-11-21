mod assemble;
mod layout;
mod load;
mod owner;
mod state;
pub mod svg;
mod worker;

pub use layout::{COLUMN_HEIGHT, HEADER_HEIGHT, Point, TableBox, layout};
pub use owner::{DiagramDelivery, DiagramLoad, DiagramOwner, LoadStatus};
pub use state::{DiagramState, EdgeGeometry};

use rsql_core::postgres::metadata_schema::{
    AttributeIdentity, ConstraintMetadata, RelationIdentity, VisibleColumn,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TableId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ColumnId {
    pub table: TableId,
    pub number: i16,
}

impl From<&AttributeIdentity> for ColumnId {
    fn from(value: &AttributeIdentity) -> Self {
        Self {
            table: TableId(value.relation_oid),
            number: value.number,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EdgeId {
    pub constraint: u32,
    pub pair: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub id: EdgeId,
    pub source: ColumnId,
    pub target: ColumnId,
}

#[derive(Debug)]
pub struct Column {
    pub id: ColumnId,
    pub name: String,
    pub ty: String,
    pub nullable: bool,
    pub primary: bool,
    pub foreign: bool,
}

impl Column {
    pub fn key_label(&self) -> Option<&'static str> {
        if self.primary {
            Some("PK")
        } else if self.foreign {
            Some("FK")
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct Table {
    pub identity: RelationIdentity,
    pub columns: Vec<Column>,
}

#[derive(Default, Debug)]
pub struct Connections {
    pub neighbors: HashSet<TableId>,
    pub edges: HashSet<EdgeId>,
}

pub struct TableMetadata {
    pub relation: RelationIdentity,
    pub columns: Result<Vec<VisibleColumn>, String>,
    pub constraints: Result<Vec<ConstraintMetadata>, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Columns,
    Constraints,
    Visibility,
}

#[derive(Debug, Clone)]
pub struct TableFailure {
    pub table: RelationIdentity,
    pub section: Section,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaIdentity {
    pub oid: u32,
    pub name: String,
}

#[derive(Debug)]
pub struct SchemaGraph {
    schema: SchemaIdentity,
    tables: Vec<Table>,
    table_index: HashMap<TableId, usize>,
    column_index: HashMap<ColumnId, usize>,
    edges: Vec<Edge>,
    edge_index: HashMap<EdgeId, usize>,
    constraint_names: HashMap<u32, String>,
    connections: HashMap<TableId, Connections>,
    failures: std::sync::Arc<[TableFailure]>,
}

impl SchemaGraph {
    pub fn from_metadata(
        schema: SchemaIdentity,
        metadata: Vec<TableMetadata>,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut builder = assemble::Builder::new(schema);
        for table in metadata {
            builder.add(table, &mut check)?;
        }
        builder.finish(check)
    }

    fn visible(&self, attr: &AttributeIdentity) -> bool {
        let id = ColumnId::from(attr);
        self.column(id)
            .is_some_and(|column| column.name == attr.name)
    }

    pub fn schema(&self) -> &SchemaIdentity {
        &self.schema
    }
    pub fn tables(&self) -> &[Table] {
        &self.tables
    }
    pub fn table(&self, id: TableId) -> Option<&Table> {
        self.table_index.get(&id).map(|&i| &self.tables[i])
    }
    pub fn column(&self, id: ColumnId) -> Option<&Column> {
        self.column_index
            .get(&id)
            .map(|&row| &self.tables[self.table_index[&id.table]].columns[row])
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }
    pub fn connections(&self, id: TableId) -> Option<&Connections> {
        self.connections.get(&id)
    }
    pub fn failures(&self) -> &[TableFailure] {
        &self.failures
    }
}

pub fn table_title(name: &str) -> String {
    if name.encode_utf16().count() <= 28 {
        return name.into();
    }
    let mut length = 0;
    let mut title: String = name
        .chars()
        .take_while(|c| {
            length += c.len_utf16();
            length <= 26
        })
        .collect();
    title.push_str("..");
    title
}
