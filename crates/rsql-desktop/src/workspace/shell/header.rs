use super::*;
use gpui_kit::component::theme::oklch;
use rsql_desktop::connection::ConnectionProfile;

impl Workspace {
    pub(super) fn header(
        &self,
        project: Option<&ConnectionProfile>,
        status: &str,
        w: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let separator = || {
            div()
                .flex_shrink_0()
                .w(px(1.))
                .h(px(16.))
                .bg(theme.border.opacity(0.5))
        };
        let connection = match project {
            Some(p) if status == "Connected" => connection_pill(p, cx).into_any_element(),
            _ if self.session.connections.borrow().profiles.is_empty() => {
                Label::new("No connection")
                    .id("no-connection")
                    .text_color(theme.muted_foreground)
                    .text_size(px(12.))
                    .into_any_element()
            }
            _ => div()
                .id("connection-picker-slot")
                .test_support()
                .min_w_0()
                .child(self.connection_picker(project, cx))
                .into_any_element(),
        };
        let search = control("Command palette", "Command palette", cx)
            .h(px(28.))
            .px(px(12.))
            .rounded(px(8.))
            .gap(px(8.))
            .border_1()
            .border_color(theme.border.opacity(0.5))
            .bg(theme.muted.opacity(0.5))
            .text_size(px(12.))
            .text_color(theme.muted_foreground)
            .hover(|s| s.text_color(theme.foreground).bg(theme.accent.opacity(0.5)))
            .child(Icon::new(IconName::Search).size(px(12.)))
            .child("Search...")
            .when(w.viewport_size().width >= px(640.), |s| {
                s.child(
                    div()
                        .id("search-shortcut")
                        .test_support()
                        .flex()
                        .items_center()
                        .h(px(20.))
                        .px(px(6.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(theme.border.opacity(0.6))
                        .bg(theme.muted)
                        .text_size(px(10.))
                        .text_color(theme.muted_foreground.opacity(0.7))
                        .child(if cfg!(target_os = "macos") {
                            "⌘K"
                        } else {
                            "CtrlK"
                        }),
                )
            })
            .on_click(cx.listener(|v, _, w, cx| v.toggle_palette(w, cx)))
            .test_support();
        let theme_button = ghost(
            "Toggle theme",
            "Toggle theme",
            if theme.is_dark() {
                IconName::Sun
            } else {
                IconName::Moon
            },
            32.,
            16.,
            cx,
        )
        .on_click(Self::shell_action(cx, Self::toggle_theme))
        .test_support();
        let updates = control("check-updates", "Updates", cx)
            .h(px(32.))
            .px(px(10.))
            .gap(px(8.))
            .flex_shrink_0()
            .rounded(px(6.))
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .hover(|s| s.bg(theme.accent).text_color(theme.accent_foreground))
            .child(Icon::new(IconName::Download).size(px(16.)))
            .child("Updates")
            .on_click(cx.listener(|v, _, _, _| v.check_for_updates()));
        div()
            .id("top-bar")
            .test_support()
            .h(px(44.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .px(px(16.))
            .bg(theme.group_box.opacity(0.8))
            .border_b_1()
            .border_color(theme.border.opacity(0.5))
            .child(
                div()
                    .flex()
                    .items_center()
                    .min_w_0()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_shrink_0()
                            .gap(px(8.))
                            .child(
                                Icon::new(IconName::Database)
                                    .size(px(16.))
                                    .text_color(theme.primary),
                            )
                            .child(
                                Label::new("RSQL")
                                    .id("header-brand")
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD),
                            ),
                    )
                    .child(separator())
                    .child(connection),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .gap(px(8.))
                    .child(search)
                    .child(separator())
                    .child(updates)
                    .child(theme_button),
            )
    }
}

fn connection_pill(p: &ConnectionProfile, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .id("connection-status")
        .test_support()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(2.))
        .rounded_full()
        .bg(theme.accent)
        .text_color(theme.muted_foreground)
        .text_size(px(12.))
        .line_height(px(16.))
        .whitespace_nowrap()
        .child(
            div()
                .flex_shrink_0()
                .size(px(8.))
                .rounded_full()
                .bg(theme.success)
                .shadow(vec![
                    BoxShadow::new(px(0.), px(0.), oklch(0.65, 0.18, 150.)).blur_radius(px(6.)),
                ]),
        )
        .child(
            Label::new(p.display_name().to_owned())
                .id("project-name")
                .min_w_0()
                .truncate()
                .line_height(px(16.))
                .text_color(theme.muted_foreground),
        )
        .child(
            div()
                .text_color(theme.muted_foreground.opacity(0.5))
                .child("•"),
        )
        .child(
            Label::new(format!("{}:{}", p.host, p.port))
                .id("project-address")
                .min_w_0()
                .truncate()
                .line_height(px(16.))
                .text_color(theme.muted_foreground),
        )
}
