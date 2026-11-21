//! A project-scoped structure snapshot and a pure, reversible draft of its DDL.

mod columns;
mod constraints;
mod dependencies;
mod fragments;
mod sql;
mod type_edits;
mod validation;

pub use columns::{Column, PG_COMMON_TYPES};
pub use constraints::{Action, ForeignKey, ForeignTarget, KeyColumns, NewObject, StoredObject};
pub use type_edits::TypeTargets;

pub use object::{ObjectKind, ObjectRef};
mod object;
mod plan;
mod rows;
use crate::postgres::metadata_schema::StructureMetadata;
pub(crate) use plan::DroppedObject;
pub use plan::{ForeignChoices, StructurePlan};
pub use rows::DraftRows;
use std::{collections::BTreeSet, fmt, sync::Arc};

#[derive(Clone, Debug)]
pub struct StructureSnapshot {
    object: ObjectRef,
    metadata: StructureMetadata,
}

impl StructureSnapshot {
    pub fn new(project: &str, metadata: StructureMetadata) -> Self {
        Self {
            object: ObjectRef::relation(project, metadata.relation.clone()),
            metadata,
        }
    }

    pub fn object(&self) -> &ObjectRef {
        &self.object
    }

    pub fn metadata(&self) -> &StructureMetadata {
        &self.metadata
    }

    pub fn draft(self) -> StructureDraft {
        StructureDraft::from_snapshot(Arc::new(self), 0)
    }
}

/// Catalog IDs are scoped to this draft's target. New IDs survive edits and are never recycled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EditId {
    Attribute(i16),
    Constraint(u32),
    Index(u32),
    New(u64),
}

#[derive(Clone, Debug)]
pub struct Entry<T> {
    id: EditId,
    original: Option<Arc<T>>,
    current: Option<Arc<T>>,
}

impl<T: Clone> Entry<T> {
    fn existing(id: EditId, value: T) -> Self {
        let value = Arc::new(value);
        Self {
            id,
            original: Some(value.clone()),
            current: Some(value),
        }
    }

    fn added(id: EditId, value: T) -> Self {
        Self {
            id,
            original: None,
            current: Some(Arc::new(value)),
        }
    }

    pub fn id(&self) -> EditId {
        self.id
    }
    pub fn original(&self) -> Option<&T> {
        self.original.as_deref()
    }
    pub fn current(&self) -> Option<&T> {
        self.current.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError(String);

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ValidationError {}

type ReviewCheck<'a> = &'a dyn Fn() -> Result<(), ValidationError>;

#[derive(Clone, Debug)]
pub struct StructureDraft {
    snapshot: Arc<StructureSnapshot>,
    baseline: Arc<Baseline>,
    /// Catalog rows stay in snapshot order. Added columns follow them.
    columns: DraftRows<Entry<Column>>,
    primary: Option<Entry<KeyColumns>>,
    additions: DraftRows<(EditId, NewObject)>,
    removed: Arc<BTreeSet<EditId>>,
    next_id: u64,
}

#[derive(Debug)]
struct Baseline {
    columns: DraftRows<Entry<Column>>,
    primary: Option<Entry<KeyColumns>>,
}

impl StructureDraft {
    fn from_snapshot(snapshot: Arc<StructureSnapshot>, next_id: u64) -> Self {
        let metadata = snapshot.metadata();
        let columns = metadata
            .columns
            .iter()
            .map(|c| Entry::existing(EditId::Attribute(c.identity.number), Column::from(c)))
            .collect::<DraftRows<_>>();
        let primary = metadata
            .constraints
            .iter()
            .find(|c| c.kind == "p")
            .map(|c| Entry::existing(EditId::Constraint(c.identity.oid), KeyColumns::from(c)));
        Self {
            snapshot,
            baseline: Arc::new(Baseline {
                columns: columns.clone(),
                primary: primary.clone(),
            }),
            columns,
            primary,
            next_id,
            additions: Default::default(),
            removed: Default::default(),
        }
    }

    pub fn snapshot(&self) -> &StructureSnapshot {
        &self.snapshot
    }

    fn allocate(&mut self) -> EditId {
        let id = EditId::New(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn discard(&mut self) {
        self.columns = self.baseline.columns.clone();
        self.primary = self.baseline.primary.clone();
        self.additions = Default::default();
        self.removed = Default::default();
    }

    pub fn drop_entry(&mut self, id: EditId) -> Result<(), ValidationError> {
        let column = self.columns.iter().position(|c| c.id == id);
        let addition = self.additions.iter().position(|(key, _)| *key == id);
        if let Some(i) = column {
            if self.columns[i].original.is_none() {
                self.columns.remove(i);
            } else {
                self.columns.edit(i).current = None;
            }
        } else if let Some(key) = self.primary.as_mut().filter(|key| key.id == id) {
            if key.original.is_some() {
                key.current = None;
            } else {
                self.primary = None;
            }
        } else if let Some(i) = addition {
            self.additions.remove(i);
        } else if self.snapshot.stored_objects().any(|o| o.id() == id) {
            Arc::make_mut(&mut self.removed).insert(id);
        } else {
            return Err(ValidationError(format!("Unknown structure entry {id:?}.")));
        }
        Ok(())
    }

    /// Restore catalog values even when other fields were edited before the drop.
    pub fn restore(&mut self, id: EditId) -> Result<(), ValidationError> {
        let index = self
            .columns
            .iter()
            .position(|c| c.id == id && c.original.is_some());
        if let Some(index) = index {
            let col = self.columns.edit(index);
            col.current.clone_from(&col.original);
        } else if let Some(key) = self
            .primary
            .as_mut()
            .filter(|key| key.id == id && key.original.is_some())
        {
            key.current.clone_from(&key.original);
        } else if self.snapshot.stored_objects().any(|o| o.id() == id) {
            Arc::make_mut(&mut self.removed).remove(&id);
        } else {
            return Err(ValidationError(format!(
                "No catalog entry to restore for {id:?}."
            )));
        }
        Ok(())
    }

    pub fn change_count(&self) -> usize {
        let primary = self.primary.as_ref().map_or(0, |p| {
            if p.original == p.current {
                0
            } else {
                usize::from(p.original.is_some()) + usize::from(p.current.is_some())
            }
        });
        self.columns
            .iter()
            .map(Entry::column_changes)
            .sum::<usize>()
            + primary
            + self.additions.len()
            + self.removed.len()
    }

    pub fn review(&self) -> Result<Vec<String>, ValidationError> {
        Ok(self.review_plan()?.statements)
    }
}
