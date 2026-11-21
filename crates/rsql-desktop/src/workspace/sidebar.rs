mod add_database;
mod groups;
mod menus;
mod rename;
pub(super) mod row;

use super::{
    catalog::Catalog,
    form::{Form, Operation},
    theme::dialog,
    *,
};
use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    assets::IconName,
    component::{Icon, tooltip::Tooltip},
};
use rsql_desktop::{connection::ConnectionProfile, saved::SavedQueryKey};
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct Navigation {
    pub form: Option<Form>,
    pub serial: u64,
    pub modal_open: bool,
    pub catalog_serial: u64,
    pub catalogs: HashMap<String, Catalog>,
    pub status: HashMap<String, String>,
    pub(super) connected_routes: HashMap<String, rsql_core::services::ProjectTicket>,
    pub(super) busy: bool,
    pub(super) selected_saved_query: Option<SavedQueryKey>,
    message: String,
    collapsed: HashSet<(String, Branch)>,
}

impl Workspace {
    fn maintenance(
        &mut self,
        work: impl Future<Output = Result<(), String>> + Send + 'static,
        done: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.run_maintenance(
            work,
            move |view, result, cx| match result {
                Ok(()) => {
                    view.navigation.message.clear();
                    done(view, cx);
                }
                Err(error) => view.navigation.message = error,
            },
            cx,
        );
    }

    fn run_maintenance(
        &mut self,
        work: impl Future<Output = Result<(), String>> + Send + 'static,
        done: impl FnOnce(&mut Self, Result<(), String>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.busy {
            return;
        }
        self.navigation.busy = true;
        let job = self.session.spawn_profile_write(work);
        cx.spawn(async move |view, cx| {
            let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = view.update(cx, |v, cx| {
                v.navigation.busy = false;
                done(v, result, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn retire_disconnected_route(&mut self, id: &str, cx: &mut App) {
        self.session
            .documents
            .borrow_mut()
            .release_obsolete_results(id);
        if self
            .navigation
            .connected_routes
            .get(id)
            .is_some_and(|route| route.is_current())
        {
            return;
        }
        self.navigation.connected_routes.remove(id);
        self.invalidate_project_language(id, cx);
        self.navigation.catalogs.remove(id);
        self.navigation.status.remove(id);
    }

    pub(super) fn disconnect(&mut self, id: String, cx: &mut Context<Self>) {
        let svc = self.session.services.clone();
        let target = id.clone();
        self.maintenance(
            async move {
                svc.disconnect(&target).await;
                Ok(())
            },
            move |v, cx| {
                v.retire_disconnected_route(&id, cx);
            },
            cx,
        );
    }
    fn remove_projects(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let work = self.session.delete_projects(ids.clone());
        self.maintenance(
            work,
            move |v, cx| {
                for id in ids {
                    v.retire_disconnected_route(&id, cx);
                }
                v.capture_edits(cx);
                v.session.apply_removals();
                v.schedule_save(cx);
            },
            cx,
        );
    }
    pub(super) fn sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("workspace-sidebar")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(theme.sidebar_border)
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(
                div()
                    .id("connections-header")
                    .test_support()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .h(px(48.))
                    .px_3()
                    .border_b_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(theme::controls::tracked("CONNECTIONS", 10., 0.1)),
                    )
                    .child(
                        gpui_kit::base::Button::new("New connection")
                            .accessibility_label("New connection")
                            .size(px(24.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .disabled(self.navigation.busy)
                            .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                            .hover(|s| {
                                s.bg(if theme.is_dark() {
                                    white().opacity(0.06)
                                } else {
                                    black().opacity(0.04)
                                })
                            })
                            .child(Icon::new(IconName::Plus).size(px(12.)))
                            .tooltip(|w, cx| Tooltip::new("New connection").build(w, cx))
                            .on_click(cx.listener(|v, _, w, cx| {
                                v.open_form(ConnectionProfile::default(), w, cx)
                            })),
                    ),
            )
            .child(self.connections_sidebar(cx))
            .child(self.saved_sidebar(cx))
    }

    fn connections_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut side = div()
            .id("connections")
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .p_1();
        if let Some(err) = self.session.load_error.borrow().clone() {
            return side.child(err).child(self.button(
                "Retry loading connections",
                cx,
                |v, _, cx| {
                    let svc = v.session.services.clone();
                    let job = v
                        .session
                        .runtime
                        .spawn(async move { rsql_desktop::profile_service::profiles(&svc).await });
                    v.navigation.busy = true;
                    cx.spawn(async move |view, cx| {
                        let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
                        let _ = view.update(cx, |v, cx| {
                            v.navigation.busy = false;
                            match result {
                                Ok(rows) => {
                                    v.session.documents.borrow_mut().remap(&rows.legacy_ids);
                                    v.session.connections.replace(rows);
                                    v.session.load_error.take();
                                }
                                Err(e) => {
                                    v.session.load_error.replace(Some(e));
                                }
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                    cx.notify();
                },
            ));
        }
        if !self.navigation.message.is_empty() {
            side = side.child(
                div()
                    .px_2()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(self.navigation.message.clone()),
            );
        }
        for group in self.session.connections.borrow().groups() {
            side = side.child(self.server_group(&group, cx));
        }
        side
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Branch {
    Server,
    Databases,
    Database,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Disconnected,
    Connected,
    Connecting,
    Failed,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connected => "Connected",
            Self::Connecting => "Connecting",
            Self::Failed => "Failed",
        }
    }
}

impl Navigation {
    fn connection_status(&self, id: &str) -> Status {
        match self.status.get(id).map(String::as_str) {
            Some("Connected") => Status::Connected,
            Some("Connecting") => Status::Connecting,
            Some("Failed") => Status::Failed,
            _ => Status::Disconnected,
        }
    }

    fn expanded(&self, id: &str, branch: Branch) -> bool {
        !self.collapsed.contains(&(id.to_owned(), branch))
    }

    fn toggle(&mut self, id: String, branch: Branch) {
        if !self.collapsed.remove(&(id.clone(), branch)) {
            self.collapsed.insert((id, branch));
        }
    }
}

fn server_group_alias<'a>(profiles: impl Iterator<Item = &'a ConnectionProfile>) -> &'a str {
    profiles
        .map(|profile| profile.server_alias.as_str())
        .find(|alias| !alias.is_empty())
        .unwrap_or_default()
}

fn server_label(first: &ConnectionProfile, count: usize, alias: &str) -> String {
    if !alias.is_empty() {
        alias.into()
    } else if count == 1 {
        first.display_name().into()
    } else {
        format!("{}:{}", first.host, first.port)
    }
}

fn connection_hint(p: &ConnectionProfile, count: usize, alias: &str) -> Option<String> {
    let name = p.display_name();
    ((count > 1 || !alias.is_empty()) && name != p.database).then(|| name.to_owned())
}
