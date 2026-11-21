use super::*;
use std::sync::Arc;
use tokio::sync::{oneshot, watch};
#[path = "catalog_fetch.rs"]
mod catalog_fetch;
#[path = "editor_catalog.rs"]
mod editor_catalog;
use catalog_fetch::fetch;
mod actions;
mod foreign_choices;
mod menu;
mod refresh;
mod tree;
pub(super) use actions::{Action, Capture, copy_name, open_draft, select_sql};

use rsql_core::{
    ApplicationServices,
    postgres::{
        metadata_schema as meta, metadata_views_functions as objects,
        schema_index::{SchemaIndex, load_schema_index},
    },
};
use rsql_desktop::{
    connection::ConnectionProfile,
    metadata::{ObjectKind, ObjectRef},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Node {
    Schemas,
    Databases,
    Tablespaces,
    Schema(String),
    Relation(ObjectRef),
    Children(ObjectRef, Child),
    Section(String, Section),
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Child {
    Columns,
    Indexes,
    Constraints,
    Triggers,
    Rules,
    Policies,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Section {
    Tables,
    Views,
    MaterializedViews,
    Functions,
    TriggerFunctions,
}
impl Section {
    const LIST: [(Self, &'static str); 5] = [
        (Self::Tables, "Tables"),
        (Self::Views, "Views"),
        (Self::MaterializedViews, "Materialized Views"),
        (Self::Functions, "Functions"),
        (Self::TriggerFunctions, "Trigger Functions"),
    ];
}
enum Data {
    Names(Vec<String>),
    Index(Arc<SchemaIndex>),
    Relations(Vec<RelationEntry>),
    Columns(Vec<ColumnLeaf>),
    Indexes(Vec<rsql_desktop::metadata::properties::IndexRow>),
    Details(Vec<DetailLeaf>),
    Routines(Vec<RoutineEntry>),
}
struct DetailLeaf {
    name: String,
    detail: String,
    definition: Option<rsql_desktop::metadata::properties::PropertyText>,
}
impl DetailLeaf {
    fn new(name: String, detail: String) -> Self {
        Self {
            name,
            detail,
            definition: None,
        }
    }
}
struct ColumnLeaf {
    number: i16,
    name: String,
    ty: String,
    nullable: bool,
    primary: bool,
}

impl From<meta::VisibleColumn> for ColumnLeaf {
    fn from(column: meta::VisibleColumn) -> Self {
        Self {
            number: column.number,
            name: column.name,
            ty: column.formatted_type,
            nullable: column.nullable,
            primary: column.primary,
        }
    }
}

struct RelationEntry {
    object: ObjectRef,
    size: String,
}
struct RoutineEntry {
    object: ObjectRef,
    arguments: String,
    return_type: Option<String>,
}

fn routine_detail(arguments: &str, return_type: Option<&str>) -> String {
    match return_type {
        Some(ty) => format!("({arguments}) → {ty}"),
        None => format!("({arguments})"),
    }
}
struct Pending {
    id: tokio::task::Id,
    _cancel_on_drop: oneshot::Sender<()>,
    published: watch::Sender<bool>,
    background_retry: bool,
}
impl Pending {
    fn new(id: tokio::task::Id, cancel: oneshot::Sender<()>) -> Self {
        Self {
            id,
            _cancel_on_drop: cancel,
            published: watch::channel(false).0,
            background_retry: false,
        }
    }
    fn has_editor_interest(&self) -> bool {
        self.background_retry || self.published.receiver_count() > 0
    }
}
enum Load {
    Pending(Pending),
    Ready(Data),
    Failed(String),
}
#[derive(Default)]
pub(super) struct Catalog {
    pub(super) epoch: u64,
    pub(super) generation: watch::Sender<()>,
    editor_attempts: BTreeSet<Node>,
    entries: BTreeMap<Node, Load>,
    expanded: BTreeSet<Node>,
    selected: Option<Node>,
    feedback: Option<refresh::Feedback>,
}

impl Catalog {
    fn is_expanded(&self, node: &Node) -> bool {
        // Default-open sections store a toggle only after an explicit collapse.
        self.expanded.contains(node)
            != matches!(
                node,
                Node::Children(_, Child::Columns) | Node::Section(_, Section::Tables)
            )
    }

    pub(in crate::workspace) fn databases(&self) -> Result<Option<&[String]>, &str> {
        match self.entries.get(&Node::Databases) {
            Some(Load::Ready(Data::Names(names))) => Ok(Some(names)),
            Some(Load::Failed(error)) => Err(error),
            _ => Ok(None),
        }
    }

    fn ready_count(&self, node: &Node) -> Option<usize> {
        match self.entries.get(node) {
            Some(Load::Ready(Data::Relations(rows))) => Some(rows.len()),
            Some(Load::Ready(Data::Routines(rows))) => Some(rows.len()),
            Some(Load::Ready(Data::Columns(rows))) => Some(rows.len()),
            Some(Load::Ready(Data::Indexes(rows))) => Some(rows.len()),
            Some(Load::Ready(Data::Details(rows))) => Some(rows.len()),
            Some(Load::Ready(Data::Names(rows))) => Some(rows.len()),
            _ => None,
        }
    }

    pub(super) fn schema_names(&self) -> Result<Option<&[String]>, &str> {
        match self.entries.get(&Node::Schemas) {
            Some(Load::Ready(Data::Names(names))) => Ok(Some(names)),
            Some(Load::Failed(error)) => Err(error),
            _ => Ok(None),
        }
    }

    fn publish(
        &mut self,
        request: tokio::task::Id,
        node: Node,
        result: Result<Data, String>,
    ) -> Option<String> {
        if !matches!(self.entries.get(&node),Some(Load::Pending(job)) if job.id==request) {
            return None;
        }
        let warm = match (&node, &result) {
            (Node::Schemas, Ok(Data::Names(names))) => names
                .iter()
                .find(|s| s.as_str() == "public")
                .or(names.first())
                .cloned(),
            _ => None,
        };
        if let Some(Load::Pending(pending)) = self.entries.get(&node) {
            pending.published.send_replace(true);
        }
        self.entries.insert(
            node,
            match result {
                Ok(data) => Load::Ready(data),
                Err(e) => Load::Failed(e),
            },
        );
        warm
    }
}

impl Workspace {
    pub(in crate::workspace) fn retry_database_discovery(
        &mut self,
        target: &Capture,
        cx: &mut Context<Self>,
    ) {
        if target.current(self) {
            self.load_node(&target.project, Node::Databases, cx);
        }
    }

    pub(super) fn refresh_catalog(&mut self, id: &str, cx: &mut Context<Self>) {
        self.retire_import_read(id, cx);
        self.invalidate_project_language(id, cx);
        self.navigation.catalog_serial += 1;
        let catalog = self.navigation.catalogs.entry(id.into()).or_default();
        let mut nodes: Vec<_> = catalog
            .entries
            .keys()
            .filter(|n| match n {
                Node::Children(object, _) => catalog.is_expanded(&Node::Relation(object.clone())),
                Node::Schema(_) => true,
                Node::Section(schema, _) => {
                    catalog.is_expanded(n) || catalog.is_expanded(&Node::Schema(schema.clone()))
                }
                _ => catalog.is_expanded(n),
            })
            .cloned()
            .collect();
        catalog.entries.clear();
        catalog.editor_attempts.clear();
        catalog.epoch = self.navigation.catalog_serial;
        catalog.generation.send_replace(());
        nodes.extend([Node::Schemas, Node::Databases, Node::Tablespaces]);
        for node in nodes {
            self.load_node(id, node, cx);
        }
    }

    pub(super) fn ensure_schema_names(
        &mut self,
        project: &str,
        retry: bool,
        cx: &mut Context<Self>,
    ) {
        if retry
            || self
                .navigation
                .catalogs
                .get(project)
                .is_some_and(|c| !c.entries.contains_key(&Node::Schemas))
        {
            self.load_node(project, Node::Schemas, cx);
        }
    }

    fn load_node(&mut self, id: &str, node: Node, cx: &mut Context<Self>) {
        if matches!(node, Node::Relation(..)) {
            return;
        }
        let Some(catalog) = self.navigation.catalogs.get_mut(id) else {
            return;
        };
        if matches!(
            catalog.entries.get(&node),
            Some(Load::Pending(_) | Load::Ready(_))
        ) {
            return;
        }
        let epoch = catalog.epoch;
        let svc = self.session.services.clone();
        let id = id.to_string();
        let target = id.clone();
        let request = node.clone();
        let (cancel, cancelled) = oneshot::channel();
        let job = self
            .session
            .runtime
            .spawn(async move { fetch(&svc, &target, &request, cancelled).await });
        let request_id = job.id();
        catalog.entries.insert(
            node.clone(),
            Load::Pending(Pending::new(request_id, cancel)),
        );
        cx.spawn(async move |view, cx| {
            let result = job
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = view.update(cx, |v, cx| {
                let Some(c) = v
                    .navigation
                    .catalogs
                    .get_mut(&id)
                    .filter(|c| c.epoch == epoch)
                else {
                    return;
                };
                let warm = c.publish(request_id, node.clone(), result);
                let feedback = c.settle_feedback(&node);
                if let Some(name) = warm {
                    v.load_node(&id, Node::Schema(name), cx);
                }
                if let Some(failed) = feedback {
                    v.toasts.show(refresh::toast(failed), cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_node(&mut self, id: &str, node: Node, cx: &mut Context<Self>) {
        let Some(c) = self.navigation.catalogs.get_mut(id) else {
            return;
        };
        let opening = !c.is_expanded(&node);
        if !c.expanded.remove(&node) {
            c.expanded.insert(node.clone());
        }
        if opening {
            if let Node::Relation(object) = &node
                && object.kind.is_table()
            {
                let children = [
                    Child::Columns,
                    Child::Indexes,
                    Child::Constraints,
                    Child::Triggers,
                    Child::Rules,
                    Child::Policies,
                ]
                .map(|kind| Node::Children(object.clone(), kind));
                for child in children {
                    self.load_node(id, child, cx);
                }
            }
            if let Node::Schema(schema) = &node {
                for (kind, _) in Section::LIST {
                    self.load_node(id, Node::Section(schema.clone(), kind), cx);
                }
            }
            self.load_node(id, node, cx);
        } else {
            if let Node::Relation(object) = &node {
                c.entries.retain(|n,load| !matches!((n,load), (Node::Children(o,_), Load::Pending(job)) if o == object && !job.has_editor_interest()));
            }
            if let Node::Schema(schema) = &node {
                c.entries.retain(|n,load| !matches!((n,load),(Node::Section(s,_),Load::Pending(_)) if s == schema));
            }
            if matches!(c.entries.get(&node), Some(Load::Pending(job)) if !job.has_editor_interest())
            {
                c.entries.remove(&node);
            }
        }
        cx.notify();
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum DraftObject {
    Schema(String),
    Relation(ObjectRef),
    Function(ObjectRef),
}

#[derive(Clone)]
pub(super) struct CatalogDraft {
    pub(super) object: DraftObject,
    pub(super) detail: Option<String>,
    pub(super) signature: Option<String>,
}
impl CatalogDraft {
    fn routine(object: ObjectRef, arguments: &str, return_type: Option<&str>) -> Self {
        Self {
            signature: Some(format!("{}({arguments})", object.name)),
            object: DraftObject::Function(object),
            detail: Some(routine_detail(arguments, return_type)),
        }
    }
}
impl PartialEq for CatalogDraft {
    fn eq(&self, other: &Self) -> bool {
        self.object == other.object
    }
}
impl Eq for CatalogDraft {}

fn literal(value: &str) -> String {
    format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

impl DraftObject {
    pub(super) fn search_fields(&self) -> (&str, &str, &str) {
        match self {
            Self::Schema(name) => (name, "", "schema"),
            Self::Function(o) => (&o.name, &o.schema, "function"),
            Self::Relation(o) => (
                &o.name,
                &o.schema,
                match o.kind {
                    ObjectKind::MaterializedView => "materialized view",
                    ObjectKind::View => "view",
                    _ => "table",
                },
            ),
        }
    }

    pub(super) fn sql(&self) -> String {
        match self {
            Self::Schema(name) => format!(
                "SET search_path TO {};",
                rsql_core::mutation::quote_ident(name)
            ),
            Self::Relation(o) => select_sql(&o.schema, &o.name),
            Self::Function(object) => {
                let Some(identity) = &object.routine_identity else {
                    return "-- Routine identity is unavailable; refresh the catalog.".into();
                };
                let schema = literal(&object.schema);
                let name = literal(&object.name);
                let args = identity
                    .argument_types
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let oid = identity.oid;
                format!(
                    "SELECT pg_catalog.pg_get_functiondef(p.oid)\n\
                    FROM pg_catalog.pg_proc p\n\
                    JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace\n\
                    WHERE n.nspname = {schema} AND p.proname = {name}\n  \
                    AND p.oid = {oid} AND ARRAY(SELECT pg_catalog.unnest(p.proargtypes)) = ARRAY[{args}]::pg_catalog.oid[] AND p.prokind IN ('f','p','w');"
                )
            }
        }
    }
}

impl Catalog {
    pub(super) fn drafts(&self, project: &str) -> Vec<CatalogDraft> {
        let mut rows = Vec::new();
        if let Ok(Some(names)) = self.schema_names() {
            rows.extend(names.iter().map(|s| CatalogDraft {
                object: DraftObject::Schema(s.clone()),
                detail: None,
                signature: None,
            }));
        }
        let mut objects = BTreeMap::new();
        for entry in self.entries.values() {
            match entry {
                Load::Ready(Data::Relations(relations)) => {
                    objects.extend(relations.iter().map(|relation| {
                        (
                            relation.object.clone(),
                            CatalogDraft {
                                object: DraftObject::Relation(relation.object.clone()),
                                detail: (!relation.size.is_empty()).then(|| relation.size.clone()),
                                signature: None,
                            },
                        )
                    }));
                }
                Load::Ready(Data::Routines(routines)) => {
                    objects.extend(routines.iter().map(|routine| {
                        (
                            routine.object.clone(),
                            CatalogDraft::routine(
                                routine.object.clone(),
                                &routine.arguments,
                                routine.return_type.as_deref(),
                            ),
                        )
                    }));
                }
                _ => {}
            }
        }
        for entry in self.entries.values() {
            let Load::Ready(Data::Index(index)) = entry else {
                continue;
            };
            for relation in &index.relations {
                if let Some(identity) = &relation.identity {
                    let object = ObjectRef::relation(project, identity.clone());
                    objects.entry(object.clone()).or_insert(CatalogDraft {
                        object: DraftObject::Relation(object),
                        detail: None,
                        signature: None,
                    });
                }
            }
            for function in &index.functions {
                if let Some(identity) = &function.identity {
                    let object = ObjectRef::routine(project, identity.clone());
                    objects.entry(object.clone()).or_insert_with(|| {
                        CatalogDraft::routine(
                            object,
                            &function.signature,
                            Some(&function.return_type),
                        )
                    });
                }
            }
        }
        rows.extend(objects.into_values());
        rows
    }
}
