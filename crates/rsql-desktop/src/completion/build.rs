use super::identifiers::{alias, quote};
use super::{
    CatalogSnapshot, CompletionContext, ExpectedKind, ScopedRelation, Suggestion, SuggestionKind,
    snippets::SNIPPETS,
};
use rsql_core::postgres::schema_index::{IndexedRelation, RelationKind, SchemaIndex};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

fn snippet_literal(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('$', "\\$")
        .replace('}', "\\}")
}

fn item(kind: SuggestionKind, label: &str, insert: String, range: &Range<usize>) -> Suggestion {
    use SuggestionKind::*;
    let rank = match kind {
        Column => 0,
        Relation => 1,
        Function => 2,
        Schema => 3,
        Keyword => 4,
        Snippet => 5,
    };
    Suggestion {
        kind,
        label: label.into(),
        insert_text: insert,
        range: range.clone(),
        sort_text: format!("{rank}{}", label.to_lowercase()),
        filter_text: label.into(),
        is_snippet: matches!(kind, Relation | Function | Snippet),
        detail: None,
        documentation: None,
    }
}

struct Items<'a> {
    unordered: Vec<Suggestion>,
    ranked: BTreeMap<(String, usize), Suggestion>,
    prefix: Option<&'a str>,
    cancelled: &'a dyn Fn() -> bool,
}
impl Items<'_> {
    fn check(&self) -> Option<()> {
        (!(self.cancelled)()).then_some(())
    }
    fn push(&mut self, item: Suggestion) {
        if let Some(prefix) = self.prefix {
            if item.filter_text.to_lowercase().starts_with(prefix) {
                // Rank while producing. A large final, uninterruptible sort would delay cancellation.
                self.ranked
                    .insert((item.sort_text.clone(), self.ranked.len()), item);
            }
        } else {
            self.unordered.push(item);
        }
    }
    fn find<'a, T>(&self, items: &'a [T], predicate: impl Fn(&T) -> bool) -> Option<Option<&'a T>> {
        for item in items {
            self.check()?;
            if predicate(item) {
                return Some(Some(item));
            }
        }
        Some(None)
    }
}

fn relations(items: &mut Items<'_>, index: &SchemaIndex, range: &Range<usize>) -> Option<()> {
    for relation in &index.relations {
        items.check()?;
        let name = &relation.name;
        let alias = alias(name);
        let insert = format!(
            "{} ${{1:{}}}",
            snippet_literal(&quote(name)),
            snippet_literal(&alias)
        );
        let mut entry = item(SuggestionKind::Relation, name, insert, range);
        entry.detail = Some(
            match relation.kind {
                RelationKind::Table => "table",
                RelationKind::View => "view",
                RelationKind::MaterializedView => "materializedview",
            }
            .into(),
        );
        entry.documentation = relation.comment.clone();
        items.push(entry);
    }
    Some(())
}

fn functions(items: &mut Items<'_>, index: &SchemaIndex, range: &Range<usize>) -> Option<()> {
    for f in &index.functions {
        items.check()?;
        let mut entry = item(
            SuggestionKind::Function,
            &f.name,
            format!("{}($0)", snippet_literal(&quote(&f.name))),
            range,
        );
        entry.detail = Some(format!("({}) → {}", f.signature, f.return_type));
        items.push(entry);
    }
    Some(())
}

fn columns(
    items: &mut Items<'_>,
    relation: &IndexedRelation,
    binding: &str,
    range: &Range<usize>,
) -> Option<()> {
    for col in &relation.columns {
        items.check()?;
        let mut detail = format!("{binding} · {}", col.data_type);
        if col.is_primary_key {
            detail.push_str(" · PK");
        }
        if !col.nullable {
            detail.push_str(" · NOT NULL");
        }
        if let Some((s, r, c)) = &col.references {
            detail.push_str(&format!(" · → {s}.{r}.{c}"));
        }
        let mut entry = item(SuggestionKind::Column, &col.name, quote(&col.name), range);
        entry.detail = Some(detail);
        items.push(entry);
    }
    Some(())
}

fn scoped_columns(
    items: &mut Items<'_>,
    source: &ScopedRelation,
    catalog: &CatalogSnapshot,
    range: &Range<usize>,
) -> Option<()> {
    if source.local {
        return Some(());
    }
    let schema = source.schema.as_ref().unwrap_or(&catalog.default_schema);
    if let Some(index) = catalog.indexes.get(schema)
        && let Some(relation) = items.find(&index.relations, |r| r.name == source.name)?
    {
        columns(items, relation, source.binding(), range)?;
    }
    Some(())
}

pub fn build(context: &CompletionContext, catalog: &CatalogSnapshot) -> Vec<Suggestion> {
    let mut items = Items {
        unordered: vec![],
        ranked: BTreeMap::new(),
        prefix: None,
        cancelled: &|| false,
    };
    collect(context, catalog, &mut items).expect("uncancelled completion");
    items.unordered
}

pub fn suggestions(
    context: &CompletionContext,
    catalog: &CatalogSnapshot,
    prefix: &str,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<Suggestion>> {
    let mut items = Items {
        unordered: vec![],
        ranked: BTreeMap::new(),
        prefix: Some(prefix),
        cancelled: &cancelled,
    };
    collect(context, catalog, &mut items)?;
    let mut sorted = Vec::with_capacity(items.ranked.len());
    for item in items.ranked.into_values() {
        if cancelled() {
            return None;
        }
        sorted.push(item);
    }
    Some(sorted)
}

fn collect(
    context: &CompletionContext,
    catalog: &CatalogSnapshot,
    items: &mut Items<'_>,
) -> Option<()> {
    items.check()?;
    if !context.enabled {
        return Some(());
    }
    let range = &context.range;
    let wants_relation = context
        .kinds
        .iter()
        .any(|k| matches!(k, ExpectedKind::Table | ExpectedKind::View));
    if let Some((name, prefix)) = context.qualifier.split_last() {
        if !wants_relation {
            let source = if let Some(schema) = prefix.last() {
                items.find(&context.scope, |s| {
                    !s.local
                        && s.alias.is_none()
                        && s.name == *name
                        && s.schema.as_ref().unwrap_or(&catalog.default_schema) == schema
                })?
            } else if let Some(source) =
                items.find(&context.scope, |s| s.alias.as_ref() == Some(name))?
            {
                Some(source)
            } else {
                items.find(&context.scope, |s| s.binding() == name)?
            };
            if let Some(source) = source {
                scoped_columns(items, source, catalog, range)?;
                return Some(());
            }
        }
        if let Some(index) = catalog.indexes.get(name) {
            relations(items, index, range)?;
            functions(items, index, range)?;
        }
        return Some(());
    }
    if context.kinds.contains(&ExpectedKind::Column) {
        for source in &context.scope {
            items.check()?;
            scoped_columns(items, source, catalog, range)?;
        }
    }
    let default = catalog.indexes.get(&catalog.default_schema);
    if let Some(index) = default
        && wants_relation
    {
        relations(items, index, range)?;
    }
    if let Some(index) = default
        && context.kinds.contains(&ExpectedKind::Function)
    {
        functions(items, index, range)?;
    }
    let wants_schema = context.kinds.contains(&ExpectedKind::Database);
    if wants_relation || wants_schema {
        let mut names = BTreeSet::new();
        for name in catalog.schemas.iter().chain(catalog.indexes.keys()) {
            items.check()?;
            names.insert(name);
        }
        for name in names {
            items.check()?;
            if wants_schema || name != &catalog.default_schema {
                let mut entry = item(SuggestionKind::Schema, name, quote(name), range);
                entry.detail = Some("schema".into());
                items.push(entry);
            }
        }
    }
    for keyword in &context.keywords {
        items.check()?;
        items.push(item(
            SuggestionKind::Keyword,
            keyword,
            keyword.clone(),
            range,
        ));
    }
    if !wants_relation {
        for &(name, detail, insert) in SNIPPETS {
            items.check()?;
            let mut entry = item(SuggestionKind::Snippet, name, insert.into(), range);
            entry.detail = Some(detail.into());
            items.push(entry);
        }
    }
    Some(())
}
