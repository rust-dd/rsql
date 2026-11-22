use super::menus::{DatabaseMenu, ServerMenu, action};
use super::*;
use gpui_kit::component::{Sizable, button::ButtonVariants, menu::ContextMenuExt};
use row::{TreeRow, glyph, trailing};
use std::collections::BTreeMap;

impl Workspace {
    pub(in crate::workspace) fn connect_sidebar(
        &mut self,
        mut profile: ConnectionProfile,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.busy || self.navigation.form.is_some() {
            return;
        }
        if profile.name.is_empty() {
            profile.name = profile.id.clone();
        }
        self.open_reconnect_form(profile, w, cx);
        self.submit(Operation::Connect, cx);
    }

    pub(super) fn server_group(
        &self,
        group: &[&ConnectionProfile],
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let first = group[0];
        let alias = server_group_alias(group.iter().copied());
        let name = server_label(first, group.len(), alias);
        let id = first.id.clone();
        let theme = cx.theme();
        let connected = group
            .iter()
            .copied()
            .find(|p| self.navigation.connection_status(&p.id) == Status::Connected);
        let status = if connected.is_some() {
            Status::Connected
        } else if group
            .iter()
            .any(|p| self.navigation.connection_status(&p.id) == Status::Connecting)
        {
            Status::Connecting
        } else {
            Status::Disconnected
        };
        let target = connected.and_then(|p| catalog::Capture::new(self, &p.id));
        let menu_items = ServerMenu {
            ids: group.iter().map(|p| p.id.clone()).collect(),
            first: first.clone(),
            label: name.clone(),
            target: target.clone(),
        };
        let owner = cx.weak_entity();
        let open = self.navigation.expanded(&id, Branch::Server);
        let mut row = TreeRow::new(0, name, glyph(IconName::Server, 14., theme.primary));
        row.bold = true;
        row.expanded = Some(open);
        let server = row
            .paint(id.clone(), cx)
            .child(trailing(
                (!alias.is_empty()).then(|| format!("{}:{}", first.host, first.port)),
                Some(status),
                cx,
            ))
            .on_click(cx.listener(move |v, _, _, cx| {
                v.navigation.toggle(id.clone(), Branch::Server);
                cx.notify();
            }))
            .context_menu(move |menu, w, cx| {
                menu_items.render(
                    theme::controls::context_menu(menu, w),
                    &owner,
                    w.focused(cx),
                )
            });
        let mut block = div().child(server);
        if !open {
            return block;
        }
        let id = first.id.clone();
        let databases_open = self.navigation.expanded(&id, Branch::Databases);
        let mut databases_by_name = BTreeMap::<&str, Vec<(&ConnectionProfile, bool)>>::new();
        for profile in group {
            databases_by_name
                .entry(&profile.database)
                .or_default()
                .push((profile, false));
        }
        let mut discovery_status = Vec::new();
        for profile in group
            .iter()
            .copied()
            .filter(|p| self.navigation.connection_status(&p.id) == Status::Connected)
        {
            let Some(catalog) = self.navigation.catalogs.get(&profile.id) else {
                continue;
            };
            match catalog.databases() {
                Ok(Some(names)) => {
                    for name in names {
                        databases_by_name
                            .entry(name)
                            .or_insert_with(|| vec![(profile, true)]);
                    }
                }
                state => discovery_status.push((profile, state.err())),
            }
        }
        let mut databases = TreeRow::new(
            1,
            "Databases",
            glyph(IconName::Database, 14., theme.muted_foreground),
        );
        databases.muted = true;
        databases.detail = Some(
            databases_by_name
                .values()
                .map(Vec::len)
                .sum::<usize>()
                .to_string(),
        );
        databases.expanded = Some(databases_open);
        databases.loading = discovery_status.iter().any(|(_, error)| error.is_none());
        block = block.child(
            databases
                .paint(format!("databases-{}", first.id), cx)
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.navigation.toggle(id.clone(), Branch::Databases);
                    cx.notify();
                })),
        );
        if databases_open {
            for (name, profiles) in databases_by_name {
                for (profile, discovered) in profiles {
                    block = if discovered {
                        block.children(self.discovered_database_row(profile, name, cx))
                    } else {
                        block.child(self.database_row(profile, group.len(), alias, cx))
                    };
                }
            }
            for (profile, error) in discovery_status {
                let Some(error) = error else { continue };
                let mut row = div()
                    .id(format!("database-discovery-{}", profile.id))
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(28.))
                    .text_xs()
                    .text_color(theme.danger);
                let message = format!("{}: {error}", profile.display_name());
                row = row
                    .aria_label(message.clone())
                    .child(div().min_w_0().truncate().child(message));
                if let Some(target) = catalog::Capture::new(self, &profile.id) {
                    row = row.child(
                        self.button("Retry", cx, move |v, _, cx| {
                            v.retry_database_discovery(&target, cx)
                        })
                        .small()
                        .ghost(),
                    );
                }
                block = block.child(row);
            }
        }
        let roles = TreeRow::new(
            1,
            "Login/Group Roles",
            glyph(IconName::Shield, 14., theme.muted_foreground),
        );
        let first_profile = first.clone();
        block = block.child(
            roles
                .paint(format!("roles-{}", first.id), cx)
                .on_click(cx.listener(move |v, _, w, cx| {
                    if let Some(target) = &target {
                        v.open_admin(Kind::Roles, target, w, cx);
                    } else {
                        v.connect_sidebar(first_profile.clone(), w, cx);
                    }
                })),
        );
        if let Some(profile) = connected {
            block.child(self.tablespace_tree(profile, cx))
        } else {
            let mut tablespaces = TreeRow::new(
                1,
                "Tablespaces",
                glyph(IconName::HardDrive, 14., theme.muted_foreground),
            );
            tablespaces.muted = true;
            tablespaces.expanded = Some(false);
            let first = first.clone();
            block.child(
                tablespaces
                    .paint(format!("tablespaces-{}", first.id), cx)
                    .on_click(
                        cx.listener(move |v, _, w, cx| v.connect_sidebar(first.clone(), w, cx)),
                    ),
            )
        }
    }

    fn discovered_database_row(
        &self,
        source: &ConnectionProfile,
        name: &str,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let target = catalog::Capture::new(self, &source.id)?;
        let row_id = format!("discovered-{}-{name}", source.id);
        let name = name.to_owned();
        let mut row = TreeRow::new(
            2,
            name.clone(),
            glyph(
                IconName::Database,
                14.,
                cx.theme().muted_foreground.opacity(0.5),
            ),
        );
        row.muted = true;
        let view = cx.weak_entity();
        let source = source.clone();
        let copy = name.clone();
        let copy_target = target.clone();
        let save = move |v: &mut Self, w: &mut Window, cx: &mut Context<Self>| {
            if target.current(v) && !v.navigation.busy && v.navigation.form.is_none() {
                v.open_form(source.clone_database(&name), w, cx);
                v.submit(Operation::Save, cx);
            }
        };
        let click = save.clone();
        Some(
            row.paint(row_id, cx)
                .on_click(cx.listener(move |v, _, w, cx| click(v, w, cx)))
                .context_menu(move |menu, w, _| {
                    let name = copy.clone();
                    let capture = copy_target.clone();
                    theme::controls::context_menu(menu, w)
                        .item(action(&view, "Connect", IconName::Link2, save.clone()))
                        .item(action(
                            &view,
                            "Copy Name",
                            IconName::Copy,
                            move |v, _, cx| {
                                if capture.current(v) {
                                    catalog::copy_name(name.clone(), cx);
                                }
                            },
                        ))
                }),
        )
    }

    fn database_row(
        &self,
        p: &ConnectionProfile,
        count: usize,
        alias: &str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let status = self.navigation.connection_status(&p.id);
        let connected = status == Status::Connected;
        let open = self.navigation.expanded(&p.id, Branch::Database);
        let color = cx.theme().muted_foreground;
        let icon = if status == Status::Connecting {
            IconName::LoaderCircle
        } else {
            IconName::Database
        };
        let mut row = TreeRow::new(2, p.database.clone(), glyph(icon, 14., color));
        row.expanded = connected.then_some(open);
        let profile = p.clone();
        let items = DatabaseMenu {
            profile: p.clone(),
            status,
            target: catalog::Capture::new(self, &p.id),
        };
        let view = cx.weak_entity();
        let row = row
            .paint(format!("db-{}", p.id), cx)
            .aria_label(format!("{} · {}", p.database, status.label()))
            .child(trailing(connection_hint(p, count, alias), Some(status), cx))
            .on_click(cx.listener(move |v, _, w, cx| {
                match v.navigation.connection_status(&profile.id) {
                    Status::Connected => {
                        v.navigation.toggle(profile.id.clone(), Branch::Database);
                        cx.notify();
                    }
                    Status::Connecting => {}
                    _ => v.connect_sidebar(profile.clone(), w, cx),
                }
            }))
            .context_menu(move |menu, w, _| {
                items.render(theme::controls::context_menu(menu, w), &view)
            });
        div()
            .id(format!("database-branch-{}", p.id))
            .child(row)
            .children(
                (open && connected && self.navigation.catalogs.contains_key(&p.id))
                    .then(|| self.tree(p, cx)),
            )
    }
}
