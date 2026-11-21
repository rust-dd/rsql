use super::*;

use gpui_kit::component::menu::ContextMenuExt;
use rsql_desktop::document::Tab;

impl Workspace {
    pub(super) fn tab_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let docs = self.session.documents.borrow();
        let profiles = self.session.connections.borrow();
        let mut list = div()
            .id("tab-list")
            .role(Role::TabList)
            .test_support()
            .flex()
            .items_center()
            .flex_1()
            .min_w_0()
            .overflow_x_scroll()
            .overflow_y_hidden()
            .gap(px(2.))
            .px(px(6.))
            .py(px(4.));
        for (index, tab) in docs.tabs.iter().enumerate() {
            let profile = tab
                .project
                .as_ref()
                .and_then(|id| profiles.profiles.iter().find(|p| &p.id == id))
                .filter(|p| tab.kind != Kind::Terminal && !p.database.is_empty());
            let prefix = profile
                .filter(|_| tab.kind != Kind::Monitor)
                .map(|p| p.display_name());
            let name = prefix.map_or_else(|| tab.title.clone(), |p| format!("{p}: {}", tab.title));
            let active = docs.selected == Some(index);
            let group = SharedString::from(format!("tab-{}", tab.id));
            let select = tab.id.clone();
            let close = tab.id.clone();
            let menu_id = tab.id.clone();
            let others = docs.tabs.len() > 1;
            let owner = cx.weak_entity();
            let close_button = control("close-tab", format!("Close {name}"), cx)
                .p(px(2.))
                .rounded(px(6.))
                .opacity(0.)
                .group_hover(group.clone(), |s| s.opacity(1.))
                .focus(|s| s.opacity(1.))
                .hover(|s| {
                    s.bg(cx.theme().danger.opacity(0.2))
                        .text_color(cx.theme().danger)
                })
                .child(Icon::new(IconName::X).size(px(12.)))
                .on_click(cx.listener(move |v, _, w, cx| {
                    cx.stop_propagation();
                    v.update_documents(w, cx, |d| d.close(&close))
                }))
                .test_support();
            list = list.child(
                control(group.clone(), name, cx)
                    .role(Role::Tab)
                    .aria_selected(active)
                    .group(group)
                    .flex_shrink_0()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .when(active, |s| s.bg(cx.theme().accent.opacity(0.8)).shadow_sm())
                    .when(!active, |s| {
                        s.text_color(cx.theme().muted_foreground).hover(|s| {
                            s.bg(cx.theme().accent.opacity(0.4))
                                .text_color(cx.theme().foreground)
                        })
                    })
                    .child(caption(tab, profile.is_some(), prefix, active, cx))
                    .child(close_button)
                    .on_click(cx.listener(move |v, _, w, cx| {
                        v.update_documents(w, cx, |d| d.select(&select))
                    }))
                    .test_support()
                    .context_menu(move |menu, w, cx| {
                        tab_menu::commands(
                            theme::controls::context_menu(menu, w),
                            owner.clone(),
                            menu_id.clone(),
                            others,
                            cx,
                        )
                    }),
            );
        }
        div()
            .id("tab-bar")
            .test_support()
            .flex()
            .items_center()
            .min_h(px(40.))
            .flex_shrink_0()
            .bg(cx.theme().group_box.opacity(0.4))
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.4))
            .child(list)
            .child(
                ghost("new-query", "New query tab", IconName::Plus, 28., 14., cx)
                    .mr(px(2.))
                    .on_click(Self::shell_action(cx, Self::new_query))
                    .test_support(),
            )
            .child(
                ghost(
                    "open-terminal",
                    "Open Terminal",
                    IconName::Terminal,
                    28.,
                    14.,
                    cx,
                )
                .mr(px(6.))
                .tooltip(|w, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(format!(
                        "Open Terminal ({})",
                        terminal_tabs::SHORTCUT
                    ))
                    .build(w, cx)
                })
                .on_click(cx.listener(|v, _, w, cx| v.dispatch_open_terminal(w, cx)))
                .test_support(),
            )
    }
}

fn caption(
    tab: &Tab,
    has_database: bool,
    prefix: Option<&str>,
    active: bool,
    cx: &App,
) -> impl IntoElement {
    let icon = match tab.kind {
        Kind::Terminal => Some(IconName::Terminal),
        Kind::Monitor => Some(IconName::Activity),
        Kind::Notify => Some(IconName::Bell),
        Kind::Roles => Some(IconName::Shield),
        Kind::SchemaDiff => Some(IconName::Columns3),
        Kind::Extensions => Some(IconName::Package),
        Kind::Enums => Some(IconName::List),
        Kind::PgSettings => Some(IconName::Settings),
        Kind::Query | Kind::Erd(_) => has_database.then_some(IconName::Database),
    };
    let color = if active {
        cx.theme().primary
    } else {
        cx.theme().muted_foreground
    };
    div()
        .id("tab-caption")
        .test_support()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .line_height(px(16.))
        .when_some(icon, |s, name| {
            let color = if name == IconName::Database {
                color.opacity(if active { 0.7 } else { 0.6 })
            } else {
                color
            };
            s.child(Icon::new(name).size(px(12.)).text_color(color))
        })
        .when_some(prefix, |s, p| {
            s.child(
                div()
                    .text_color(cx.theme().muted_foreground.opacity(0.7))
                    .child(format!("{p}:")),
            )
        })
        .child(
            div()
                .when(active, |s| s.font_weight(FontWeight::MEDIUM))
                .child(tab.title.clone()),
        )
}
