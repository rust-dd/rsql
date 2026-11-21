mod divider;
mod footer;
mod header;
mod picker;
mod tab_menu;
mod tabs;

use super::*;
use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme, Icon, label::Label},
    prelude::FluentBuilder,
};

fn control(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .tab_index(0)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .whitespace_nowrap()
        .focus_visible(|s| {
            s.shadow(vec![
                BoxShadow::new(px(0.), px(0.), cx.theme().ring).spread_radius(px(2.)),
            ])
        })
        .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
}

fn ghost(
    id: &'static str,
    label: &'static str,
    icon: IconName,
    size: f32,
    glyph: f32,
    cx: &App,
) -> Stateful<Div> {
    control(id, label, cx)
        .size(px(size))
        .flex_shrink_0()
        .rounded(px(8.))
        .hover(|s| {
            s.bg(cx.theme().accent)
                .text_color(cx.theme().accent_foreground)
        })
        .child(Icon::new(icon).size(px(glyph)))
}

impl Workspace {
    pub(super) fn shell(&self, w: &mut Window, cx: &mut Context<Self>) -> Div {
        let docs = self.session.documents.borrow();
        let profiles = self.session.connections.borrow();
        let project = docs
            .active()
            .and_then(|t| t.project.as_ref())
            .and_then(|id| profiles.profiles.iter().find(|p| &p.id == id));
        let status = project
            .and_then(|p| self.navigation.status.get(&p.id))
            .map(String::as_str)
            .unwrap_or("Disconnected");
        let mut panes = div().flex().flex_1().min_h_0().overflow_hidden();
        let query = docs
            .active()
            .filter(|tab| tab.kind == Kind::Query && self.editor(Side::Main).is_some());
        let window_width = f32::from(w.viewport_size().width);
        let width = self.sidebar_width.min((window_width - 180.).max(0.));
        if let Some(tab) = query {
            for side in [Side::Main, Side::Right] {
                let Some(state) = self.editor(side) else {
                    continue;
                };
                let editor = self.language_editor(state, side, w, cx);
                let toolbar = (side == Side::Right)
                    .then(|| self.query_toolbar(tab, side, project.is_some(), false, cx));
                // Unsplit, the legacy editor took a share of the whole column under the top bar.
                let column = (!tab.split).then(|| w.viewport_size().height - px(44. + 28.));
                panes = panes.child(self.editor_split(
                    side,
                    toolbar,
                    editor,
                    self.result_view(tab, side, cx),
                    column,
                    cx,
                ));
            }
        }
        if let Some(tab) = docs.active().filter(|t| matches!(t.kind, Kind::Erd(_))) {
            panes = match self.erd_panels.get(&tab.id) {
                Some(panel) => panes.child(panel.clone()),
                None => panes.items_center().justify_center().child(Label::new(
                    "Connect this project to view its schema diagram",
                )),
            };
        }
        if let Some(tab) = docs.active().filter(|t| t.kind == Kind::SchemaDiff) {
            panes = match self.schema_panels.get(&tab.id) {
                Some(panel) => panes.child(panel.clone()),
                None => panes
                    .items_center()
                    .justify_center()
                    .child("Connect this project to compare schemas"),
            };
        }
        if let Some(tab) = docs.active().filter(|t| admin::name(&t.kind).is_some()) {
            panes = match self.admin_panel(&tab.id) {
                Some(panel) => panes.child(panel.clone()),
                None => panes.items_center().justify_center().child(
                    gpui_kit::component::label::Label::new(
                        "Connect this project to view administration details",
                    ),
                ),
            };
        }
        if let Some(tab) = docs.active().filter(|t| t.kind == Kind::Notify) {
            panes = match self.notify_panels.get(&tab.id) {
                Some(panel) => panes.child(panel.clone()),
                None => panes.items_center().justify_center().child(Label::new(
                    "Connect this project to listen for notifications",
                )),
            };
        }
        if let Some(tab) = docs.active().filter(|t| t.kind == Kind::Monitor) {
            panes = match self.monitors.get(&tab.id).filter(|p| !p.read(cx).retired) {
                Some(panel) => panes.child(panel.clone()),
                None => panes.items_center().justify_center().child(Label::new(
                    "Connect this project to view performance statistics",
                )),
            };
        }
        if let Some(panel) = docs.active().and_then(|t| self.terminal_panels.get(&t.id)) {
            panes = match panel {
                Ok(view) => panes.child(view.clone()),
                Err(error) => panes.items_center().justify_center().child(error.clone()),
            };
        }
        if docs.tabs.is_empty() {
            panes = panes
                .items_center()
                .justify_center()
                .child(self.empty_state(cx));
        }
        let center = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.tab_bar(cx))
            .children(query.map(|tab| {
                self.query_toolbar(
                    tab,
                    Side::Main,
                    project.is_some(),
                    window_width - width - 1. >= 960.,
                    cx,
                )
            }))
            .child(panes);
        let body = div()
            .flex()
            .flex_1()
            .min_h_0()
            .relative()
            .child(
                div()
                    .w(px(width))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(self.sidebar(cx)),
            )
            .child(center.ml(px(1.)))
            .child(self.sidebar_divider(width, cx));
        let (history_error, history_status) = self.session.history_footer();
        let save_error = self.session.save_status().borrow().clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.header(project, status, w, cx))
            .children(self.update_panel(cx))
            .child(body)
            .children(footer::errors(save_error, history_error, cx))
            .child(footer::render(
                project,
                status,
                self.session.resources.latest(),
                history_status,
                cx,
            ))
    }
    fn empty_state(&self, cx: &Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.))
            .child(
                Label::new("RSQL")
                    .id("empty-brand")
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(60.))
                    .line_height(px(60.))
                    .text_color(cx.theme().muted_foreground.opacity(0.4)),
            )
            .child(
                Label::new("No tabs open")
                    .id("empty-description")
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .text_color(cx.theme().muted_foreground.opacity(0.6)),
            )
            .child(
                control("empty-new-query", "New Query", cx)
                    .rounded(px(8.))
                    .px(px(16.))
                    .py(px(8.))
                    .gap(px(8.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .bg(cx.theme().primary.opacity(0.1))
                    .text_color(cx.theme().primary)
                    .hover(|s| s.bg(cx.theme().primary.opacity(0.2)))
                    .child(Icon::new(IconName::Plus).size(px(18.)))
                    .child("New Query")
                    .on_click(Self::shell_action(cx, Self::new_query))
                    .test_support(),
            )
    }
}
