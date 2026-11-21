use super::*;
use gpui_kit::{
    assets::IconName,
    base::{Button as Press, DialogClose},
    component::{Icon, Sizable, spinner::Spinner, theme::oklch},
};
use rsql_desktop::metadata::ObjectKind;

pub(in crate::workspace::properties) fn icon(name: IconName, size: f32, color: Hsla) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}

pub(super) fn close(cx: &App) -> DialogClose {
    let idle = cx.theme().muted_foreground.opacity(0.6);
    let hover = cx.theme().muted.opacity(0.5);
    let foreground = cx.theme().foreground;
    DialogClose::new()
        .absolute()
        .top(px(12.))
        .right(px(12.))
        .trigger(move |button| {
            button
                .flex()
                .size(px(28.))
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .text_color(idle)
                .hover(move |s| s.bg(hover).text_color(foreground))
                .child(Icon::new(IconName::X).size(px(16.)))
        })
}

pub(super) fn pill(button: Press, label: SharedString, selected: bool, cx: &App) -> Press {
    let fg = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let background = cx.theme().background;
    button
        .accessibility_label(label)
        .role(Role::Tab)
        .aria_selected(selected)
        .selected(selected)
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .whitespace_nowrap()
        .font_weight(FontWeight::MEDIUM)
        .text_color(muted)
        .styles(|style| style.disabled(|s| s.opacity(0.5)))
        .hover(move |s| s.text_color(fg))
        .styles(|style| {
            style.selected(|s| {
                s.bg(background).text_color(fg).shadow(vec![BoxShadow {
                    color: black().opacity(0.1),
                    offset: point(px(0.), px(1.)),
                    blur_radius: px(2.),
                    spread_radius: px(0.),
                    inset: false,
                }])
            })
        })
}

pub(super) fn pill_rail(cx: &App) -> Div {
    div()
        .flex()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(8.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.2))
        .bg(cx.theme().background.opacity(0.3))
}

impl Panel {
    pub(super) fn header(&self, p: &Properties, connection: String, cx: &App) -> impl IntoElement {
        let kind = p.object.kind;
        let (label, glyph, accent) = match kind {
            ObjectKind::View => ("View", IconName::Eye, oklch(0.623, 0.214, 259.815)),
            ObjectKind::MaterializedView => (
                "Materialized View",
                IconName::Layers,
                oklch(0.627, 0.265, 303.9),
            ),
            kind if kind.is_table() => (kind.label(), IconName::Table, cx.theme().primary),
            _ if matches!(p.sections.get(&Section::Overview), Some(LoadState::Ready(Content::Overview(rows))) if rows.iter().any(|(key, value)| key == "return type" && value.preview.as_ref() == "trigger")) => {
                (
                    "Trigger Function",
                    IconName::Zap,
                    oklch(0.705, 0.213, 47.604),
                )
            }
            _ => (kind.label(), IconName::FileCode, oklch(0.769, 0.188, 70.08)),
        };
        let loading = p
            .sections
            .values()
            .any(|section| matches!(section, LoadState::Loading { .. }));
        div()
            .id("properties-header")
            .test_support()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .px(px(20.))
            .pt(px(20.))
            .pb(px(12.))
            .bg(linear_gradient(
                180.,
                linear_color_stop(accent.opacity(0.2), 0.),
                linear_color_stop(accent.opacity(0.05), 1.),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .pr(px(24.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(32.))
                            .flex_shrink_0()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(cx.theme().border.opacity(0.3))
                            .bg(cx.theme().background.opacity(0.6))
                            .child(icon(glyph, 16., accent)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(self.title(p, cx))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .px(px(6.))
                                            .py(px(2.))
                                            .rounded(px(4.))
                                            .bg(cx.theme().background.opacity(0.4))
                                            .text_size(px(10.))
                                            .line_height(px(15.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(cx.theme().muted_foreground)
                                            .child(label.to_uppercase()),
                                    )
                                    .child(self.schema_copy(p, cx))
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_color(cx.theme().muted_foreground.opacity(0.3))
                                            .child("|"),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(11.))
                                            .line_height(px(16.5))
                                            .text_color(cx.theme().muted_foreground.opacity(0.6))
                                            .child(connection),
                                    )
                                    .when(loading, |d| {
                                        d.child(
                                            Spinner::new()
                                                .with_size(px(12.))
                                                .color(cx.theme().muted_foreground),
                                        )
                                    }),
                            ),
                    ),
            )
            .child(div().mt(px(12.)).child(self.tabs(p, cx)))
            .when(!p.busy, |d| d.child(close(cx)))
    }

    fn schema_copy(&self, p: &Properties, cx: &App) -> Press {
        let schema = p.object.schema.clone();
        let captured = schema.clone();
        let foreground = cx.theme().foreground;
        self.press("properties-copy-schema", move |_, _, cx| {
            catalog::copy_name(captured.clone(), cx);
        })
        .accessibility_label(format!("Copy schema name: {schema}"))
        .disabled(p.busy)
        .min_w_0()
        .truncate()
        .text_size(px(11.))
        .line_height(px(16.5))
        .text_color(cx.theme().muted_foreground)
        .hover(move |style| style.text_color(foreground).underline())
        .child(schema)
    }

    fn title(&self, p: &Properties, cx: &App) -> Div {
        let name = qualified(&p.object.schema, &p.object.name);
        let tooltip = name.clone();
        let idle = cx.theme().muted_foreground.opacity(0.4);
        let fg = cx.theme().foreground;
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .min_w_0()
            .child(
                div()
                    .id("properties-title")
                    .test_support()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(16.))
                    .line_height(px(24.))
                    .child(Label::new(p.object.name.clone()).line_height(px(24.)))
                    .tooltip(move |w, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(w, cx)
                    }),
            )
            .child(
                self.press("properties-copy-name", move |v, _, cx| {
                    catalog::copy_name(name.clone(), cx);
                    v.property_copied(Copied::Name, cx);
                })
                .accessibility_label("Copy qualified name")
                .disabled(p.busy)
                .flex()
                .items_center()
                .justify_center()
                .size(px(16.))
                .flex_shrink_0()
                .text_color(idle)
                .hover(move |s| s.text_color(fg))
                .child(
                    if p.copied.is_some_and(|(_, copied)| copied == Copied::Name) {
                        icon(IconName::Check, 12., cx.theme().success)
                    } else {
                        Icon::new(IconName::Copy).size(px(12.))
                    },
                ),
            )
    }

    fn tabs(&self, p: &Properties, cx: &App) -> impl IntoElement {
        div()
            .id("properties-tabs")
            .test_support()
            .overflow_x_scroll()
            .child(
                pill_rail(cx).children(
                    [
                        (Tab::Overview, IconName::Database),
                        (Tab::Structure, IconName::Pencil),
                        (Tab::Columns, IconName::Columns3),
                        (Tab::Indexes, IconName::Key),
                        (Tab::ForeignKeys, IconName::Link2),
                        (Tab::Ddl, IconName::FileCode),
                        (Tab::Actions, IconName::Zap),
                    ]
                    .into_iter()
                    .filter(|(tab, _)| {
                        p.object.kind.is_table()
                            || matches!(tab, Tab::Overview | Tab::Ddl | Tab::Actions)
                    })
                    .map(|(tab, glyph)| {
                        let label: SharedString = match tab {
                            Tab::Columns => match p.sections.get(&Section::Columns) {
                                Some(LoadState::Ready(Content::Columns(rows))) => {
                                    format!("Columns ({})", rows.len()).into()
                                }
                                _ => tab.label().into(),
                            },
                            Tab::Indexes => match p.sections.get(&Section::Indexes) {
                                Some(LoadState::Ready(Content::Indexes(rows))) => {
                                    format!("Indexes ({})", rows.len()).into()
                                }
                                _ => tab.label().into(),
                            },
                            _ => tab.label().into(),
                        };
                        let button = self.press(tab.label(), move |v, w, cx| {
                            v.change_property_tab(tab, w, cx)
                        });
                        pill(button, label.clone(), tab == p.tab, cx)
                            .disabled(p.busy)
                            .flex_shrink_0()
                            .gap(px(6.))
                            .px(px(12.))
                            .py(px(6.))
                            .text_size(px(11.))
                            .line_height(px(16.5))
                            .child(Icon::new(glyph).size(px(12.)))
                            .child(label)
                    }),
                ),
            )
    }
}
