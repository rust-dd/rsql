use super::*;
use gpui_kit::prelude::FluentBuilder;

pub(in crate::workspace) struct TreeRow {
    pub depth: usize,
    pub label: SharedString,
    pub tooltip: Option<SharedString>,
    pub icon: Icon,
    pub detail: Option<String>,
    pub detail_badge: Option<(&'static str, Hsla)>,
    pub expanded: Option<bool>,
    pub loading: bool,
    pub selected: bool,
    pub bold: bool,
    pub muted: bool,
}

impl TreeRow {
    pub(in crate::workspace) fn new(
        depth: usize,
        label: impl Into<SharedString>,
        icon: Icon,
    ) -> Self {
        Self {
            depth,
            label: label.into(),
            tooltip: None,
            icon,
            detail: None,
            detail_badge: None,
            expanded: None,
            loading: false,
            selected: false,
            bold: false,
            muted: false,
        }
    }

    pub(in crate::workspace) fn paint(
        self,
        id: impl Into<ElementId>,
        cx: &App,
    ) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
        let theme = cx.theme();
        let tooltip = self.tooltip.unwrap_or_else(|| self.label.clone());
        let description = self.detail.as_ref().map(|detail| {
            self.detail_badge
                .map_or_else(|| detail.clone(), |(badge, _)| format!("{detail} {badge}"))
        });
        div()
            .id(id)
            .test_support()
            .role(Role::Button)
            .aria_label(self.label.clone())
            .when_some(description, |row, detail| row.aria_description(detail))
            .aria_selected(self.selected)
            .tab_index(0)
            .relative()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .w_full()
            .min_w_0()
            .h(px(22.))
            .pl(px(4. + self.depth as f32 * 8.))
            .pr_1p5()
            .rounded(px(4.))
            .cursor_pointer()
            .line_height(px(16.))
            .text_size(px(12.))
            .text_color(if self.muted {
                theme.muted_foreground
            } else {
                theme.sidebar_foreground
            })
            .when(self.selected, |r| r.bg(theme.primary.opacity(0.1)))
            .when(!self.selected, |r| r.hover(|s| s.bg(theme.sidebar_accent)))
            .children((0..self.depth).map(|depth| {
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(10. + 8. * depth as f32))
                    .w(px(1.))
                    .bg(theme.sidebar_border.opacity(0.3))
            }))
            .child(
                div()
                    .w(px(12.))
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .children(self.expanded.map(|open| {
                        glyph(
                            if self.loading {
                                IconName::LoaderCircle
                            } else if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                            12.,
                            theme.muted_foreground,
                        )
                    })),
            )
            .child(
                div()
                    .size(px(14.))
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .child(self.icon),
            )
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .flex_1()
                    .min_w_0()
                    .gap_1p5()
                    .pl_0p5()
                    .child(
                        div()
                            .max_w_full()
                            .flex_shrink_0()
                            .truncate()
                            .when(self.bold, |s| s.font_weight(FontWeight::SEMIBOLD))
                            .child(self.label),
                    )
                    .children(self.detail.map(|detail| {
                        div()
                            .flex()
                            .items_baseline()
                            .gap_1()
                            .min_w_0()
                            .flex_shrink(999.)
                            .overflow_hidden()
                            .text_size(px(10.))
                            .text_color(theme.muted_foreground.opacity(0.8))
                            .child(div().truncate().min_w_0().child(detail))
                            .children(self.detail_badge.map(|(text, color)| {
                                div().flex_shrink_0().text_color(color).child(text)
                            }))
                    })),
            )
            .tooltip(move |w, cx| Tooltip::new(tooltip.clone()).build(w, cx))
    }
}

pub(in crate::workspace) fn trailing_text(text: String, cx: &App) -> Div {
    trailing(Some(text), None, cx)
}

pub(super) fn trailing(text: Option<String>, status: Option<Status>, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .max_w(relative(0.45))
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_1p5()
        .ml_1()
        .children(text.map(|text| {
            div()
                .min_w_0()
                .truncate()
                .text_size(px(10.))
                .text_color(theme.muted_foreground)
                .child(text)
        }))
        .children(status.map(|status| {
            let color = match status {
                Status::Connected => theme.success,
                Status::Connecting => theme.warning,
                Status::Failed => theme.danger,
                Status::Disconnected => theme.muted_foreground.opacity(0.3),
            };
            div()
                .size(px(6.))
                .flex_shrink_0()
                .rounded_full()
                .bg(color)
                .when(status != Status::Disconnected, |dot| {
                    dot.shadow(vec![
                        BoxShadow::new(px(0.), px(0.), color).blur_radius(px(6.)),
                    ])
                })
        }))
}

pub(in crate::workspace) fn glyph(name: IconName, size: f32, color: Hsla) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}
