use super::{EditId, Entry, ForeignTarget, NewObject, StructureDraft, ValidationError, sql};
use crate::postgres::metadata_schema::RelationKind;
use std::collections::HashSet;

fn name(value: &str, label: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.contains('\0') || value.len() > 63 {
        return Err(ValidationError(format!(
            "Enter a nonempty {label} of at most 63 bytes without NUL."
        )));
    }
    Ok(())
}

fn reserve<'a>(
    names: &mut HashSet<&'a str>,
    value: &'a str,
    label: &str,
) -> Result<(), ValidationError> {
    name(value, label)?;
    if !names.insert(value) {
        return Err(ValidationError(format!("Duplicate {label} {value:?}.")));
    }
    Ok(())
}

impl StructureDraft {
    pub(super) fn validate(&self, check: super::ReviewCheck<'_>) -> Result<(), ValidationError> {
        let meta = self.snapshot.metadata();
        if !matches!(
            meta.relation.kind,
            RelationKind::Table | RelationKind::PartitionedTable | RelationKind::ForeignTable
        ) {
            return Err(ValidationError("Structure edits require a table.".into()));
        }
        let backing = self.dropped_backing_indexes(check)?;
        let mut names = HashSet::new();
        for col in &self.columns {
            check()?;
            if let Some(current) = &col.current {
                if col.original().is_none_or(|old| current.name != old.name) {
                    name(&current.name, "column name")?;
                }
                if !names.insert(&current.name) {
                    return Err(ValidationError(format!(
                        "Duplicate column name {:?}.",
                        current.name
                    )));
                }
                if let Some(old) = &col.original
                    && old.data_type != current.data_type
                {
                    self.check_column_dependencies(
                        col.id,
                        &old.name,
                        "changing the type of",
                        &backing,
                        check,
                    )?;
                }
                if let Some(old) = &col.original
                    && (current.default != old.default || current.nullable != old.nullable)
                    && let EditId::Attribute(number) = col.id
                {
                    let source = meta
                        .columns
                        .iter()
                        .find(|c| c.identity.number == number)
                        .expect("catalog column");
                    if current.nullable
                        && !old.nullable
                        && source.domain_not_null
                        && old.data_type == current.data_type
                    {
                        return Err(ValidationError(format!(
                            "Column {:?} uses a NOT NULL domain. Choose a nullable type before making this column nullable.",
                            current.name
                        )));
                    }
                    if current.nullable
                        && !old.nullable
                        && (!source.identity_generation.is_empty()
                            || self
                                .primary_key()
                                .and_then(|p| p.current())
                                .is_some_and(|k| k.columns.contains(&col.id)))
                    {
                        return Err(ValidationError(format!(
                            "Column {:?} must remain NOT NULL while it is an identity or primary key column.",
                            current.name
                        )));
                    }
                    if current.default != old.default
                        && (!source.generated.is_empty() || !source.identity_generation.is_empty())
                    {
                        return Err(ValidationError(format!(
                            "A generated or identity column {:?} cannot have its default edited.",
                            old.name
                        )));
                    }
                }
            } else if let Some(old) = &col.original {
                self.check_column_dependencies(col.id, &old.name, "dropping", &backing, check)?;
            }
        }
        self.check_self_references(&backing, check)?;
        self.validate_keys(check)
    }

    pub(super) fn column_names(
        &self,
        ids: &[EditId],
        owner: &str,
        check: super::ReviewCheck<'_>,
    ) -> Result<Vec<&str>, ValidationError> {
        if ids.is_empty() {
            return Err(ValidationError(format!("Select columns for {owner:?}.")));
        }
        let mut distinct = HashSet::new();
        let mut names = Vec::with_capacity(ids.len());
        for id in ids {
            check()?;
            let entry = self.columns.iter().find(|c| c.id == *id);
            let Some(column) = entry.and_then(Entry::current) else {
                let missing = entry
                    .and_then(Entry::original)
                    .map_or_else(|| format!("{id:?}"), |c| format!("{:?}", c.name));
                return Err(ValidationError(format!(
                    "{owner:?} refers to removed or missing column {missing}. Update its columns or drop it explicitly."
                )));
            };
            if !distinct.insert(id) {
                return Err(ValidationError(format!(
                    "Duplicate column {:?} in {owner:?}.",
                    column.name
                )));
            }
            names.push(column.name.as_str());
        }
        Ok(names)
    }

    fn validate_keys(&self, check: super::ReviewCheck<'_>) -> Result<(), ValidationError> {
        let meta = self.snapshot.metadata();
        let mut constraints = HashSet::new();
        let mut relations = HashSet::from([meta.relation.name.as_str()]);
        for c in &meta.constraints {
            check()?;
            if c.kind != "p" && !self.is_dropped(EditId::Constraint(c.identity.oid)) {
                constraints.insert(c.identity.name.as_str());
            }
        }
        for i in &meta.indexes {
            check()?;
            if !i.primary
                && !self.is_dropped(EditId::Index(i.oid))
                && !i
                    .constraint
                    .as_ref()
                    .is_some_and(|c| self.is_dropped(EditId::Constraint(c.oid)))
            {
                relations.insert(i.name.as_str());
            }
        }
        if let Some(pk) = &self.primary
            && let Some(key) = &pk.current
        {
            self.column_names(&key.columns, &key.name, check)?;
            if pk.original == pk.current {
                constraints.insert(&key.name);
                relations.insert(&key.name);
            } else {
                reserve(&mut constraints, &key.name, "constraint name")?;
                reserve(&mut relations, &key.name, "relation name")?;
                if let EditId::Constraint(oid) = pk.id
                    && let Some(c) = meta.constraints.iter().find(|c| c.identity.oid == oid)
                    && !sql::plain_primary(c, meta)
                {
                    return Err(ValidationError(format!(
                        "Primary key {:?} has additional options. Edit its full definition to preserve them.",
                        c.identity.name
                    )));
                }
            }
        }
        for (_, obj) in &self.additions {
            check()?;
            let key = obj.key();
            self.column_names(&key.columns, &key.name, check)?;
            if !matches!(obj, NewObject::Index { .. }) {
                reserve(&mut constraints, &key.name, "constraint name")?;
            }
            if !matches!(obj, NewObject::ForeignKey(_)) {
                reserve(&mut relations, &key.name, "relation name")?;
            }
            if let NewObject::ForeignKey(fk) = obj {
                let target_len = match &fk.target {
                    ForeignTarget::ThisTable(ids) => {
                        self.column_names(ids, &key.name, check)?.len()
                    }
                    ForeignTarget::Relation {
                        schema,
                        table,
                        columns,
                    } => {
                        name(schema, "target schema")?;
                        name(table, "target table")?;
                        if *schema == meta.relation.schema && *table == meta.relation.name {
                            return Err(ValidationError(format!(
                                "Use this table's draft columns for self-reference {:?}.",
                                key.name
                            )));
                        }
                        let mut distinct = HashSet::new();
                        for col in columns {
                            check()?;
                            reserve(&mut distinct, col, "target column")?;
                        }
                        columns.len()
                    }
                };
                if target_len != key.columns.len() {
                    return Err(ValidationError(format!(
                        "Foreign key {:?} needs one target column for each source column.",
                        key.name
                    )));
                }
            }
        }
        Ok(())
    }
}
