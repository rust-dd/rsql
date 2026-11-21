use super::{
    EditId, ForeignKey, ForeignTarget, NewObject, StructureDraft, StructureSnapshot,
    ValidationError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum DroppedObject {
    Constraint(u32),
    Index(u32),
}

#[derive(Debug, Clone)]
pub struct StructurePlan {
    snapshot: Arc<StructureSnapshot>,
    pub(super) statements: Vec<String>,
    pub(crate) drops: BTreeMap<usize, DroppedObject>,
    pub(crate) columns: Vec<i16>,
    pub(super) type_edits: Vec<super::type_edits::TypeEdit>,
    pub(crate) requires_descendant_review: bool,
    pub(crate) constraints: Vec<u32>,
    pub(crate) indexes: Vec<u32>,
    pub(crate) namespaces: BTreeSet<String>,
    pub(crate) foreign_keys: Vec<ForeignKey>,
    pub(crate) foreign_identities: BTreeMap<String, (u32, u32, Vec<i16>)>,
}

impl StructurePlan {
    pub fn snapshot(&self) -> &StructureSnapshot {
        &self.snapshot
    }
    pub fn statements(&self) -> &[String] {
        &self.statements
    }
}

impl StructureDraft {
    /// Freeze the exact reviewed SQL together with the catalog identities it can change.
    pub fn review_plan(&self) -> Result<StructurePlan, ValidationError> {
        self.review_plan_with_types(&super::TypeTargets::default())
    }

    pub fn review_plan_with_types(
        &self,
        types: &super::TypeTargets,
    ) -> Result<StructurePlan, ValidationError> {
        self.checked_plan(types, &|| Ok(()))
    }

    fn checked_plan(
        &self,
        types: &super::TypeTargets,
        check: super::ReviewCheck<'_>,
    ) -> Result<StructurePlan, ValidationError> {
        check()?;
        let mut changed = !self.additions.is_empty()
            || !self.removed.is_empty()
            || self
                .primary
                .as_ref()
                .is_some_and(|pk| pk.original != pk.current);
        let mut requires_descendant_review = false;
        let snapshot = self.snapshot.clone();
        let mut columns = Vec::new();
        let mut type_edits = Vec::new();
        for (position, col) in self.columns.iter().enumerate() {
            check()?;
            changed |= col.original != col.current;
            if let Some((old, new)) = col.original().zip(col.current()) {
                requires_descendant_review |=
                    old.default != new.default || old.nullable != new.nullable;
            }
            if let (EditId::Attribute(n), Some(old)) = (col.id, &col.original)
                && col
                    .current
                    .as_ref()
                    .is_none_or(|new| new.data_type != old.data_type)
            {
                columns.push(n);
                if let Some(column) = &col.current {
                    let source = &snapshot.metadata().columns[position];
                    let edit = super::type_edits::TypeEdit {
                        column: column.clone(),
                        collation: source.collation.clone(),
                        target: types.get(&column.data_type),
                    };
                    if let Some(target) = edit.target {
                        edit.validate(target)?;
                    }
                    type_edits.push(edit);
                }
            }
        }
        requires_descendant_review |= !columns.is_empty();
        let mut drops = BTreeMap::new();
        let statements = if changed {
            self.validate(check)?;
            self.statements(&mut drops, types, check)?
        } else {
            Vec::new()
        };
        let mut constraints = BTreeSet::new();
        let mut indexes = Vec::new();
        for object in drops.values() {
            check()?;
            match *object {
                DroppedObject::Constraint(oid) => {
                    constraints.insert(oid);
                }
                DroppedObject::Index(oid) => indexes.push(oid),
            }
        }
        for index in &snapshot.metadata().indexes {
            check()?;
            if index
                .constraint
                .as_ref()
                .is_some_and(|c| constraints.contains(&c.oid))
            {
                indexes.push(index.oid);
            }
        }
        let mut namespaces = BTreeSet::from([snapshot.object().schema.clone()]);
        let mut foreign_keys = Vec::new();
        for (_, object) in &self.additions {
            check()?;
            if let NewObject::ForeignKey(
                fk @ ForeignKey {
                    target: ForeignTarget::Relation { schema, .. },
                    ..
                },
            ) = object
            {
                namespaces.insert(schema.clone());
                foreign_keys.push(fk.clone());
            }
        }
        Ok(StructurePlan {
            snapshot,
            statements,
            drops,
            requires_descendant_review,
            columns,
            type_edits,
            constraints: constraints.into_iter().collect(),
            indexes,
            namespaces,
            foreign_keys,
            foreign_identities: Default::default(),
        })
    }
}

/// The identities behind the column names offered by a foreign-key picker.
#[derive(Debug, Clone)]
pub struct ForeignChoices {
    namespace: u32,
    relation: crate::postgres::metadata_schema::RelationIdentity,
    pub columns: Vec<(String, i16)>,
}
impl ForeignChoices {
    pub async fn load(
        client: &tokio_postgres::Client,
        object: &super::ObjectRef,
    ) -> Result<Self, crate::Error> {
        use crate::postgres::metadata_schema::{load_visible_columns, relation_and_namespace};
        let (relation, namespace) =
            relation_and_namespace(client, &object.schema, &object.name).await?;
        if super::ObjectRef::relation(&object.project, relation.clone()) != *object {
            return Err(crate::Error::QueryFailed(
                "Object identity changed; refresh the catalog".into(),
            ));
        }
        let columns = load_visible_columns(client, &object.schema, &object.name)
            .await?
            .into_iter()
            .map(|c| (c.name, c.number))
            .collect();
        Ok(Self {
            namespace,
            relation,
            columns,
        })
    }
}
impl From<crate::postgres::metadata_schema::StructureMetadata> for ForeignChoices {
    fn from(meta: crate::postgres::metadata_schema::StructureMetadata) -> Self {
        Self {
            namespace: meta.namespace_oid,
            relation: meta.relation,
            columns: meta
                .columns
                .into_iter()
                .map(|c| (c.identity.name, c.identity.number))
                .collect(),
        }
    }
}

impl StructureDraft {
    pub fn review_selected_targets(
        &self,
        targets: &BTreeMap<EditId, Arc<ForeignChoices>>,
    ) -> Result<StructurePlan, ValidationError> {
        self.review_selected_targets_while(targets, || true)
    }

    pub fn review_selected_targets_while(
        &self,
        targets: &BTreeMap<EditId, Arc<ForeignChoices>>,
        current: impl Fn() -> bool,
    ) -> Result<StructurePlan, ValidationError> {
        self.review_resolved_targets_while(targets, &super::TypeTargets::default(), current)
    }

    pub fn review_resolved_targets_while(
        &self,
        targets: &BTreeMap<EditId, Arc<ForeignChoices>>,
        types: &super::TypeTargets,
        current: impl Fn() -> bool,
    ) -> Result<StructurePlan, ValidationError> {
        let check = || {
            if current() {
                Ok(())
            } else {
                Err(ValidationError("Structure review cancelled.".into()))
            }
        };
        let mut plan = self.checked_plan(types, &check)?;
        for (id, object) in self.added_objects() {
            check()?;
            let NewObject::ForeignKey(fk) = object else {
                continue;
            };
            let ForeignTarget::Relation {
                schema,
                table,
                columns,
            } = &fk.target
            else {
                continue;
            };
            let error = || {
                ValidationError(format!(
                    "Select the referenced table and columns for {:?} again.",
                    fk.key.name
                ))
            };
            let target = targets
                .get(id)
                .filter(|t| &t.relation.schema == schema && &t.relation.name == table)
                .ok_or_else(error)?;
            let attributes = columns
                .iter()
                .map(|name| {
                    check()?;
                    target
                        .columns
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, n)| *n)
                        .ok_or_else(error)
                })
                .collect::<Result<Vec<_>, _>>()?;
            plan.foreign_identities.insert(
                fk.key.name.clone(),
                (target.namespace, target.relation.oid, attributes),
            );
        }
        Ok(plan)
    }
}
