use super::*;
use gpui_kit::component::menu::{ContextMenuItemStyle, PopupMenu, PopupMenuItem};

pub(super) type View = WeakEntity<Workspace>;

pub(super) fn action(
    view: &View,
    label: &'static str,
    icon: IconName,
    callback: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) -> PopupMenuItem {
    let view = view.clone();
    PopupMenuItem::new(label)
        .icon(icon)
        .on_click(move |_, w, cx| {
            let _ = view.update(cx, |v, cx| {
                if !v.navigation.busy {
                    callback(v, w, cx);
                }
            });
        })
}

fn new_query(view: &View, project: String) -> PopupMenuItem {
    action(view, "New Query", IconName::Plus, move |v, w, cx| {
        v.update_documents(w, cx, |docs| {
            docs.open(Kind::Query, Some(project.clone()));
        });
    })
}

fn edit(view: &View, profile: ConnectionProfile) -> PopupMenuItem {
    action(
        view,
        "Edit Connection",
        IconName::PenLine,
        move |v, w, cx| {
            let mut edit = profile.clone();
            if edit.name.is_empty() {
                edit.name = edit.id.clone();
            }
            v.open_edit_form(edit, w, cx);
        },
    )
}

fn admin_tool(
    view: &View,
    label: &'static str,
    kind: Kind,
    target: catalog::Capture,
) -> PopupMenuItem {
    let icon = match kind {
        Kind::Roles => IconName::Shield,
        Kind::PgSettings => IconName::Settings,
        Kind::Monitor => IconName::Activity,
        Kind::Notify => IconName::Zap,
        Kind::Extensions => IconName::Package,
        _ => IconName::List,
    };
    action(view, label, icon, move |v, w, cx| {
        v.open_admin(kind.clone(), &target, w, cx)
    })
}

pub(super) struct ServerMenu {
    pub ids: Vec<String>,
    pub first: ConnectionProfile,
    pub label: String,
    pub target: Option<catalog::Capture>,
}

impl ServerMenu {
    pub(super) fn render(
        &self,
        menu: PopupMenu,
        view: &View,
        previous_focus: Option<FocusHandle>,
    ) -> PopupMenu {
        let mut menu = menu.label("Server");
        if let Some(target) = &self.target {
            menu = menu
                .item(new_query(view, target.project.clone()))
                .item(admin_tool(
                    view,
                    "Performance Monitor",
                    Kind::Monitor,
                    target.clone(),
                ))
                .item(admin_tool(
                    view,
                    "PG Settings",
                    Kind::PgSettings,
                    target.clone(),
                ));
        }
        let first = self.first.clone();
        let ids = self.ids.clone();
        let add_focus = previous_focus.clone();
        menu = menu
            .item(action(
                view,
                "Add Database",
                IconName::Plus,
                move |v, w, cx| v.open_add_database(first.clone(), add_focus.clone(), w, cx),
            ))
            .item(action(
                view,
                "Rename Server",
                IconName::Pencil,
                move |v, w, cx| {
                    v.open_server_rename(ids.clone(), previous_focus.clone(), w, cx);
                },
            ));
        if self.ids.len() == 1 {
            menu = menu.item(edit(view, self.first.clone()));
        }
        let label = self.label.clone();
        let ids = self.ids.clone();
        menu.separator()
            .item(action(
                view,
                "Copy Name",
                IconName::Copy,
                move |_, _, cx| {
                    catalog::copy_name(label.clone(), cx);
                },
            ))
            .separator()
            .context_item(
                action(view, "Delete", IconName::Trash, move |v, _, cx| {
                    v.remove_projects(ids.clone(), cx)
                }),
                ContextMenuItemStyle {
                    destructive: true,
                    ..Default::default()
                },
            )
    }
}

pub(super) struct DatabaseMenu {
    pub profile: ConnectionProfile,
    pub status: Status,
    pub target: Option<catalog::Capture>,
}

impl DatabaseMenu {
    pub(super) fn render(&self, menu: PopupMenu, view: &View) -> PopupMenu {
        let project = self.profile.id.clone();
        let connect = self.profile.clone();
        let mut menu = menu
            .label("Database")
            .item(new_query(view, project.clone()))
            .item(action(
                view,
                if self.status == Status::Connected {
                    "Reconnect"
                } else {
                    "Connect"
                },
                IconName::RefreshCw,
                move |v, w, cx| v.connect_sidebar(connect.clone(), w, cx),
            ));
        if self.status == Status::Connected {
            let id = project.clone();
            menu = menu.item(action(
                view,
                "Refresh",
                IconName::RefreshCw,
                move |v, _, cx| v.refresh_connection(&id, cx),
            ));
            if let Some(target) = &self.target {
                let capture = target.clone();
                menu = menu
                    .item(admin_tool(
                        view,
                        "LISTEN/NOTIFY",
                        Kind::Notify,
                        target.clone(),
                    ))
                    .item(action(
                        view,
                        "Schema Diff",
                        IconName::Columns3,
                        move |v, w, cx| v.open_schema_diff(&capture, w, cx),
                    ))
                    .item(admin_tool(
                        view,
                        "Extensions",
                        Kind::Extensions,
                        target.clone(),
                    ))
                    .item(admin_tool(view, "Enum Types", Kind::Enums, target.clone()));
            }
        }
        let database = self.profile.database.clone();
        menu.separator()
            .item(edit(view, self.profile.clone()))
            .separator()
            .item(action(
                view,
                "Copy Name",
                IconName::Copy,
                move |_, _, cx| {
                    catalog::copy_name(database.clone(), cx);
                },
            ))
            .separator()
            .context_item(
                action(view, "Delete", IconName::Trash, move |v, _, cx| {
                    v.remove_projects(vec![project.clone()], cx)
                }),
                ContextMenuItemStyle {
                    destructive: true,
                    ..Default::default()
                },
            )
    }
}
