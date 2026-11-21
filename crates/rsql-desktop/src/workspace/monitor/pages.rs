use super::super::numbers::fixed;
use super::*;
use chrono::{DateTime, Local};
use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme, Icon, label::Label},
    prelude::FluentBuilder,
};
use view::color;

pub(super) const CARDS: [(&str, IconName); 4] = [
    ("Active Connections", IconName::Users),
    ("Database Size", IconName::Database),
    ("Cache Hit Ratio", IconName::Zap),
    ("Deadlocks", IconName::HardDrive),
];

fn box_panel(cx: &App) -> Div {
    div()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
}

fn title(text: &'static str, cx: &App) -> Div {
    div()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(Label::new(text).font_weight(FontWeight::SEMIBOLD))
}

pub(super) fn preview(sql: &str) -> String {
    let mut chars = sql.chars();
    let mut text = chars.by_ref().take(200).collect::<String>();
    if chars.next().is_some() {
        text.push_str("...");
    }
    text
}

impl Panel {
    pub(super) fn overview(&self, summary: &Summary, cx: &App) -> Div {
        let stats = match &self.snapshot.sections[Lane::Stats as usize].rows {
            Some(Rows::Stats(pairs)) => Some(pairs.as_slice()),
            _ => None,
        };
        let cards = div()
            .flex()
            .flex_wrap()
            .gap_3()
            .children(CARDS.into_iter().enumerate().map(|(index, (name, icon))| {
                let value = stats
                    .and_then(|r| r.iter().find(|(key, _)| key == name))
                    .map_or("N/A", |(_, v)| v);
                box_panel(cx)
                    .id(("monitor-card", index))
                    .flex_1()
                    .min_w(px(120.))
                    .p_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_color(cx.theme().muted_foreground)
                            .child(Icon::new(icon).size(px(16.)))
                            .child(
                                Label::new(name)
                                    .text_size(px(10.))
                                    .text_color(cx.theme().muted_foreground),
                            ),
                    )
                    .child(
                        Label::new(value.to_owned())
                            .text_size(px(18.))
                            .font_weight(FontWeight::BOLD),
                    )
            }));
        let mut pairs = box_panel(cx)
            .id("monitor-stats")
            .child(title("Database Statistics", cx));
        if let Some(stats) = stats {
            for (index, (key, value)) in stats.iter().enumerate() {
                pairs = pairs.child(
                    div()
                        .id(("stat-pair", index))
                        .flex()
                        .justify_between()
                        .px_3()
                        .py_1()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(Label::new(key.clone()).text_color(cx.theme().muted_foreground))
                        .child(Label::new(value.clone())),
                );
            }
            if stats.is_empty() {
                pairs = pairs.child(Label::new("No stats available"));
            }
        }
        let summary = box_panel(cx)
            .id("monitor-summary")
            .child(title("Session Query Summary", cx))
            .child(
                div().flex().children(
                    [
                        ("Total Queries", summary.total.to_string()),
                        (
                            "Avg Execution Time",
                            format!("{}ms", fixed(summary.average(), 1)),
                        ),
                        ("Failed Queries", summary.failed.to_string()),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, value))| {
                        div()
                            .id(("summary-metric", index))
                            .p_3()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                Label::new(value)
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::BOLD)
                                    .when(label == "Failed Queries" && summary.failed > 0, |l| {
                                        l.text_color(cx.theme().danger)
                                    }),
                            )
                            .child(
                                Label::new(label)
                                    .text_size(px(10.))
                                    .text_color(cx.theme().muted_foreground),
                            )
                    }),
                ),
            );
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(cards)
            .child(pairs)
            .child(summary)
    }

    pub(super) fn history_page(&self, summary: &Summary, cx: &App) -> Div {
        box_panel(cx)
            .child(title("Slowest Queries (Session)", cx))
            .when(summary.slowest.is_empty(), |d| {
                d.child(
                    div()
                        .p_4()
                        .child(Label::new("No queries executed yet in this session")),
                )
            })
            .children(summary.slowest.iter().map(|(sequence, e)| {
                let when = DateTime::from_timestamp_millis(e.executed_at).map_or_else(
                    || e.executed_at.to_string(),
                    |t| t.with_timezone(&Local).format("%H:%M:%S").to_string(),
                );
                let status = if e.success {
                    format!("{} rows", e.row_count)
                } else {
                    "FAILED".into()
                };
                div()
                    .id(ElementId::NamedInteger(
                        "monitor-history-entry".into(),
                        *sequence,
                    ))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(
                                Label::new(format!("{when} - {status}"))
                                    .text_size(px(10.))
                                    .text_color(color(
                                        if e.success {
                                            Tone::Neutral
                                        } else {
                                            Tone::Danger
                                        },
                                        cx,
                                    )),
                            )
                            .child(
                                Label::new(format!("{}ms", fixed(e.duration_ms, 1)))
                                    .text_size(px(11.))
                                    .when(e.duration_ms > 1000., |l| {
                                        l.text_color(cx.theme().danger)
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .mt_1()
                            .bg(cx.theme().muted.opacity(0.5))
                            .rounded_md()
                            .p_1p5()
                            .child(Label::new(preview(&e.sql)).text_size(px(10.))),
                    )
                    .children(e.error.as_ref().map(|s| {
                        Label::new(s.clone())
                            .text_size(px(10.))
                            .text_color(cx.theme().danger)
                    }))
            }))
    }
}
