use super::*;
use crate::workspace::sidebar::row::{TreeRow, glyph, trailing_text};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme,
    label::Label,
    menu::{ContextMenuExt, PopupMenuItem},
};
impl Workspace {
    fn catalog_row(
        &self,
        p: &ConnectionProfile,
        node: Node,
        label: String,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        self.catalog_row_with_kind(p, node, label, false, None, cx)
    }

    fn catalog_row_with_kind(
        &self,
        p: &ConnectionProfile,
        node: Node,
        label: String,
        trigger: bool,
        size: Option<String>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let c = &self.navigation.catalogs[&p.id];
        let line = self.catalog_header(p, &node, label, trigger, size, cx);
        let mut row = div()
            .id(SharedString::from(format!("{node:?}")))
            .child(line);
        if let Some(Load::Failed(error)) = c.entries.get(&node) {
            let id = p.id.clone();
            let key = node.clone();
            row = row
                .child(error.clone())
                .child(self.button("Retry", cx, move |v, _, cx| {
                    v.load_node(&id, key.clone(), cx)
                }));
        }
        if !c.is_expanded(&node) {
            return row;
        }
        if let Node::Schema(schema) = &node {
            for (kind, label) in Section::LIST {
                let section = Node::Section(schema.clone(), kind);
                if kind == Section::Tables || c.ready_count(&section) != Some(0) {
                    row = row.child(self.catalog_row(p, section, label.into(), cx));
                }
            }
            return row;
        }
        if matches!(c.entries.get(&node), Some(Load::Failed(_))) {
            return row;
        }
        if let Node::Relation(object) = &node {
            if !object.kind.is_table() {
                return row;
            }
            for kind in [
                Child::Columns,
                Child::Indexes,
                Child::Constraints,
                Child::Triggers,
                Child::Rules,
                Child::Policies,
            ] {
                let section = Node::Children(object.clone(), kind);
                if kind == Child::Columns || c.ready_count(&section) != Some(0) {
                    let label = match kind {
                        Child::Policies => "RLS Policies".into(),
                        kind => format!("{kind:?}"),
                    };
                    row = row.child(self.catalog_row(p, section, label, cx));
                }
            }
            return row;
        }
        match c.entries.get(&node) {
            Some(Load::Ready(Data::Names(names))) if node == Node::Schemas => {
                for name in names {
                    row = row.child(self.catalog_row(
                        p,
                        Node::Schema(name.clone()),
                        name.clone(),
                        cx,
                    ));
                }
            }
            Some(Load::Ready(Data::Relations(relations))) => {
                for entry in relations {
                    row = row.child(self.catalog_row_with_kind(
                        p,
                        Node::Relation(entry.object.clone()),
                        entry.object.name.clone(),
                        false,
                        entry.object.kind.is_table().then(|| entry.size.clone()),
                        cx,
                    ));
                }
            }
            Some(Load::Ready(Data::Columns(columns))) => {
                if let Node::Children(object, _) = &node
                    && let Some(capture) = Capture::new(self, &p.id)
                {
                    for column in columns {
                        let owner = cx.weak_entity();
                        let capture = capture.clone();
                        let name = column.name.clone();
                        let mut leaf = TreeRow::new(
                            7,
                            column.name.clone(),
                            glyph(
                                if column.primary {
                                    IconName::Key
                                } else {
                                    IconName::Columns3
                                },
                                12.,
                                if column.primary {
                                    cx.theme().warning
                                } else {
                                    cx.theme().muted_foreground.opacity(0.6)
                                },
                            ),
                        );
                        leaf.detail = Some(column.ty.clone());
                        leaf.detail_badge = column
                            .nullable
                            .then_some(("NULL", cx.theme().muted_foreground.opacity(0.5)));
                        row =
                            row.child(
                                leaf.paint(format!("column-{}-{}", object.oid, column.number), cx)
                                    .context_menu(move |menu, w, _| {
                                        let menu = theme::controls::context_menu(menu, w);
                                        let (owner, capture, name) =
                                            (owner.clone(), capture.clone(), name.clone());
                                        menu.item(
                                            PopupMenuItem::element(|_, _| {
                                                Label::new("Copy Column Name")
                                            })
                                            .icon(IconName::Copy)
                                            .on_click(move |_, _, cx| {
                                                let _ = owner.update(cx, |v, cx| {
                                                    if capture.current(v) {
                                                        copy_name(name.clone(), cx);
                                                    }
                                                });
                                            }),
                                        )
                                    }),
                            );
                    }
                }
            }
            Some(Load::Ready(Data::Details(rows))) => {
                let icon = match node {
                    Node::Tablespaces => IconName::HardDrive,
                    Node::Children(_, Child::Constraints) => IconName::Link2,
                    Node::Children(_, Child::Triggers) => IconName::Zap,
                    Node::Children(_, Child::Rules) => IconName::ScrollText,
                    _ => IconName::Lock,
                };
                let tablespace = node == Node::Tablespaces;
                for (index, value) in rows.iter().enumerate() {
                    let mut leaf = TreeRow::new(
                        if tablespace { 2 } else { 7 },
                        value.name.clone(),
                        glyph(icon, 12., cx.theme().muted_foreground.opacity(0.6)),
                    );
                    leaf.detail = Some(value.detail.clone());
                    leaf.tooltip = value
                        .definition
                        .as_ref()
                        .map(|text| text.preview.to_string().into());
                    if tablespace && let Some(capture) = Capture::new(self, &p.id) {
                        let name = value.name.clone();
                        let owner = cx.weak_entity();
                        row = row.child(
                            leaf.paint(format!("tablespace-{}-{name}", p.id), cx)
                                .context_menu(move |menu, w, _| {
                                    let (owner, capture, name) =
                                        (owner.clone(), capture.clone(), name.clone());
                                    theme::controls::context_menu(menu, w).item(
                                        PopupMenuItem::new("Copy Name")
                                            .icon(IconName::Copy)
                                            .on_click(move |_, _, cx| {
                                                let _ = owner.update(cx, |v, cx| {
                                                    if capture.current(v) {
                                                        copy_name(name.clone(), cx);
                                                    }
                                                });
                                            }),
                                    )
                                }),
                        );
                    } else {
                        row = row.child(leaf.paint(("catalog-detail", index), cx));
                    }
                }
            }
            Some(Load::Ready(Data::Indexes(indexes))) => {
                let blue = gpui_kit::component::theme::oklch(0.623, 0.214, 259.815);
                for index in indexes {
                    let mut leaf = TreeRow::new(
                        7,
                        index.name.clone(),
                        glyph(
                            if index.unique && !index.primary {
                                IconName::Shield
                            } else {
                                IconName::Key
                            },
                            12.,
                            if index.primary {
                                cx.theme().warning
                            } else if index.unique {
                                blue
                            } else {
                                cx.theme().muted_foreground.opacity(0.6)
                            },
                        ),
                    );
                    leaf.detail = Some(format!("({})", index.columns.preview));
                    leaf.detail_badge = index.unique.then_some(("UNIQUE", blue.opacity(0.6)));
                    leaf.tooltip = Some(
                        format!(
                            "{} · {}\n{}",
                            index.name,
                            index.role(),
                            index.definition.preview
                        )
                        .into(),
                    );
                    row = row.child(
                        leaf.paint(format!("index-{}-{}", index.relation_oid, index.oid), cx),
                    );
                }
            }
            Some(Load::Ready(Data::Routines(routines))) => {
                let trigger = matches!(node, Node::Section(_, Section::TriggerFunctions));
                for entry in routines {
                    row = row.child(self.catalog_row_with_kind(
                        p,
                        Node::Relation(entry.object.clone()),
                        entry.object.name.clone(),
                        trigger,
                        Some(routine_detail(
                            &entry.arguments,
                            if trigger {
                                None
                            } else {
                                entry.return_type.as_deref()
                            },
                        )),
                        cx,
                    ));
                }
            }
            // The header row's spinner already shows a pending load, as in the legacy tree.
            _ => {}
        }
        row
    }

    fn catalog_header(
        &self,
        p: &ConnectionProfile,
        node: &Node,
        label: String,
        trigger: bool,
        size: Option<String>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let c = &self.navigation.catalogs[&p.id];
        let id = p.id.clone();
        let key = node.clone();
        let count = c.ready_count(node);
        let key_id = if let Node::Relation(o) = node {
            format!("object-{}-{}", p.id, o.oid)
        } else {
            format!("row-{node:?}")
        };
        let (name, depth) = match node {
            Node::Schema(_) | Node::Schemas => (IconName::FolderOpen, 3),
            Node::Section(_, Section::Tables) => (IconName::Table, 4),
            Node::Section(_, Section::Views) => (IconName::Eye, 4),
            Node::Section(_, Section::MaterializedViews) => (IconName::Layers, 4),
            Node::Section(_, Section::Functions) => (IconName::FileCode, 4),
            Node::Section(_, Section::TriggerFunctions) => (IconName::Zap, 4),
            Node::Children(_, Child::Triggers) => (IconName::Zap, 6),
            Node::Children(_, Child::Columns) => (IconName::Columns3, 6),
            Node::Children(_, Child::Indexes) => (IconName::Key, 6),
            Node::Children(_, Child::Constraints) => (IconName::Link2, 6),
            Node::Children(_, Child::Policies) => (IconName::Lock, 6),
            Node::Children(_, Child::Rules) => (IconName::ScrollText, 6),
            Node::Tablespaces => (IconName::HardDrive, 1),
            Node::Databases => (IconName::Database, 1),
            Node::Relation(o) => (
                match o.kind {
                    ObjectKind::View => IconName::Eye,
                    ObjectKind::MaterializedView => IconName::Layers,
                    kind if kind.is_table() => IconName::Table,
                    _ if trigger => IconName::Zap,
                    _ => IconName::FileCode,
                },
                5,
            ),
        };
        let expandable = !matches!(node, Node::Relation(o) if !o.kind.is_table());
        let mut row = TreeRow::new(depth, label, glyph(name, 14., cx.theme().muted_foreground));
        row.detail = count.map(|n| n.to_string());
        row.expanded = expandable.then(|| c.is_expanded(node));
        row.loading = matches!(c.entries.get(node), Some(Load::Pending(_)));
        row.selected = c.selected.as_ref() == Some(node);
        row.muted = matches!(
            node,
            Node::Children(..) | Node::Section(..) | Node::Tablespaces
        );
        let line = row
            .paint(key_id, cx)
            .children(size.map(|size| trailing_text(size, cx)));
        let capture = Capture::new(self, &p.id);
        let clicked = capture.clone();

        let line = line.on_click(cx.listener(move |v, e: &ClickEvent, w, cx| {
            if let Some(c) = v.navigation.catalogs.get_mut(&id) {
                c.selected = Some(key.clone());
            }
            if let Node::Relation(o) = &key
                && let Some(target) = clicked.as_ref()
                && e.click_count()
                    == if matches!(o.kind, ObjectKind::View | ObjectKind::MaterializedView) {
                        1
                    } else {
                        2
                    }
            {
                let action = if o.routine_identity.is_some() {
                    Action::Definition
                } else {
                    Action::Select
                };
                v.object_action(target, o, action, w, cx);
            } else if expandable {
                v.toggle_node(&id, key.clone(), cx);
            }
            cx.notify();
        }));
        let item = node.clone();
        let owner = cx.weak_entity();

        if matches!(node, Node::Relation(_) | Node::Schema(_))
            && let Some(target) = capture
        {
            line.context_menu(move |menu, w, cx| {
                let menu = theme::controls::context_menu(menu, w);
                let _ = owner.update(cx, |v, cx| {
                    if let Some(c) = v.navigation.catalogs.get_mut(&target.project) {
                        c.selected = Some(item.clone());
                    }
                    cx.notify();
                });
                menu::object_menu(menu, &item, &target, &owner)
            })
            .into_any_element()
        } else {
            line.into_any_element()
        }
    }

    pub(in crate::workspace) fn tree(&self, p: &ConnectionProfile, cx: &Context<Self>) -> Div {
        let catalog = &self.navigation.catalogs[&p.id];
        match catalog.schema_names() {
            Ok(Some(schemas)) => div().children(
                schemas
                    .iter()
                    .map(|name| self.catalog_row(p, Node::Schema(name.clone()), name.clone(), cx)),
            ),
            _ => div().child(self.catalog_row(p, Node::Schemas, "Schemas".into(), cx)),
        }
    }

    pub(in crate::workspace) fn tablespace_tree(
        &self,
        p: &ConnectionProfile,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div().id(format!("tablespace-branch-{}", p.id)).children(
            self.navigation
                .catalogs
                .contains_key(&p.id)
                .then(|| self.catalog_row(p, Node::Tablespaces, "Tablespaces".into(), cx)),
        )
    }
}
