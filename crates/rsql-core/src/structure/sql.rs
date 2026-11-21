use super::{
    Column, DroppedObject, EditId, Entry, ForeignTarget, KeyColumns, NewObject, StructureDraft,
    ValidationError, constraints::Group, fragments,
};
use crate::{
    mutation::quote_ident,
    postgres::metadata_schema::{ColumnMetadata, ConstraintMetadata, StructureMetadata},
};
use std::collections::{BTreeMap, HashSet};

fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_ident(schema), quote_ident(name))
}

fn identifiers<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    names
        .into_iter()
        .map(quote_ident)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn plain_primary(constraint: &ConstraintMetadata, meta: &StructureMetadata) -> bool {
    let cols = identifiers(constraint.columns.iter().map(|c| c.name.as_str()));
    let raw = format!("ALTER TABLE t ADD {};", constraint.definition);
    if !fragments::equivalent(&raw, &format!("ALTER TABLE t ADD PRIMARY KEY ({cols});")) {
        return false;
    }
    let Some(oid) = constraint.backing_index_oid else {
        return true;
    };
    let Some(index) = meta.indexes.iter().find(|i| i.oid == oid) else {
        return false;
    };
    // pg_get_constraintdef omits storage options which are present in pg_get_indexdef.
    let table = qualified(&meta.relation.schema, &meta.relation.name);
    let name = quote_ident(&index.name);
    index.valid
        && fragments::equivalent(
            &index.definition,
            &format!("CREATE UNIQUE INDEX {name} ON {table} USING btree ({cols});"),
        )
}

impl StructureDraft {
    pub(super) fn statements(
        &self,
        drops: &mut BTreeMap<usize, DroppedObject>,
        types: &super::TypeTargets,
        check: super::ReviewCheck<'_>,
    ) -> Result<Vec<String>, ValidationError> {
        let relation = &self.snapshot.metadata().relation;
        let table = qualified(&relation.schema, &relation.name);
        let alter = format!("ALTER TABLE {table}");
        let drop_constraint = |name| format!("{alter} DROP CONSTRAINT {};", quote_ident(name));
        let mut sql = Vec::new();
        if !self.removed.is_empty() {
            let meta = self.snapshot.metadata();
            for kind in ["f", "u"] {
                for constraint in &meta.constraints {
                    check()?;
                    let oid = constraint.identity.oid;
                    if constraint.kind == kind && self.removed.contains(&EditId::Constraint(oid)) {
                        drops.insert(sql.len(), DroppedObject::Constraint(oid));
                        sql.push(drop_constraint(&constraint.identity.name));
                    }
                }
            }
            for index in &meta.indexes {
                check()?;
                if self.removed.contains(&EditId::Index(index.oid)) {
                    drops.insert(sql.len(), DroppedObject::Index(index.oid));
                    sql.push(format!(
                        "DROP INDEX {};",
                        qualified(&relation.schema, &index.name)
                    ));
                }
            }
        }
        let pk = self.primary.as_ref().filter(|p| p.original != p.current);
        if let Some(key) = pk
            && let (EditId::Constraint(oid), Some(old)) = (key.id(), key.original())
        {
            drops.insert(sql.len(), DroppedObject::Constraint(oid));
            sql.push(drop_constraint(&old.name));
        }
        self.column_statements(&alter, &mut sql, types, check)?;
        let key_list = |key: &KeyColumns| {
            self.column_names(&key.columns, &key.name, check)
                .map(identifiers)
        };
        if let Some(key) = pk.and_then(Entry::current) {
            sql.push(format!(
                "{alter} ADD CONSTRAINT {} PRIMARY KEY ({});",
                quote_ident(&key.name),
                key_list(key)?
            ));
        }
        for group in [Group::Unique, Group::Index, Group::Foreign] {
            for (_, object) in &self.additions {
                check()?;
                if object.group() != group {
                    continue;
                }
                let key = object.key();
                let name = quote_ident(&key.name);
                let cols = key_list(key)?;
                let stmt = match object {
                    NewObject::Unique(_) => {
                        format!("{alter} ADD CONSTRAINT {name} UNIQUE ({cols});")
                    }
                    NewObject::Index { unique, .. } => {
                        let unique = if *unique { "UNIQUE " } else { "" };
                        format!("CREATE {unique}INDEX {name} ON {table} ({cols});")
                    }
                    NewObject::ForeignKey(fk) => {
                        let (target, targets) = match &fk.target {
                            ForeignTarget::ThisTable(ids) => (
                                table.clone(),
                                identifiers(self.column_names(ids, &key.name, check)?),
                            ),
                            ForeignTarget::Relation {
                                schema,
                                table,
                                columns,
                            } => (
                                qualified(schema, table),
                                identifiers(columns.iter().map(String::as_str)),
                            ),
                        };
                        format!(
                            "{alter} ADD CONSTRAINT {name} FOREIGN KEY ({cols}) REFERENCES {target} ({targets}) ON UPDATE {} ON DELETE {};",
                            fk.on_update.sql(),
                            fk.on_delete.sql()
                        )
                    }
                };
                sql.push(stmt);
            }
        }
        for stmt in &sql {
            check()?;
            fragments::statement(stmt)?;
        }
        Ok(sql)
    }

    fn column_statements(
        &self,
        alter: &str,
        sql: &mut Vec<String>,
        types: &super::TypeTargets,
        check: super::ReviewCheck<'_>,
    ) -> Result<(), ValidationError> {
        let mut generated = Vec::new();
        let mut ordinary = Vec::new();
        let mut occupied = HashSet::new();
        let mut changed = Vec::new();
        let mut pending = Vec::new();
        for (col, source) in self.columns.iter().zip(&self.snapshot.metadata().columns) {
            check()?;
            if let Some(new) = col.current() {
                occupied.insert(source.identity.name.as_str());
                let old = col.original().expect("catalog column");
                if !std::ptr::eq(old, new) && old != new {
                    changed.push((source, old, new));
                    if old.name != new.name {
                        pending.push((old, new));
                    }
                }
            } else if source.generated.is_empty() {
                ordinary.push(&source.identity.name);
            } else {
                generated.push(&source.identity.name);
            }
        }
        for name in generated.into_iter().chain(ordinary) {
            check()?;
            sql.push(format!("{alter} DROP COLUMN {};", quote_ident(name)));
        }
        for col in self
            .columns
            .iter()
            .skip(self.snapshot.metadata().columns.len())
            .filter_map(Entry::current)
        {
            check()?;
            if !occupied.insert(&col.name) {
                return Err(ValidationError(format!(
                    "Rename the existing column {:?} before adding a column with that name.",
                    col.name
                )));
            }
            let ty = fragments::data_type(&col.data_type)?;
            let null = if col.nullable { "" } else { " NOT NULL" };
            let mut stmt = format!("{alter} ADD COLUMN {} {ty}{null}", quote_ident(&col.name));
            if let Some(raw) = &col.default {
                check()?;
                stmt.push_str(&format!(" DEFAULT ({})", fragments::default(raw)?));
            }
            sql.push(format!("{stmt};"));
        }

        while !pending.is_empty() {
            check()?;
            let Some(i) = pending
                .iter()
                .position(|(_, new)| !occupied.contains(new.name.as_str()))
            else {
                return Err(ValidationError(format!(
                    "Rename {:?} through an unused intermediate name to resolve the rename cycle.",
                    pending[0].0.name
                )));
            };
            let (old, new) = pending.remove(i);
            sql.push(format!(
                "{alter} RENAME COLUMN {} TO {};",
                quote_ident(&old.name),
                quote_ident(&new.name)
            ));
            occupied.remove(old.name.as_str());
            occupied.insert(&new.name);
        }
        for (source, old, new) in changed {
            check()?;
            alter_column(alter, old, new, source, types, sql, check)?;
        }
        Ok(())
    }
}

fn alter_column(
    alter: &str,
    old: &Column,
    new: &Column,
    source: &ColumnMetadata,
    types: &super::TypeTargets,
    sql: &mut Vec<String>,
    check: super::ReviewCheck<'_>,
) -> Result<(), ValidationError> {
    let name = quote_ident(&new.name);
    let prefix = format!("{alter} ALTER COLUMN {name}");
    let cleared_default = new.removes_default_before_cast(old);
    if cleared_default {
        // PostgreSQL coerces the existing default independently of the USING expression.
        sql.push(format!("{prefix} DROP DEFAULT;"));
    }
    if new.data_type != old.data_type {
        let ty = fragments::data_type(&new.data_type)?;
        let using = if !source.generated.is_empty() {
            String::new()
        } else {
            format!(" USING {name}::{ty}")
        };
        let collation = source
            .collation
            .as_ref()
            .filter(|c| c.preserve_explicitly)
            .map(|c| format!(" COLLATE {}", qualified(&c.schema, &c.name)))
            .unwrap_or_default();
        let mut statement = format!("{prefix} TYPE {ty}{collation}{using}");
        if source.domain_not_null && !source.local_not_null && !new.nullable {
            let target = types.get(&new.data_type).ok_or_else(|| ValidationError(format!(
                "Resolve the destination type for column {:?} before reviewing its inherited NOT NULL requirement.",
                new.name
            )))?;
            if !target.domain_not_null {
                statement.push_str(&format!(", ALTER COLUMN {name} SET NOT NULL"));
            }
        }
        statement.push(';');
        sql.push(statement);
    }
    if new.nullable != old.nullable {
        let op = if new.nullable { "DROP" } else { "SET" };
        sql.push(format!("{prefix} {op} NOT NULL;"));
    }
    check()?;
    if new.default != old.default && !(cleared_default && new.default.is_none()) {
        let op = match &new.default {
            None => "DROP DEFAULT".into(),
            Some(raw) => format!("SET DEFAULT {}", fragments::default(raw)?),
        };
        sql.push(format!("{prefix} {op};"));
    }
    Ok(())
}
