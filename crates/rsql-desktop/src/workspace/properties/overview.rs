mod format;
mod routine;
use super::view::{Panel, header::icon};
use super::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme, Sizable, label::Label, spinner::Spinner},
};
use properties::{DetailKind, DetailRow, PropertyText};

type Fields = [(String, PropertyText)];
const UNAVAILABLE: &str = "Unavailable";

fn field<'a>(fields: &'a Fields, key: &str) -> Option<&'a PropertyText> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn value(fields: &Fields, key: &str) -> SharedString {
    field(fields, key)
        .map(|v| SharedString::from(&v.preview))
        .unwrap_or_else(|| UNAVAILABLE.into())
}

fn grouped(fields: &Fields, key: &str) -> SharedString {
    field(fields, key)
        .and_then(|v| format::count(&v.preview))
        .map_or_else(|| value(fields, key), SharedString::from)
}

fn maintained(fields: &Fields, key: &str, now: chrono::DateTime<chrono::Utc>) -> SharedString {
    let shown = value(fields, key);
    // A present statistics row with a NULL timestamp means the table was never processed.
    if shown == UNAVAILABLE && value(fields, "live tuples") != UNAVAILABLE {
        return "never".into();
    }
    format::relative(&shown, now).map_or(shown, SharedString::from)
}

pub(super) fn loading(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(12.))
        .py(px(64.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(cx.theme().muted_foreground)
        .child(
            Spinner::new()
                .with_size(px(20.))
                .color(cx.theme().muted_foreground),
        )
        .child("Loading...")
}

pub(super) fn section(
    title: &'static str,
    glyph: IconName,
    body: impl IntoElement,
    cx: &App,
) -> Div {
    let muted = cx.theme().muted_foreground;
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(20.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .bg(cx.theme().muted.opacity(0.4))
                        .child(icon(glyph, 14., muted.opacity(0.6))),
                )
                .child(
                    Label::new(title.to_uppercase())
                        .id(title)
                        .text_size(px(10.))
                        .line_height(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(muted.opacity(0.8)),
                )
                .child(div().flex_1().h(px(1.)).bg(cx.theme().border.opacity(0.3))),
        )
        .child(body)
}

fn card(
    key: &str,
    label: &str,
    value: SharedString,
    glyph: IconName,
    accent: Hsla,
    cx: &App,
) -> impl IntoElement {
    let border = cx.theme().border;
    let muted = cx.theme().muted;
    div()
        .id(SharedString::from(key))
        .test_support()
        .flex()
        .items_center()
        .gap(px(8.))
        .flex_1()
        .min_w_0()
        .px(px(12.))
        .py(px(10.))
        .rounded(px(12.))
        .border_1()
        .border_color(border.opacity(0.3))
        .bg(linear_gradient(
            135.,
            linear_color_stop(muted.opacity(0.4), 0.),
            linear_color_stop(muted.opacity(0.1), 1.),
        ))
        .hover(move |d| d.border_color(border.opacity(0.5)))
        .child(
            div()
                .size(px(24.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(cx.theme().background.opacity(0.5))
                .rounded(px(6.))
                .child(icon(glyph, 14., accent)),
        )
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    Label::new(label.to_uppercase())
                        .text_size(px(9.))
                        .line_height(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().muted_foreground.opacity(0.6)),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(14.))
                        .line_height(px(17.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(Label::new(value).line_height(px(17.5))),
                ),
        )
}

fn cards(keys: &[(&str, &str, IconName)], fields: &Fields, cx: &App) -> Div {
    div()
        .flex()
        .gap(px(8.))
        .children(keys.iter().map(|&(key, label, glyph)| {
            let color = match key {
                "live tuples" => cx.theme().success,
                "dead tuples" => cx.theme().warning,
                _ => cx.theme().muted_foreground,
            };
            let shown = match key {
                "row estimate" | "live tuples" | "dead tuples" => grouped(fields, key),
                _ => value(fields, key),
            };
            card(key, label, shown, glyph, color, cx)
        }))
}

fn info_row(label: &'static str, displayed: SharedString, border: bool, cx: &App) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .py(px(6.))
        .text_size(px(11.))
        .line_height(px(16.5))
        .when(border, |d| {
            d.border_b_1().border_color(cx.theme().border.opacity(0.2))
        })
        .child(
            div()
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground.opacity(0.7))
                .child(label.to_owned()),
        )
        .child(
            div()
                .min_w_0()
                .text_right()
                .font_weight(FontWeight::MEDIUM)
                .child(Label::new(displayed).id(label).line_height(px(16.5))),
        )
}

/// Row-major like the source two-column grid, where only the final cell drops its rule.
fn info_grid(entries: [(&'static str, SharedString); 4], cx: &App) -> Div {
    let [a, b, c, d] = entries;
    div().flex().flex_col().gap(px(4.)).px(px(4.)).children(
        [[(a, true), (b, true)], [(c, true), (d, false)]].map(|cells| {
            div()
                .flex()
                .gap(px(16.))
                .children(cells.map(|((label, shown), rule)| info_row(label, shown, rule, cx)))
        }),
    )
}

pub(super) fn statistics(fields: &Fields, cx: &App) -> Div {
    let scan = |key: &'static str, label: &str| {
        div()
            .id(key)
            .flex_1()
            .min_w_0()
            .px(px(12.))
            .py(px(8.))
            .rounded(px(6.))
            .bg(cx.theme().muted.opacity(0.3))
            .text_size(px(12.))
            .line_height(px(16.))
            .child(
                div()
                    .text_size(px(10.))
                    .line_height(px(15.))
                    .mb(px(2.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        Label::new(label.to_uppercase())
                            .line_height(px(15.))
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .child(Label::new(grouped(fields, key)).line_height(px(16.)))
    };
    let now = chrono::Utc::now();
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(cards(
                    &[
                        ("row estimate", "Rows (est.)", IconName::Database),
                        ("table size", "Table Size", IconName::HardDrive),
                        ("total size", "Total Size", IconName::HardDrive),
                    ],
                    fields,
                    cx,
                ))
                .child(cards(
                    &[
                        ("index size", "Index Size", IconName::Key),
                        ("live tuples", "Live Tuples", IconName::Check),
                        ("dead tuples", "Dead Tuples", IconName::TriangleAlert),
                    ],
                    fields,
                    cx,
                )),
        )
        .child(section(
            "Scan Statistics",
            IconName::RefreshCw,
            div()
                .flex()
                .gap(px(8.))
                .child(scan("seq scan", "Sequential Scans"))
                .child(scan("idx scan", "Index Scans")),
            cx,
        ))
        .child(section(
            "Maintenance",
            IconName::RefreshCw,
            info_grid(
                [
                    ("Last Vacuum", maintained(fields, "last vacuum", now)),
                    (
                        "Last Auto Vacuum",
                        maintained(fields, "last autovacuum", now),
                    ),
                    ("Last Analyze", maintained(fields, "last analyze", now)),
                    (
                        "Last Auto Analyze",
                        maintained(fields, "last autoanalyze", now),
                    ),
                ],
                cx,
            ),
            cx,
        ))
}

impl Panel {
    pub(super) fn overview_details(
        &self,
        section_kind: Section,
        rows: &[DetailRow],
        cx: &App,
    ) -> Div {
        let (label, glyph) = match section_kind {
            Section::Constraints => ("Constraints", IconName::Link2),
            Section::Triggers => ("Triggers", IconName::Zap),
            Section::Rules => ("Rules", IconName::ScrollText),
            Section::Policies => ("RLS Policies", IconName::Lock),
            _ => unreachable!(),
        };
        section(
            label,
            glyph,
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(rows.iter().enumerate().map(|(i, row)| {
                    let muted = cx.theme().muted_foreground;
                    let blue = gpui_kit::component::theme::oklch(0.623, 0.214, 259.815);
                    let (row_icon, color) = match row.kind {
                        DetailKind::Primary => (IconName::Key, cx.theme().warning),
                        DetailKind::Foreign => (IconName::Link2, blue),
                        DetailKind::Unique => (IconName::Shield, blue),
                        DetailKind::Check => (IconName::Check, muted),
                        _ => (glyph, muted.opacity(0.5)),
                    };
                    let exact = row.exact.full.clone();
                    div()
                        .id((label, i))
                        .test_support()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(10.))
                        .py(px(6.))
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(cx.theme().border.opacity(0.2))
                        .bg(cx.theme().muted.opacity(0.2))
                        .child(
                            div()
                                .id("metadata-glyph")
                                .test_support()
                                .flex_shrink_0()
                                .child(icon(row_icon, 12., color)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .child(Label::new(row.name.clone()).id("metadata-name")),
                        )
                        .when_some(row.badge.as_ref(), |d, badge| {
                            d.child(
                                div()
                                    .id("metadata-badge")
                                    .test_support()
                                    .flex_shrink_0()
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(6.))
                                    .bg(cx.theme().muted.opacity(0.5))
                                    .child(
                                        Label::new(badge.clone())
                                            .text_size(px(9.))
                                            .line_height(px(13.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(muted.opacity(0.7)),
                                    ),
                            )
                        })
                        .children(row.value.as_ref().map(|value| {
                            div().min_w_0().child(
                                Label::new(value.clone())
                                    .id("metadata-value")
                                    .text_color(muted),
                            )
                        }))
                        .child(div().flex_1())
                        .child(
                            self.press("metadata-copy", move |v, w, cx| {
                                v.transfer_property_text(exact.clone(), true, w, cx);
                            })
                            .accessibility_label("Copy metadata details")
                            .size(px(20.))
                            .rounded(px(4.))
                            .flex_shrink_0()
                            .hover(move |s| s.bg(muted.opacity(0.12)))
                            .child(icon(IconName::Copy, 12., muted)),
                        )
                })),
            cx,
        )
    }
}
