use super::{EditId, Entry, StructureDraft, StructureSnapshot, ValidationError};
use crate::postgres::metadata_schema::{ConstraintMetadata, IndexMetadata};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyColumns {
    pub name: String,
    pub columns: Vec<EditId>,
}

impl From<&ConstraintMetadata> for KeyColumns {
    fn from(c: &ConstraintMetadata) -> Self {
        Self {
            name: c.identity.name.clone(),
            columns: c
                .columns
                .iter()
                .map(|a| EditId::Attribute(a.number))
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl Action {
    pub const ALL: [Self; 5] = [
        Self::NoAction,
        Self::Restrict,
        Self::Cascade,
        Self::SetNull,
        Self::SetDefault,
    ];

    pub fn sql(self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }
}

impl FromStr for Action {
    type Err = ValidationError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|a| a.sql() == value)
            .ok_or_else(|| ValidationError(format!("Unknown foreign key action {value:?}.")))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForeignTarget {
    ThisTable(Vec<EditId>),
    Relation {
        schema: String,
        table: String,
        columns: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignKey {
    pub key: KeyColumns,
    pub target: ForeignTarget,
    pub on_update: Action,
    pub on_delete: Action,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewObject {
    Unique(KeyColumns),
    Index { key: KeyColumns, unique: bool },
    ForeignKey(ForeignKey),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Group {
    Foreign,
    Unique,
    Index,
}

impl NewObject {
    pub fn key(&self) -> &KeyColumns {
        match self {
            Self::Unique(key) | Self::Index { key, .. } => key,
            Self::ForeignKey(fk) => &fk.key,
        }
    }

    pub(super) fn group(&self) -> Group {
        match self {
            Self::Unique(_) => Group::Unique,
            Self::Index { .. } => Group::Index,
            Self::ForeignKey(_) => Group::Foreign,
        }
    }
}

/// Existing non-primary constraints and indexes expose their full, immutable catalog metadata.
#[derive(Clone, Copy, Debug)]
pub enum StoredObject<'a> {
    ForeignKey(&'a ConstraintMetadata),
    Unique(&'a ConstraintMetadata),
    Index(&'a IndexMetadata),
}

impl<'a> StoredObject<'a> {
    pub fn id(self) -> EditId {
        match self {
            Self::ForeignKey(c) | Self::Unique(c) => EditId::Constraint(c.identity.oid),
            Self::Index(i) => EditId::Index(i.oid),
        }
    }
    pub fn name(self) -> &'a str {
        match self {
            Self::ForeignKey(c) | Self::Unique(c) => &c.identity.name,
            Self::Index(i) => &i.name,
        }
    }
}

impl StructureSnapshot {
    pub fn stored_objects(&self) -> impl Iterator<Item = StoredObject<'_>> {
        let meta = self.metadata();
        let constraints = meta
            .constraints
            .iter()
            .filter_map(|c| match c.kind.as_str() {
                "f" => Some(StoredObject::ForeignKey(c)),
                "u" => Some(StoredObject::Unique(c)),
                _ => None,
            });
        let indexes = meta.indexes.iter().filter(|i| {
            !i.primary
                && i.constraint.is_none()
                && !meta
                    .constraints
                    .iter()
                    .any(|c| c.backing_index_oid == Some(i.oid))
        });
        constraints.chain(indexes.map(StoredObject::Index))
    }
}

impl StructureDraft {
    pub fn primary_key(&self) -> Option<&Entry<KeyColumns>> {
        self.primary.as_ref()
    }

    pub fn set_primary_key(&mut self, key: KeyColumns) -> EditId {
        if let Some(entry) = &mut self.primary {
            entry.current = Some(std::sync::Arc::new(key));
            entry.id
        } else {
            let id = self.allocate();
            self.primary = Some(Entry::added(id, key));
            id
        }
    }

    pub fn add_object(&mut self, object: NewObject) -> EditId {
        let id = self.allocate();
        self.additions.push((id, object));
        id
    }

    pub fn added_objects(&self) -> &super::DraftRows<(EditId, NewObject)> {
        &self.additions
    }

    pub fn new_object_mut(&mut self, id: EditId) -> Option<&mut NewObject> {
        let index = self.additions.iter().position(|(i, _)| *i == id)?;
        Some(&mut self.additions.edit(index).1)
    }

    pub fn is_dropped(&self, id: EditId) -> bool {
        match id {
            EditId::Index(_) => self.removed.contains(&id),
            EditId::Constraint(_) => {
                self.removed.contains(&id)
                    || self
                        .primary
                        .as_ref()
                        .is_some_and(|pk| pk.id == id && pk.current.is_none())
            }
            EditId::Attribute(_) => self
                .columns
                .iter()
                .any(|col| col.id == id && col.current.is_none()),
            EditId::New(_) => false,
        }
    }
}
