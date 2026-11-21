use super::{
    header::icon,
    layout::{cell, empty, headers, row, table},
    *,
};
use gpui_kit::{
    assets::IconName,
    component::{theme::oklch, tooltip::Tooltip},
};
use properties::{ColumnRow, NO_DEFAULT};

/// Matches the visible span of the source's 150 px truncated cell with room to spare.
const DEFAULT_PREVIEW_BYTES: usize = 64;

impl Panel {
    pub(super) fn columns(&self, columns: &[ColumnRow], cx: &App) -> impl IntoElement {
        let tracks = [
            ("#", 42.),
            ("", 36.),
            ("Name", 0.),
            ("Type", 0.),
            ("Nullable", 92.),
            ("Default", 150.),
        ];
        let mut body = table(cx).min_w(px(570.)).child(headers(tracks, cx));
        for c in columns {
            let fg = cx.theme().muted_foreground;
            body = body.child(
                row(("column", c.number as usize), cx)
                    .child(cell(
                        42.,
                        div()
                            .text_color(fg.opacity(0.3))
                            .child(c.number.to_string()),
                    ))
                    .child(cell(
                        36.,
                        icon(
                            if c.primary {
                                IconName::Key
                            } else {
                                IconName::Columns3
                            },
                            12.,
                            if c.primary {
                                cx.theme().warning
                            } else {
                                fg.opacity(0.2)
                            },
                        ),
                    ))
                    .child(cell(
                        0.,
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(Label::new(c.name.clone()).line_height(px(16.5))),
                    ))
                    .child(cell(
                        0.,
                        // An inline span in the source: tight to the text, padding outside the line box.
                        div().flex().child(
                            div()
                                .min_w_0()
                                .rounded(px(4.))
                                .px(px(6.))
                                .py(px(2.))
                                .my(px(-2.))
                                .bg(cx.theme().primary.opacity(0.06))
                                .text_color(cx.theme().primary.opacity(0.7))
                                .child(
                                    Label::new(c.ty.clone())
                                        .line_height(px(16.5))
                                        .text_color(cx.theme().primary.opacity(0.7)),
                                ),
                        ),
                    ))
                    .child(cell(
                        92.,
                        if c.nullable {
                            div().text_color(fg.opacity(0.4)).child(
                                Label::new("YES")
                                    .line_height(px(16.5))
                                    .text_color(fg.opacity(0.4)),
                            )
                        } else {
                            div()
                                .text_size(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(oklch(0.75, 0.183, 55.934).opacity(0.8))
                                .child(
                                    Label::new("NOT NULL")
                                        .line_height(px(16.5))
                                        .text_color(oklch(0.75, 0.183, 55.934).opacity(0.8)),
                                )
                        },
                    ))
                    .child(cell(150., self.column_default(c, cx))),
            );
        }
        if columns.is_empty() {
            body = body.child(empty("No columns found", IconName::Columns3, cx));
        }
        div()
            .id("property-columns")
            .test_support()
            .overflow_x_scroll()
            .child(body)
    }

    /// One truncated line like the source; hover shows the bounded preview and an
    /// abbreviated payload keeps its exact off-thread Copy.
    fn column_default(&self, c: &ColumnRow, cx: &App) -> Div {
        let muted = cx.theme().muted_foreground;
        let text = &c.default;
        if text.full.as_ref() == NO_DEFAULT {
            return div().text_color(muted.opacity(0.2)).child("-");
        }
        let line = text.preview.find('\n').unwrap_or(text.preview.len());
        let end = text
            .preview
            .floor_char_boundary(line.min(DEFAULT_PREVIEW_BYTES));
        let shown = if end == text.preview.len() {
            SharedString::from(&text.preview)
        } else {
            format!("{}…", &text.preview[..end]).into()
        };
        let hover = SharedString::from(&text.preview);
        let foreground = cx.theme().foreground;
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .min_w_0()
            .child(
                div()
                    .id(("column-default", c.number as usize))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(muted.opacity(0.6))
                    .child(
                        Label::new(shown)
                            .line_height(px(16.5))
                            .text_color(muted.opacity(0.6)),
                    )
                    .tooltip(move |w, cx| Tooltip::new(hover.clone()).max_w(px(480.)).build(w, cx)),
            )
            .when(text.abbreviated(), |d| {
                d.child(
                    self.copy_value("Copy default", text)
                        .accessibility_label("Copy default")
                        .flex_shrink_0()
                        .text_color(muted)
                        .hover(move |s| s.text_color(foreground))
                        .child(gpui_kit::component::Icon::new(IconName::Copy).size(px(12.))),
                )
            })
    }
}
