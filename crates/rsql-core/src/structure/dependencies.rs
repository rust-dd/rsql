use super::{EditId, StructureDraft, ValidationError};
use pg_query::ParserOptions;
use serde_json::Value;
use std::collections::BTreeSet;

fn references(
    expression: &str,
    column: &str,
    whole_row_name: Option<&str>,
    check: super::ReviewCheck<'_>,
) -> Result<bool, ValidationError> {
    check()?;
    let sql = format!("SELECT ({expression})");
    let Ok(parsed) = pg_query::parse(&sql, ParserOptions::DEFAULT) else {
        return Ok(true);
    };
    // pg_query's nodes() visitor deliberately skips some expression children. Walk the complete
    // serialized raw tree so array subscripts, indirections and nested expressions are covered.
    check()?;
    let Ok(tree) = serde_json::to_value(&parsed.protobuf) else {
        return Ok(true);
    };
    let mut pending = vec![&tree];
    while let Some(node) = pending.pop() {
        check()?;
        match node {
            Value::Object(map) => {
                if let Some(reference) = map.get("ColumnRef") {
                    let Some(fields) = reference["fields"].as_array() else {
                        return Ok(true);
                    };
                    let Some(last) = fields
                        .last()
                        .and_then(|f| f.pointer("/node/String/sval"))
                        .and_then(Value::as_str)
                    else {
                        // A whole-row star, or a reference the current binding cannot resolve.
                        return Ok(true);
                    };
                    if last == column || (fields.len() == 1 && Some(last) == whole_row_name) {
                        return Ok(true);
                    }
                } else {
                    pending.extend(map.values());
                }
            }
            Value::Array(items) => pending.extend(items),
            _ => (),
        }
    }
    Ok(false)
}

impl StructureDraft {
    fn drops_constraint_before_columns(&self, oid: u32) -> bool {
        let id = EditId::Constraint(oid);
        self.removed.contains(&id)
            || self
                .primary
                .as_ref()
                .is_some_and(|pk| pk.id == id && pk.original != pk.current)
    }

    pub(super) fn dropped_backing_indexes(
        &self,
        check: super::ReviewCheck<'_>,
    ) -> Result<BTreeSet<u32>, ValidationError> {
        let mut indexes = BTreeSet::new();
        if !self.removed.is_empty()
            || self
                .primary
                .as_ref()
                .is_some_and(|pk| pk.original != pk.current)
        {
            for c in &self.snapshot.metadata().constraints {
                check()?;
                if let Some(oid) = c.backing_index_oid
                    && self.drops_constraint_before_columns(c.identity.oid)
                {
                    indexes.insert(oid);
                }
            }
        }
        Ok(indexes)
    }

    fn drops_index_before_columns(&self, oid: u32, backing: &BTreeSet<u32>) -> bool {
        self.removed.contains(&EditId::Index(oid)) || backing.contains(&oid)
    }

    pub(super) fn check_self_references(
        &self,
        backing: &BTreeSet<u32>,
        check: super::ReviewCheck<'_>,
    ) -> Result<(), ValidationError> {
        let meta = self.snapshot.metadata();
        for c in &meta.constraints {
            check()?;
            let Some(fk) = &c.foreign_key else { continue };
            if fk.target.oid == meta.relation.oid
                && !self.drops_constraint_before_columns(c.identity.oid)
                && self.drops_index_before_columns(fk.referenced_index_oid, backing)
            {
                let index = meta
                    .indexes
                    .iter()
                    .find(|i| i.oid == fk.referenced_index_oid)
                    .map_or_else(
                        || fk.referenced_index_oid.to_string(),
                        |i| format!("{:?}", i.name),
                    );
                return Err(ValidationError(format!(
                    "Drop self-referencing foreign key {:?} before dropping or replacing its referenced index {index}.",
                    c.identity.name
                )));
            }
        }
        Ok(())
    }

    pub(super) fn check_column_dependencies(
        &self,
        id: EditId,
        column: &str,
        action: &str,
        backing: &BTreeSet<u32>,
        check: super::ReviewCheck<'_>,
    ) -> Result<(), ValidationError> {
        let meta = self.snapshot.metadata();
        let whole_row_name = (!meta
            .columns
            .iter()
            .any(|c| c.identity.name == meta.relation.name))
        .then_some(meta.relation.name.as_str());
        let mentions = |expr: &str| references(expr, column, whole_row_name, check);
        let mut owners = BTreeSet::new();
        for c in &meta.constraints {
            check()?;
            if self.drops_constraint_before_columns(c.identity.oid) {
                continue;
            }
            if c.columns.iter().any(|a| EditId::Attribute(a.number) == id)
                || c.foreign_key.as_ref().is_some_and(|fk| {
                    fk.target.oid == meta.relation.oid
                        && fk
                            .columns
                            .iter()
                            .any(|pair| EditId::Attribute(pair.target.number) == id)
                })
            {
                owners.insert(format!("constraint {:?}", c.identity.name));
            }
        }
        for i in &meta.indexes {
            check()?;
            if self.drops_index_before_columns(i.oid, backing) {
                continue;
            }
            let mut uses_column = false;
            for e in &i.entries {
                check()?;
                uses_column = match &e.column {
                    Some(a) => EditId::Attribute(a.number) == id,
                    None => mentions(&e.definition)?,
                };
                if uses_column {
                    break;
                }
            }
            if !uses_column && let Some(predicate) = &i.predicate {
                uses_column = mentions(predicate)?;
            }
            if uses_column {
                owners.insert(format!("index {:?}", i.name));
            }
        }
        for other in &meta.columns {
            check()?;
            let other_id = EditId::Attribute(other.identity.number);
            if other_id != id
                && !self.is_dropped(other_id)
                && let Some(expr) = &other.generation_expression
                && mentions(expr)?
            {
                owners.insert(format!("generated column {:?}", other.identity.name));
            }
        }
        if owners.is_empty() {
            return Ok(());
        }
        let owners = owners.into_iter().collect::<Vec<_>>().join(", ");
        Err(ValidationError(format!(
            "Resolve {owners} before {action} column {column:?}."
        )))
    }
}
