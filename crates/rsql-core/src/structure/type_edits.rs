use super::{Column, StructureDraft, StructurePlan, ValidationError};
use crate::{Error, error::query_failed, postgres::metadata_schema::ColumnCollation};
use std::{collections::BTreeMap, sync::Arc};
use tokio_postgres::Client;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TypeTarget {
    oid: u32,
    collation: u32,
    pub domain_not_null: bool,
}

#[derive(Debug, Default)]
pub struct TypeTargets(BTreeMap<String, TypeTarget>);

impl TypeTargets {
    pub async fn load(client: &Client, names: &[&str]) -> Result<Self, Error> {
        if names.is_empty() {
            return Ok(Self::default());
        }
        let rows = client.query(
            "SELECT t.oid, t.typcollation, EXISTS (WITH RECURSIVE bases AS (SELECT t.typnotnull, t.typbasetype
                  UNION ALL SELECT b.typnotnull, b.typbasetype FROM bases JOIN pg_catalog.pg_type b ON b.oid = bases.typbasetype)
                  SELECT FROM bases WHERE typnotnull)
             FROM pg_catalog.unnest($1::text[]) WITH ORDINALITY input(name, pos)
             LEFT JOIN pg_catalog.pg_type t ON t.oid = pg_catalog.to_regtype(input.name) ORDER BY input.pos", &[&names]
        ).await.map_err(query_failed)?;
        let mut targets = BTreeMap::new();
        for (name, row) in names.iter().zip(rows) {
            let Some(oid) = row.get::<_, Option<u32>>(0) else {
                return Err(Error::QueryFailed(format!("Type {name:?} is unavailable.")));
            };
            targets.insert(
                (*name).to_owned(),
                TypeTarget {
                    oid,
                    collation: row.get(1),
                    domain_not_null: row.get(2),
                },
            );
        }
        Ok(Self(targets))
    }

    pub(super) fn get(&self, name: &str) -> Option<TypeTarget> {
        self.0.get(name).copied()
    }
}

impl StructureDraft {
    pub fn type_edit_names(&self) -> Vec<&str> {
        self.columns
            .iter()
            .take(self.snapshot.metadata().columns.len())
            .filter_map(|col| {
                let (old, new) = col.original().zip(col.current())?;
                (!std::ptr::eq(old, new) && old.data_type != new.data_type)
                    .then_some(new.data_type.as_str())
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(super) struct TypeEdit {
    pub column: Arc<Column>,
    pub collation: Option<ColumnCollation>,
    pub target: Option<TypeTarget>,
}

impl TypeEdit {
    pub(super) fn validate(&self, target: TypeTarget) -> Result<(), ValidationError> {
        let column = &self.column;
        if column.nullable && target.domain_not_null {
            return Err(ValidationError(format!(
                "Column {:?} cannot be nullable with the NOT NULL domain {:?}.",
                column.name, column.data_type
            )));
        }
        if let Some(coll) = &self.collation
            && ((coll.preserve_explicitly && target.collation == 0)
                || (!coll.preserve_explicitly
                    && target.collation != 0
                    && target.collation != coll.oid))
        {
            return Err(ValidationError(format!(
                "Changing column {:?} to {:?} would lose its collation {}.{}. Choose a compatible type or explicitly change the collation in SQL.",
                column.name, column.data_type, coll.schema, coll.name
            )));
        }
        Ok(())
    }
}

impl StructurePlan {
    pub fn has_type_edits(&self) -> bool {
        !self.type_edits.is_empty()
    }

    pub(crate) fn require_reviewed_type_targets(&self) -> Result<(), Error> {
        if let Some(edit) = self.type_edits.iter().find(|edit| edit.target.is_none()) {
            return Err(Error::QueryFailed(format!(
                "Resolve the destination type for column {:?} and review the changes before applying.",
                edit.column.name
            )));
        }
        Ok(())
    }

    /// Check live compatibility. Apply requires identities captured when the plan was constructed.
    pub async fn check_type_edits(&self, client: &Client) -> Result<(), Error> {
        if !self.has_type_edits() {
            return Ok(());
        }
        let names = self
            .type_edits
            .iter()
            .map(|e| e.column.data_type.as_str())
            .collect::<Vec<_>>();
        let targets = TypeTargets::load(client, &names).await?;
        for edit in &self.type_edits {
            let current = targets.get(&edit.column.data_type).expect("resolved type");
            if edit.target.is_some_and(|target| target != current) {
                return Err(Error::QueryFailed(format!(
                    "The reviewed destination type changed for column {:?}. Review again.",
                    edit.column.name
                )));
            }
            edit.validate(current)
                .map_err(|e| Error::QueryFailed(e.to_string()))?;
        }
        Ok(())
    }
}
