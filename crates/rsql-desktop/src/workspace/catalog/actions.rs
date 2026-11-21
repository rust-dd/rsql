use super::*;
use rsql_core::services::ProjectTicket;
use rsql_desktop::metadata::properties::qualified;

#[derive(Clone)]
pub(in crate::workspace) struct Capture {
    pub(in crate::workspace) project: String,
    epoch: u64,
    pub(in crate::workspace) generation: watch::Receiver<()>,
    pub(in crate::workspace) route: ProjectTicket,
}
impl PartialEq for Capture {
    fn eq(&self, other: &Self) -> bool {
        self.project == other.project
            && self.epoch == other.epoch
            && self.generation.same_channel(&other.generation)
            && self.route.same_generation(&other.route)
    }
}
impl Eq for Capture {}

impl Capture {
    pub(in crate::workspace) fn new(v: &Workspace, project: &str) -> Option<Self> {
        let route = v.navigation.connected_routes.get(project)?.clone();
        let catalog = v.navigation.catalogs.get(project)?;
        route.is_current().then(|| Self {
            project: project.into(),
            epoch: catalog.epoch,
            generation: catalog.generation.subscribe(),
            route,
        })
    }
    pub(in crate::workspace) fn current_route(&self, v: &Workspace) -> bool {
        self.route.is_current()
            && v.navigation
                .connected_routes
                .get(&self.project)
                .is_some_and(|r| r.same_generation(&self.route))
            && !*v.session.preflight_stop.borrow()
    }

    pub(in crate::workspace) fn current(&self, v: &Workspace) -> bool {
        self.current_route(v)
            && matches!(self.generation.has_changed(), Ok(false))
            && v.navigation
                .catalogs
                .get(&self.project)
                .is_some_and(|c| c.epoch == self.epoch)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum Action {
    Select,
    Count,
    Latest,
    Properties,
    CreateTable,
    CreateView,
    Definition,
    Refresh,
    Import,
    Copy,
    NewQuery,
}
impl Action {
    pub(in crate::workspace) fn label(self) -> &'static str {
        match self {
            Self::Select => "SELECT 100",
            Self::Count => "SELECT COUNT(*)",
            Self::Latest => "Latest 10",
            Self::Properties => "Properties",
            Self::Definition => "Show definition",
            Self::CreateTable => "Show CREATE TABLE",
            Self::CreateView => "Show CREATE VIEW",
            Self::Refresh => "REFRESH",
            Self::Import => "Import CSV",
            Self::Copy => "Copy name",
            Self::NewQuery => "New query",
        }
    }
    pub(in crate::workspace) fn for_object(o: &ObjectRef) -> Vec<Self> {
        use Action::*;
        match o.kind {
            ObjectKind::View => vec![Select, Properties, CreateView, Copy],
            ObjectKind::MaterializedView => vec![Select, Refresh, Properties, Copy],
            ObjectKind::ForeignTable => vec![Select, Count, Properties, Copy],
            ObjectKind::PartitionedTable | ObjectKind::Table => {
                vec![Select, Count, Properties, Import, CreateTable, Copy]
            }
            _ => vec![Definition, Properties, Copy],
        }
    }
}

pub(in crate::workspace) fn select_sql(schema: &str, name: &str) -> String {
    format!("SELECT * FROM {} LIMIT 100;", qualified(schema, name))
}
pub(in crate::workspace) fn open_draft(
    v: &mut Workspace,
    project: &str,
    sql: String,
    w: &mut Window,
    cx: &mut Context<Workspace>,
) {
    v.update_documents(w, cx, |d| {
        d.open(Kind::Query, Some(project.into())).main.draft.text = sql
    });
}
pub(in crate::workspace) fn copy_name(text: String, cx: &mut App) {
    clipboard_order::ClipboardWrite::begin(cx);
    crate::desktop::DesktopServices::write_clipboard(cx, ClipboardItem::new_string(text));
}

impl Workspace {
    pub(in crate::workspace) fn object_action(
        &mut self,
        target: &Capture,
        o: &ObjectRef,
        action: Action,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !target.current(self) || target.project != o.project {
            return;
        }
        let name = qualified(&o.schema, &o.name);
        let sql = match action {
            Action::Select => DraftObject::Relation(o.clone()).sql(),
            Action::Latest => format!("SELECT * FROM {name} ORDER BY 1 DESC LIMIT 10;"),
            Action::Count => format!("SELECT COUNT(*) FROM {name};"),
            Action::Refresh => format!("REFRESH MATERIALIZED VIEW {name};"),
            Action::Definition => DraftObject::Function(o.clone()).sql(),
            Action::Copy => return copy_name(name, cx),
            Action::Import => return self.open_import(target, o, w, cx),
            Action::Properties => return self.open_properties(target.clone(), o.clone(), w, cx),
            Action::CreateTable | Action::CreateView => definition_lookup(o),
            Action::NewQuery => return,
        };
        open_draft(self, &o.project, sql, w, cx);
    }
}

fn definition_lookup(object: &ObjectRef) -> String {
    let (kind, value) = match object.kind {
        ObjectKind::View => (
            "v",
            "pg_catalog.pg_get_viewdef(c.oid, true) AS view_definition",
        ),
        kind => (
            if kind == ObjectKind::PartitionedTable {
                "p"
            } else {
                "r"
            },
            r#"pg_catalog.format('CREATE TABLE %I.%I (%s);', n.nspname, c.relname,
  COALESCE((SELECT pg_catalog.string_agg(pg_catalog.format('%I %s%s%s',
    a.attname, pg_catalog.format_type(a.atttypid, a.atttypmod),
    CASE WHEN a.attnotnull THEN ' NOT NULL' ELSE '' END,
    COALESCE(' DEFAULT ' || v.column_default, '')),
    ', ' ORDER BY a.attnum)
    FROM pg_catalog.pg_attribute a
    JOIN information_schema.columns v ON v.table_schema = n.nspname AND v.table_name = c.relname AND v.ordinal_position = a.attnum
    WHERE a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped), '')) AS ddl"#,
        ),
    };
    format!(
        "SELECT {value}
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
WHERE c.oid = {} AND c.relkind = '{kind}' AND n.nspname = {} AND c.relname = {};",
        object.oid,
        literal(&object.schema),
        literal(&object.name)
    )
}
