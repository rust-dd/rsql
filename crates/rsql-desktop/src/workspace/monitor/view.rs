use super::*;
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Disableable, Icon,
        button::{Button, ButtonVariants},
        label::Label,
    },
    prelude::FluentBuilder,
};
use theme::controls::compact_button;

pub(super) const TABS: [(Page, &str, IconName); 7] = [
    (Page::Overview, "Overview", IconName::ChartColumn),
    (Page::Activity, "Activity", IconName::Activity),
    (Page::Tables, "Table Stats", IconName::Table),
    (Page::History, "Query History", IconName::Clock3),
    (Page::Locks, "Locks", IconName::Lock),
    (Page::Indexes, "Index Advisor", IconName::Search),
    (Page::Bloat, "Bloat", IconName::Gauge),
];

pub(super) fn color(tone: Tone, cx: &App) -> Hsla {
    match tone {
        Tone::Neutral => cx.theme().muted_foreground,
        Tone::Good => cx.theme().success,
        Tone::Warning => cx.theme().warning,
        Tone::Danger => cx.theme().danger,
    }
}

impl Panel {
    fn header(&self, cx: &Context<Self>) -> Div {
        let auto = if self.auto {
            "Pause auto-refresh"
        } else {
            "Start auto-refresh (5s)"
        };
        let latest = self
            .snapshot
            .sections
            .iter()
            .filter_map(|s| s.success)
            .max();
        div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Icon::new(IconName::Activity)
                    .size(px(16.))
                    .text_color(cx.theme().primary),
            )
            .child(
                Label::new("Performance Monitor")
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.)),
            )
            .child(Label::new(self.database.clone()).text_color(cx.theme().muted_foreground))
            .child(div().flex_1())
            .children(latest.map(|t| {
                Label::new(format!("Refreshed at {}", t.format("%H:%M:%S")))
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
            }))
            .child(
                compact_button(Button::new("monitor-auto"))
                    .ghost()
                    .w(px(28.))
                    .icon(if self.auto {
                        IconName::Pause
                    } else {
                        IconName::Play
                    })
                    .tooltip(auto)
                    .accessibility_label(auto)
                    .on_click(cx.listener(|p, _, _, cx| {
                        if p.current() {
                            p.auto = !p.auto;
                            p.arm_timer(cx);
                            cx.notify();
                        }
                    })),
            )
            .child(
                compact_button(Button::new("monitor-refresh"))
                    .ghost()
                    .w(px(28.))
                    .icon(if self.pending.is_some() {
                        IconName::LoaderCircle
                    } else {
                        IconName::RefreshCw
                    })
                    .disabled(self.pending.is_some())
                    .tooltip("Refresh")
                    .accessibility_label("Refresh")
                    .on_click(cx.listener(|p, _, _, cx| p.refresh(cx))),
            )
    }

    fn tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("monitor-tabs")
            .test_support()
            .role(Role::TabList)
            .overflow_x_scroll()
            .flex()
            .flex_shrink_0()
            .px_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .children(TABS.into_iter().map(|(page, label, icon)| {
                let selected = page == self.page;
                div()
                    .flex_shrink_0()
                    .border_b_2()
                    .border_color(if selected {
                        cx.theme().primary
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .child(
                        gpui_kit::base::Button::new(label)
                            .role(Role::Tab)
                            .accessibility_label(label)
                            .aria_selected(selected)
                            .rounded_md()
                            .gap_1p5()
                            .hover(|b| b.bg(cx.theme().muted).text_color(cx.theme().foreground))
                            .focus_visible(|b| b.border_1().border_color(cx.theme().primary))
                            .h(px(32.))
                            .px_3()
                            .child(Icon::new(icon).size(px(14.)))
                            .child(label)
                            .when(selected, |b| b.track_focus(&self.focus))
                            .text_color(if selected {
                                cx.theme().foreground
                            } else {
                                cx.theme().muted_foreground
                            })
                            .on_click(cx.listener(move |p, _, w, cx| {
                                if p.current() {
                                    p.page = page;
                                    p.inspection = None;
                                    p.focus.focus(w, cx);
                                    cx.notify();
                                }
                            })),
                    )
            }))
    }

    fn status(&self, lane: Lane, section: &Section, cx: &Context<Self>) -> Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .flex_shrink_0()
            .gap_2()
            .text_size(px(10.))
            .text_color(cx.theme().muted_foreground)
            .children(
                section.success.map(|t| {
                    Label::new(format!("Last successful refresh {}", t.format("%H:%M:%S")))
                }),
            )
            .when(section.loading, |d| d.child(Label::new("Refreshing…")))
            .when_some(section.error.as_ref(), |d, e| {
                d.child(Label::new(e.clone()).text_color(cx.theme().danger))
                    .child(
                        compact_button(Button::new("monitor-retry"))
                            .label("Retry")
                            .disabled(self.pending.is_some())
                            .on_click(cx.listener(move |p, _, _, cx| {
                                if p.page.lane() == Some(lane) {
                                    p.request(&[lane], cx);
                                }
                            })),
                    )
            })
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self
            .page
            .lane()
            .map(|l| &self.snapshot.sections[l as usize]);
        let mut content = div()
            .id(("monitor-content", self.page as usize))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .p_4()
            .gap_2();
        if let (Some(lane), Some(s)) = (self.page.lane(), section) {
            content = content.child(self.status(lane, s, cx));
        }
        content = match self.page {
            Page::Overview | Page::History => {
                let entries = {
                    let log = self.session.history();
                    log.entries
                        .iter()
                        .map(|e| (e.sequence, e.entry.clone()))
                        .collect::<Vec<_>>()
                };
                let summary = Summary::new(entries.into_iter(), &self.capture.project);
                content
                    .overflow_y_scroll()
                    .child(if self.page == Page::Overview {
                        self.overview(&summary, cx)
                    } else {
                        self.history_page(&summary, cx)
                    })
            }
            _ => {
                if let Some(rows) = section.and_then(|s| s.rows.as_ref()) {
                    content.child(self.table(rows, cx))
                } else {
                    content
                }
            }
        };
        div()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .text_size(px(12.))
            .child(self.header(cx))
            .child(self.tabs(cx))
            .child(content)
    }
}
