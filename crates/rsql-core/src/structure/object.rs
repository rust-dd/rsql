use crate::postgres::{
    metadata_schema::{RelationIdentity, RelationKind},
    metadata_views_functions::{RoutineIdentity, RoutineKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectKind {
    Table,
    PartitionedTable,
    ForeignTable,
    View,
    MaterializedView,
    Function,
    Procedure,
    Window,
}

impl ObjectKind {
    pub fn is_table(self) -> bool {
        matches!(
            self,
            Self::Table | Self::PartitionedTable | Self::ForeignTable
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Table => "Table",
            Self::PartitionedTable => "Partitioned table",
            Self::ForeignTable => "Foreign table",
            Self::View => "View",
            Self::MaterializedView => "Materialized view",
            Self::Function => "Function",
            Self::Procedure => "Procedure",
            Self::Window => "Window function",
        }
    }
}

/// A project-scoped server identity, independent of catalog display labels.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectRef {
    pub project: String,
    pub schema: String,
    pub name: String,
    pub kind: ObjectKind,
    pub oid: u32,
    pub routine_identity: Option<RoutineIdentity>,
}

impl ObjectRef {
    pub fn relation(project: &str, relation: RelationIdentity) -> Self {
        Self {
            project: project.into(),
            schema: relation.schema,
            name: relation.name,
            kind: match relation.kind {
                RelationKind::Table => ObjectKind::Table,
                RelationKind::PartitionedTable => ObjectKind::PartitionedTable,
                RelationKind::ForeignTable => ObjectKind::ForeignTable,
                RelationKind::View => ObjectKind::View,
                RelationKind::MaterializedView => ObjectKind::MaterializedView,
            },
            oid: relation.oid,
            routine_identity: None,
        }
    }

    pub fn routine(project: &str, identity: RoutineIdentity) -> Self {
        Self {
            project: project.into(),
            schema: identity.schema.clone(),
            name: identity.name.clone(),
            kind: match identity.kind {
                RoutineKind::Function => ObjectKind::Function,
                RoutineKind::Procedure => ObjectKind::Procedure,
                RoutineKind::Window => ObjectKind::Window,
            },
            oid: identity.oid,
            routine_identity: Some(identity),
        }
    }
}
