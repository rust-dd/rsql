use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{ContextMenuItemStyle, PopupMenu, PopupMenuItem};

pub(super) fn object_menu(
    mut menu: PopupMenu,
    item: &Node,
    target: &Capture,
    owner: &WeakEntity<Workspace>,
) -> PopupMenu {
    if let Node::Schema(schema) = item {
        let view = owner.clone();
        let capture = target.clone();
        let schema = schema.clone();
        menu = menu.item(
            PopupMenuItem::new("ERD Diagram")
                .icon(IconName::Layers)
                .on_click(move |_, w, cx| {
                    let _ = view.update(cx, |v, cx| v.open_erd(&capture, &schema, w, cx));
                }),
        );
    }
    let is_table = matches!(item, Node::Relation(o) if o.kind.is_table());
    if is_table {
        menu = menu.label("Query");
    }
    let mut actions = match item {
        Node::Schema(_) => vec![Action::Copy, Action::NewQuery],
        Node::Relation(o) => Action::for_object(o),
        _ => vec![],
    };
    if let Some(import) = actions.iter().position(|action| *action == Action::Import)
        && let Some(properties) = actions
            .iter()
            .position(|action| *action == Action::Properties)
    {
        actions.swap(import, properties);
    }
    let mut previous = None;
    for action in actions {
        if let Node::Relation(_) = item
            && separated(previous, action)
        {
            menu = menu.separator();
        }
        previous = Some(action);
        let node = item.clone();
        let target = target.clone();
        let owner = owner.clone();
        let (label, icon) = entry(item, action);
        let button = PopupMenuItem::new(label)
            .icon(icon)
            .on_click(move |_, w, cx| {
                let _ = owner.update(cx, |v, cx| match &node {
                    Node::Relation(o) => v.object_action(&target, o, action, w, cx),
                    Node::Schema(s) if target.current(v) => {
                        if action == Action::Copy {
                            copy_name(s.clone(), cx);
                        } else {
                            open_draft(
                                v,
                                &target.project,
                                DraftObject::Schema(s.clone()).sql(),
                                w,
                                cx,
                            );
                        }
                    }
                    _ => {}
                });
            });
        menu = if is_table && action == Action::Copy {
            menu.context_item(
                button,
                ContextMenuItemStyle {
                    shortcut: Some(
                        if cfg!(target_os = "macos") {
                            "⌘C"
                        } else {
                            "Ctrl+C"
                        }
                        .into(),
                    ),
                    ..Default::default()
                },
            )
        } else {
            menu.item(button)
        };
    }
    menu
}

/// Source groups: queries, Import CSV, Properties with its DDL, then Copy Name.
fn separated(previous: Option<Action>, action: Action) -> bool {
    match action {
        Action::Import | Action::Copy => true,
        Action::Properties => previous.is_some_and(|p| p != Action::Definition),
        _ => false,
    }
}

fn entry(item: &Node, action: Action) -> (&'static str, IconName) {
    let select = match item {
        Node::Relation(o) if o.kind == ObjectKind::View => IconName::Eye,
        Node::Relation(o) if o.kind == ObjectKind::MaterializedView => IconName::Layers,
        _ => IconName::Table,
    };
    match (item, action) {
        (Node::Schema(_), Action::Copy) => ("Copy Schema Name", IconName::Copy),
        (Node::Schema(_), Action::NewQuery) => ("New Query", IconName::Plus),
        (_, Action::Select) => ("SELECT TOP 100", select),
        (_, Action::Definition) => ("Show Definition", IconName::FileCode),
        (_, Action::Copy) => ("Copy Name", IconName::Copy),
        (_, Action::Count | Action::Latest) => (action.label(), IconName::Table),
        (_, Action::Import) => (action.label(), IconName::FileUp),
        (_, Action::Properties) => (action.label(), IconName::Settings2),
        (_, Action::Refresh) => (action.label(), IconName::RefreshCw),
        _ => (action.label(), IconName::FileCode),
    }
}
