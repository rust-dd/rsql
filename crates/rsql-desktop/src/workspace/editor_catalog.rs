use super::{Catalog, Data, Load, Node, Workspace};
use gpui_kit::Context;
use rsql_desktop::completion::{CatalogSnapshot, CompletionContext, ExpectedKind};
use tokio::sync::watch;

pub(in crate::workspace) struct CatalogRead {
    pub snapshot: CatalogSnapshot,
    pub wait: Option<watch::Receiver<bool>>,
    pub incomplete: bool,
    pub discovering: bool,
}

impl Catalog {
    fn required_node(
        &self,
        context: &CompletionContext,
        snapshot: &CatalogSnapshot,
    ) -> Option<Node> {
        if !context.enabled || (context.kinds.is_empty() && context.qualifier.is_empty()) {
            return None;
        }
        if !matches!(
            self.entries.get(&Node::Schemas),
            Some(Load::Ready(Data::Names(_)))
        ) {
            return Some(Node::Schemas);
        }
        needed(context, snapshot).map(Node::Schema)
    }

    fn snapshot(&self) -> CatalogSnapshot {
        let schemas = match self.entries.get(&Node::Schemas) {
            Some(Load::Ready(Data::Names(names))) => names.clone(),
            _ => vec![],
        };
        let default_schema = schemas
            .iter()
            .find(|s| s.as_str() == "public")
            .or(schemas.first())
            .map(String::as_str)
            .unwrap_or("public")
            .into();
        let indexes = self
            .entries
            .iter()
            .filter_map(|(key, entry)| match (key, entry) {
                (Node::Schema(name), Load::Ready(Data::Index(index))) => {
                    Some((name.clone(), index.clone()))
                }
                _ => None,
            })
            .collect();
        CatalogSnapshot {
            schemas,
            default_schema,
            indexes,
        }
    }
}

fn needed(context: &CompletionContext, snapshot: &CatalogSnapshot) -> Option<String> {
    if !context.enabled {
        return None;
    }
    if let Some((name, prefix)) = context.qualifier.split_last() {
        if !context
            .kinds
            .iter()
            .any(|k| matches!(k, ExpectedKind::Table | ExpectedKind::View))
        {
            if let Some(schema) = prefix.last() {
                return Some(schema.clone());
            }
            if let Some(source) = context.scope.iter().find(|s| s.binding() == name) {
                return (!source.local).then(|| {
                    source
                        .schema
                        .as_ref()
                        .unwrap_or(&snapshot.default_schema)
                        .clone()
                });
            }
        }
        return snapshot.schemas.iter().find(|s| *s == name).cloned();
    }
    context
        .kinds
        .iter()
        .any(|k| {
            matches!(
                k,
                ExpectedKind::Column
                    | ExpectedKind::Table
                    | ExpectedKind::View
                    | ExpectedKind::Function
            )
        })
        .then(|| snapshot.default_schema.clone())
}

impl Workspace {
    pub(in crate::workspace) fn editor_catalog(
        &mut self,
        project: Option<&str>,
        context: &CompletionContext,
        cx: &mut Context<Self>,
    ) -> CatalogRead {
        let mut result = CatalogRead {
            snapshot: Default::default(),
            wait: None,
            incomplete: false,
            discovering: false,
        };
        let Some((id, catalog)) =
            project.and_then(|id| self.navigation.catalogs.get_mut(id).map(|c| (id, c)))
        else {
            return result;
        };
        result.snapshot = catalog.snapshot();
        let Some(node) = catalog.required_node(context, &result.snapshot) else {
            return result;
        };
        result.discovering = node == Node::Schemas;
        let entry = catalog.entries.get(&node);
        if matches!(entry, Some(Load::Ready(_))) {
            return result;
        }
        result.incomplete = true;
        let first = catalog.editor_attempts.insert(node.clone());
        let wait = first && !matches!(entry, Some(Load::Failed(_)));
        let starts_background = !wait && !matches!(entry, Some(Load::Pending(_)));
        self.load_node(id, node.clone(), cx);
        if let Some(Load::Pending(load)) = self
            .navigation
            .catalogs
            .get_mut(id)
            .and_then(|c| c.entries.get_mut(&node))
        {
            if starts_background {
                load.background_retry = true;
            }
            if wait {
                result.wait = Some(load.published.subscribe());
            }
        }
        result
    }

    pub(in crate::workspace) fn read_editor_catalog(
        &self,
        project: Option<&str>,
        context: &CompletionContext,
    ) -> (CatalogSnapshot, bool) {
        let Some(catalog) = project.and_then(|id| self.navigation.catalogs.get(id)) else {
            return (Default::default(), false);
        };
        let snapshot = catalog.snapshot();
        let waiting = catalog
            .required_node(context, &snapshot)
            .is_some_and(|node| !matches!(catalog.entries.get(&node), Some(Load::Ready(_))));
        (snapshot, waiting)
    }
}
