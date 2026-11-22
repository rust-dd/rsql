use super::{header::icon, *};
use gpui_kit::{assets::IconName, base::Button as Press, component::theme::oklch};
use properties::{IndexRow, PropertyText, Related};

pub(super) fn table(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(12.))
        .overflow_hidden()
        .border_1()
        .border_color(cx.theme().border.opacity(0.3))
        .bg(cx.theme().muted.opacity(0.1))
        .text_size(px(11.))
        .line_height(px(16.5))
}

fn grid() -> Div {
    div().flex().items_start()
}

pub(super) fn cell(width: f32, child: impl IntoElement) -> Div {
    let element = div().min_w_0().px(px(12.)).child(child);
    if width == 0. {
        element.flex_1()
    } else {
        element.w(px(width)).flex_shrink_0()
    }
}

pub(super) fn headers<const N: usize>(columns: [(&'static str, f32); N], cx: &App) -> Div {
    grid()
        .py(px(8.))
        .bg(cx.theme().muted.opacity(0.3))
        .border_b_1()
        .border_color(cx.theme().border.opacity(0.3))
        .text_size(px(9.))
        .line_height(px(13.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground.opacity(0.6))
        .children(columns.into_iter().enumerate().map(|(i, (label, width))| {
            cell(
                width,
                Label::new(label.to_uppercase())
                    .id(("column-heading", i))
                    .line_height(px(13.5))
                    .text_color(cx.theme().muted_foreground.opacity(0.6)),
            )
            .id(("metadata-heading-track", i))
            .test_support()
        }))
}

pub(super) fn row(
    id: impl Into<ElementId>,
    cx: &App,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    let primary = cx.theme().primary;
    grid()
        .id(id)
        .test_support()
        .py(px(6.))
        .border_t_1()
        .border_color(cx.theme().border.opacity(0.15))
        .hover(move |s| s.bg(primary.opacity(0.04)))
}

pub(super) fn empty(text: &'static str, glyph: IconName, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.))
        .py(px(48.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(cx.theme().muted_foreground)
        .child(icon(glyph, 24., cx.theme().muted_foreground.opacity(0.3)))
        .child(text)
}

pub(super) fn toolbar_button(
    button: Press,
    glyph: IconName,
    label: &'static str,
    cx: &App,
) -> Press {
    let hover = cx.theme().muted.opacity(0.5);
    let foreground = cx.theme().foreground;
    button
        .accessibility_label(label)
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.))
        .h(px(24.))
        .px(px(8.))
        .rounded(px(8.))
        .text_size(px(10.))
        .line_height(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().muted_foreground)
        .hover(move |s| s.bg(hover).text_color(foreground))
        .child(gpui_kit::component::Icon::new(glyph).size(px(10.)))
        .child(label)
}

pub(in crate::workspace::properties) fn code_frame(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .min_h_0()
        .rounded(px(12.))
        .overflow_hidden()
        .border_1()
        .border_color(cx.theme().border.opacity(0.4))
        .bg(cx.theme().background)
}

pub(super) fn code_bar(
    label: &'static str,
    glyph: Option<IconName>,
    actions: Div,
    cx: &App,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .flex_shrink_0()
        .px(px(12.))
        .py(px(6.))
        .bg(cx.theme().muted.opacity(0.3))
        .border_b_1()
        .border_color(cx.theme().border.opacity(0.3))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(10.))
                .line_height(px(15.))
                .text_color(cx.theme().muted_foreground.opacity(0.6))
                .children(
                    glyph.map(|name| icon(name, 12., cx.theme().muted_foreground.opacity(0.5))),
                )
                .child(label),
        )
        .child(actions)
}

pub(in crate::workspace::properties) fn code_text(text: &PropertyText, cx: &App) -> Div {
    div()
        .text_size(px(11.))
        .line_height(px(18.))
        .text_color(cx.theme().foreground.opacity(0.9))
        .child(
            Label::new(SharedString::from(&text.preview))
                .line_height(px(18.))
                .text_color(cx.theme().foreground.opacity(0.9)),
        )
}

impl Panel {
    pub(super) fn copy_value(&self, label: &'static str, value: &PropertyText) -> Press {
        let full = value.full.clone();
        self.press(label, move |v, w, cx| {
            v.transfer_property_text(full.clone(), true, w, cx)
        })
    }

    pub(super) fn ddl(&self, p: &Properties, text: &PropertyText, cx: &App) -> Div {
        if text.full.is_empty() {
            return empty("No DDL available", IconName::FileCode, cx);
        }
        let copied = p.copied.is_some_and(|(_, copied)| copied == Copied::Ddl);
        let actions = div()
            .flex()
            .gap(px(4.))
            .child(toolbar_button(
                self.press("Open in editor", |v, w, cx| v.transfer_ddl(false, w, cx)),
                IconName::Play,
                "Open in Tab",
                cx,
            ))
            .child(
                toolbar_button(
                    self.press("Copy DDL", |v, w, cx| v.transfer_ddl(true, w, cx)),
                    if copied {
                        IconName::Check
                    } else {
                        IconName::Copy
                    },
                    if copied { "Copied" } else { "Copy" },
                    cx,
                )
                .when(copied, |b| b.text_color(cx.theme().success)),
            );
        code_frame(cx)
            .flex_1()
            .child(code_bar("DDL", Some(IconName::FileCode), actions, cx))
            .child(
                div()
                    .id("properties-ddl-scroll")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(16.))
                    .child(code_text(text, cx)),
            )
    }

    pub(super) fn indexes(&self, indexes: &[IndexRow], cx: &App) -> impl IntoElement {
        let mut body = div();
        if indexes.is_empty() {
            body = body.child(empty("No indexes found", IconName::Key, cx));
        } else {
            body = table(cx).min_w(px(560.)).child(headers(
                [
                    ("", 36.),
                    ("Index Name", 0.),
                    ("Columns", 0.),
                    ("Type", 120.),
                ],
                cx,
            ));
        }
        for index in indexes {
            let theme = cx.theme();
            let blue = oklch(0.623, 0.214, 259.815);
            let (glyph, glyph_color, label, fill, border, text) = if index.primary {
                (
                    IconName::Key,
                    theme.warning,
                    "PRIMARY KEY",
                    theme.warning.opacity(0.15),
                    theme.warning.opacity(0.2),
                    theme.warning,
                )
            } else if index.unique {
                (
                    IconName::Shield,
                    blue,
                    "UNIQUE",
                    blue.opacity(0.15),
                    blue.opacity(0.2),
                    oklch(0.707, 0.165, 254.624),
                )
            } else {
                (
                    IconName::Key,
                    theme.muted_foreground.opacity(0.25),
                    "INDEX",
                    theme.muted.opacity(0.5),
                    theme.border.opacity(0.3),
                    theme.muted_foreground,
                )
            };
            body = body.child(
                row(("index", index.oid as usize), cx).py(px(8.)).child(
                    grid()
                        .w_full()
                        .child(cell(36., icon(glyph, 12., glyph_color)))
                        .child(
                            cell(
                                0.,
                                Label::new(index.name.clone())
                                    .font_weight(FontWeight::MEDIUM)
                                    .line_height(px(16.5)),
                            )
                            .id(("index-track", 1usize))
                            .test_support(),
                        )
                        .child(
                            cell(
                                0.,
                                div()
                                    .child(
                                        Label::new(SharedString::from(&index.columns.preview))
                                            .line_height(px(16.5))
                                            .text_color(theme.muted_foreground.opacity(0.7)),
                                    )
                                    .when(index.columns.abbreviated(), |d| {
                                        d.child(toolbar_button(
                                            self.copy_value("Copy index columns", &index.columns),
                                            IconName::Copy,
                                            "Copy index columns",
                                            cx,
                                        ))
                                    }),
                            )
                            .id(("index-track", 2usize))
                            .test_support(),
                        )
                        .child(
                            cell(
                                120.,
                                div().flex().items_start().child(
                                    div()
                                        .id("index-role-pill")
                                        .test_support()
                                        .px(px(8.))
                                        .py(px(2.))
                                        .whitespace_nowrap()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(border)
                                        .bg(fill)
                                        .child(
                                            Label::new(label)
                                                .text_size(px(9.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .line_height(px(13.5))
                                                .text_color(text),
                                        ),
                                ),
                            )
                            .id(("index-track", 3usize))
                            .test_support(),
                        ),
                ),
            );
        }
        div()
            .id("property-indexes")
            .test_support()
            .overflow_x_scroll()
            .child(body)
    }

    pub(super) fn related_table(
        &self,
        p: &Properties,
        keys: &[Related],
        incoming: bool,
        cx: &App,
    ) -> impl IntoElement {
        // The source lists ON UPDATE only for outgoing keys.
        let titles = if incoming {
            headers(
                [
                    ("Constraint", 0.),
                    ("From Table", 0.),
                    ("Column", 0.),
                    ("On Delete", 90.),
                ],
                cx,
            )
        } else {
            headers(
                [
                    ("Constraint", 0.),
                    ("Column", 0.),
                    ("References", 0.),
                    ("On Delete", 90.),
                    ("On Update", 90.),
                ],
                cx,
            )
        };
        let fg = cx.theme().muted_foreground;
        let blue = oklch(0.623, 0.214, 259.815);
        div()
            .id(if incoming {
                "incoming-table"
            } else {
                "outgoing-table"
            })
            .overflow_x_scroll()
            .child(
                table(cx)
                    .min_w(px(620.))
                    .child(titles)
                    .children(keys.iter().enumerate().map(|(i, key)| {
                        let capture = p.capture.clone();
                        let related = key.related.clone();
                        let id = format!("Open {}", qualified(&related.schema, &related.name));
                        let table_name = if incoming { &key.source } else { &key.target };
                        // Source data cells use px-2 under px-3 headings.
                        let cell = |width, child: AnyElement| cell(width, child).px(px(8.));
                        let link = cell(
                            0.,
                            self.press(id, move |v, w, cx| {
                                v.dismiss_properties(w, cx);
                                v.object_action(&capture, &related, Action::Select, w, cx);
                            })
                            .accessibility_label(format!("Open {table_name} in Query"))
                            .text_color(blue)
                            .hover(|s| s.underline())
                            .child(table_name.clone())
                            .into_any_element(),
                        );
                        let pairs = cell(
                            0.,
                            div()
                                .text_color(cx.theme().primary.opacity(0.8))
                                .children(key.pairs.iter().enumerate().map(|(i, pair)| {
                                    Label::new(pair.clone())
                                        .id(("pair", i))
                                        .line_height(px(16.5))
                                        .text_color(cx.theme().primary.opacity(0.8))
                                }))
                                .into_any_element(),
                        );
                        let (update, delete) =
                            key.actions.split_once(" · ").unwrap_or((&key.actions, ""));
                        let update = update.strip_prefix("ON UPDATE ").unwrap_or(update);
                        let delete = delete.strip_prefix("ON DELETE ").unwrap_or(delete);
                        let rule = |text: &str| {
                            cell(
                                90.,
                                div()
                                    .text_color(fg)
                                    .child(
                                        Label::new(text.to_owned())
                                            .line_height(px(16.5))
                                            .text_color(fg),
                                    )
                                    .into_any_element(),
                            )
                        };
                        let branch = row(("foreign-key", i), cx)
                            .child(cell(0., Label::new(key.name.clone()).into_any_element()));
                        if incoming {
                            branch.child(link).child(pairs).child(rule(delete))
                        } else {
                            branch
                                .child(pairs)
                                .child(link)
                                .child(rule(delete))
                                .child(rule(update))
                        }
                    })),
            )
    }
}
