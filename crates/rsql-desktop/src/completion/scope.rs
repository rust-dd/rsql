use super::{
    Analysis, Declaration, Reference, ScopedRelation, SymbolKind,
    parser::{Budget, identifier},
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};
use tree_sitter::Node;

type Name = (Option<String>, String);

struct Source {
    relation: ScopedRelation,
    name_range: Option<Range<usize>>,
    alias: Option<Range<usize>>,
    definition: Option<Range<usize>>,
    start: usize,
}

#[derive(Default)]
struct Scope {
    parent: Option<usize>,
    /// Visible source positions at the parent query level.
    outer_sources: Option<Range<usize>>,
    sources: Vec<Source>,
    ctes: HashMap<String, (usize, Range<usize>)>,
}

struct RelationChange {
    visible_from: usize,
    definition: Option<Range<usize>>,
}

struct Field {
    scope: usize,
    schema: Option<String>,
    name: String,
    range: Range<usize>,
}

#[derive(Default)]
pub(super) struct Syntax {
    scopes: Vec<Scope>,
    fields: Vec<Field>,
    creates: HashMap<Name, Vec<RelationChange>>,
    pub analysis: Analysis,
    caret_scope: Option<usize>,
}

fn child<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    node.named_children(&mut node.walk())
        .find(|n| n.kind() == kind)
}

fn name(sql: &str, node: Node) -> String {
    identifier(&sql[node.byte_range()])
}

impl Syntax {
    fn scope(&mut self, parent: Option<usize>, outer_sources: Option<Range<usize>>) -> usize {
        let id = self.scopes.len();
        self.scopes.push(Scope {
            parent,
            outer_sources,
            ..Default::default()
        });
        id
    }

    pub fn read(
        root: Node,
        sql: &str,
        marker: Option<Range<usize>>,
        budget: &Budget,
    ) -> Result<Self, String> {
        let mut syntax = Self::default();
        let base = syntax.scope(None, None);
        let mut pending = vec![(root, base, None)];
        let mut top_statement = None;
        while let Some((node, mut scope, join_left)) = pending.pop() {
            budget.check()?;
            let parent = node.parent();
            let parent_kind = parent.map(|p| p.kind());
            match node.kind() {
                _ if node.is_missing() || node.is_error() => {
                    if marker
                        .as_ref()
                        .is_some_and(|r| node.start_byte() <= r.start && r.end <= node.end_byte())
                    {
                        // Recovery can detach an unfinished JOIN from the statement.
                        // The caret tree contains exactly one lexer-delimited statement.
                        syntax.caret_scope = Some(if scope == base {
                            top_statement.unwrap_or(base)
                        } else {
                            scope
                        });
                    }
                    continue;
                }
                "function_body" => continue,
                "statement" | "subquery" | "create_query" => {
                    let before = match parent_kind {
                        Some("relation")
                            if parent.and_then(|p| child(p, "keyword_lateral")).is_some() =>
                        {
                            parent.map(|p| 0..p.start_byte())
                        }
                        Some("cte" | "relation") => Some(0..0),
                        Some("lateral_join") => parent.map(|p| 0..p.start_byte()),
                        _ => None,
                    };
                    scope = syntax.scope(Some(scope), before);
                    if parent_kind == Some("program") {
                        top_statement = Some(scope);
                    }
                }
                "cte" => {
                    if let Some(id) = node.named_child(0).filter(|n| n.kind() == "identifier") {
                        let recursive = parent
                            .and_then(|p| p.named_child(1))
                            .is_some_and(|n| n.kind() == "keyword_recursive");
                        let visible = if recursive { 0 } else { node.end_byte() };
                        let key = syntax.declare(sql, id, SymbolKind::Cte);
                        syntax.scopes[scope]
                            .ctes
                            .insert(key, (visible, id.byte_range()));
                    }
                }
                "relation" | "lateral_join" | "insert" => syntax.source(
                    sql,
                    node,
                    scope,
                    child(node, "object_reference"),
                    marker.as_ref(),
                ),
                "object_reference" if node.child_by_field_name("database").is_none() => {
                    let Some(id) = node.child_by_field_name("name") else {
                        continue;
                    };
                    let key = (
                        node.child_by_field_name("schema").map(|s| name(sql, s)),
                        name(sql, id),
                    );
                    match parent_kind {
                        Some(
                            "create_table"
                            | "create_view"
                            | "create_materialized_view"
                            | "drop_table"
                            | "drop_view"
                            | "drop_materialized_view",
                        ) if parent.and_then(|p| p.parent()).is_some_and(|p| {
                            p.kind() == "statement"
                                && p.parent().is_some_and(|p| p.kind() == "program")
                        }) =>
                        {
                            let target = if parent_kind.is_some_and(|k| k.starts_with("create_")) {
                                syntax.declare(sql, id, SymbolKind::Relation);
                                Some(id.byte_range())
                            } else {
                                None
                            };
                            syntax.creates.entry(key).or_default().push(RelationChange {
                                visible_from: parent.expect("matched DDL parent").end_byte(),
                                definition: target,
                            });
                        }
                        Some("field" | "all_fields") => syntax.fields.push(Field {
                            scope,
                            schema: key.0,
                            name: key.1,
                            range: id.byte_range(),
                        }),
                        Some("from") => {
                            syntax.source(sql, node, scope, Some(node), marker.as_ref())
                        }
                        _ => (),
                    }
                }
                "identifier" if marker.as_ref() == Some(&node.byte_range()) => {
                    syntax.caret_scope = Some(scope)
                }
                _ => (),
            }
            let start = pending.len();
            let mut arm = scope;
            let mut left = None;
            for child in node.children(&mut node.walk()) {
                budget.check()?;
                if node.kind() == "set_operation" && child.kind() == "select" {
                    arm = syntax.scope(Some(scope), None);
                }
                if child.kind() == "keyword_on"
                    && let Some(left) = join_left
                {
                    // ON sees the operands of this join, excluding other comma groups
                    // and joins that occur later at the same query level.
                    arm = syntax.scope(Some(scope), Some(left..node.end_byte()));
                }
                if node.kind() == "from" && child.kind() == "relation" {
                    left = Some(child.start_byte());
                }
                let child_left = if child.kind().ends_with("join") {
                    left
                } else {
                    None
                };
                if node.kind() == "insert" {
                    match child.kind() {
                        "keyword_values" | "select" | "from" | "set_operation" if arm == scope => {
                            arm = syntax.scope(Some(scope), Some(0..0));
                        }
                        // ON CONFLICT can refer to the existing target row again.
                        "keyword_on" => arm = scope,
                        _ => (),
                    }
                }
                pending.push((child, arm, child_left));
            }
            pending[start..].reverse();
        }
        syntax.resolve(budget)?;
        Ok(syntax)
    }

    fn declare(&mut self, sql: &str, node: Node, kind: SymbolKind) -> String {
        let name = name(sql, node);
        self.analysis.declarations.push(Declaration {
            kind,
            name: name.clone(),
            range: node.byte_range(),
        });
        name
    }

    fn source(
        &mut self,
        sql: &str,
        node: Node,
        scope: usize,
        object: Option<Node>,
        marker: Option<&Range<usize>>,
    ) {
        let id = object.and_then(|o| o.child_by_field_name("name"));
        if id.is_some_and(|n| Some(&n.byte_range()) == marker) {
            return;
        }
        let alias = node
            .child_by_field_name("alias")
            .filter(|n| Some(&n.byte_range()) != marker);
        let Some(label) = id.or(alias) else { return };
        let alias_name = alias.map(|n| self.declare(sql, n, SymbolKind::Alias));
        let relation = ScopedRelation {
            name: name(sql, label),
            schema: object
                .and_then(|o| o.child_by_field_name("schema"))
                .map(|s| name(sql, s)),
            alias: alias_name,
            local: object.is_none_or(|o| o.child_by_field_name("database").is_some()),
        };
        self.scopes[scope].sources.push(Source {
            relation,
            alias: alias.map(|a| a.byte_range()),
            name_range: id.map(|n| n.byte_range()),
            definition: None,
            start: node.start_byte(),
        });
    }

    fn cte(
        &self,
        mut scope: usize,
        name: &str,
        at: usize,
        budget: &Budget,
    ) -> Result<Option<Range<usize>>, String> {
        loop {
            budget.check()?;
            let s = &self.scopes[scope];
            if let Some((start, def)) = s.ctes.get(name)
                && *start <= at
            {
                return Ok(Some(def.clone()));
            }
            let Some(parent) = s.parent else {
                return Ok(None);
            };
            scope = parent;
        }
    }

    fn resolve(&mut self, budget: &Budget) -> Result<(), String> {
        for sc in 0..self.scopes.len() {
            for idx in 0..self.scopes[sc].sources.len() {
                budget.check()?;
                let source = &self.scopes[sc].sources[idx];
                let Some(ref range) = source.name_range else {
                    continue;
                };
                if source.relation.local {
                    continue;
                }
                let key = (&source.relation.schema, &source.relation.name);
                let local = if key.0.is_none() {
                    self.cte(sc, key.1, source.start, budget)?
                } else {
                    None
                };
                let target = local.clone().or_else(|| {
                    let history = self.creates.get(&(key.0.clone(), key.1.clone()))?;
                    let i = history.partition_point(|change| change.visible_from <= source.start);
                    history.get(i.checked_sub(1)?)?.definition.clone()
                });
                if let Some(ref def) = target {
                    self.analysis.references.push(Reference {
                        range: range.clone(),
                        definition: def.clone(),
                    });
                }
                let source = &mut self.scopes[sc].sources[idx];
                source.relation.local = local.is_some();
                source.definition = target;
            }
        }
        for field in std::mem::take(&mut self.fields) {
            let mut definition = None;
            for (s, before) in self.layers(field.scope) {
                budget.check()?;
                let mut matched = None;
                for source in &s.sources {
                    budget.check()?;
                    if before.is_some_and(|bounds| !bounds.contains(&source.start)) {
                        continue;
                    }
                    let matches = match &field.schema {
                        Some(schema) => {
                            source.relation.alias.is_none()
                                && source.relation.schema.as_ref() == Some(schema)
                                && source.relation.name == field.name
                        }
                        None => source.relation.binding() == field.name,
                    };
                    if matches {
                        matched = Some(source);
                        if source.alias.is_some() {
                            break;
                        }
                    }
                }
                if let Some(source) = matched {
                    definition = source.alias.clone().or_else(|| source.definition.clone());
                    break;
                }
            }
            if let Some(definition) = definition {
                self.analysis.references.push(Reference {
                    range: field.range,
                    definition,
                });
            }
        }
        Ok(())
    }

    fn layers(&self, first: usize) -> impl Iterator<Item = (&Scope, Option<&Range<usize>>)> {
        std::iter::successors(Some((&self.scopes[first], None)), |(s, _)| {
            s.parent
                .map(|p| (&self.scopes[p], s.outer_sources.as_ref()))
        })
    }

    pub fn visible(
        &self,
        budget: &Budget,
        schema_qualified: bool,
    ) -> Result<Vec<ScopedRelation>, String> {
        let Some(first) = self.caret_scope else {
            return Ok(Vec::new());
        };
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for (s, before) in self.layers(first) {
            budget.check()?;
            let mut names = HashSet::new();
            for source in &s.sources {
                budget.check()?;
                let name = source.relation.binding();
                if before.is_none_or(|bounds| bounds.contains(&source.start))
                    && (schema_qualified || !seen.contains(name))
                {
                    result.push(source.relation.clone());
                    names.insert(name);
                }
            }
            seen.extend(names);
        }
        Ok(result)
    }
}
