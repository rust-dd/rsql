use super::super::editor::{Editor, Field, Page};
use super::{
    header::{pill, pill_rail},
    layout::{code_bar, code_frame, code_text, toolbar_button},
    *,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable, input::Input};
use gpui_kit::component::{Sizable, button::ButtonVariants};
use rsql_desktop::structure::*;

pub(super) fn line() -> Div {
    div().flex().flex_wrap().items_center().gap_1()
}
pub(super) fn stack() -> Div {
    div().flex().flex_col().gap_1()
}

impl Panel {
    pub(super) fn edit_structure(&self, e: &Editor, cx: &App) -> Div {
        let tabs = pill_rail(cx)
            .flex_shrink_0()
            .children(Page::ALL.into_iter().map(|(page, label)| {
                let button = self.press(format!("edit-{label}"), move |v, _, cx| {
                    let p = v.properties.as_mut().unwrap();
                    let e = p.editor.as_mut().unwrap();
                    e.page = page;
                    e.input = None;
                    cx.notify();
                });
                pill(button, label.into(), page == e.page, cx)
                    .px(px(10.))
                    .py(px(4.))
                    .text_size(px(10.))
                    .line_height(px(15.))
                    .child(label)
            }));
        let rows = match e.page {
            Page::Columns => self.structure_columns(e, cx),
            Page::Primary => self.primary_key_rows(e, cx),
            _ => self.structure_objects(e, cx),
        };
        let rows = if e.page == Page::Primary {
            rows
        } else {
            rows.child(self.add_row(e, cx))
        };
        stack()
            .gap(px(8.))
            .flex_1()
            .min_h_0()
            .child(tabs)
            .child(
                div()
                    .id("structure-rows")
                    .test_support()
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .children(self.structure_preview(e, cx))
            .children(self.structure_footer(e, cx))
    }

    fn structure_preview(&self, e: &Editor, cx: &App) -> Option<impl IntoElement> {
        let (_, sql) = e.review.as_ref().filter(|_| e.show_sql)?;
        let mut actions = div().flex().gap(px(4.));
        for (id, label, glyph, copy) in [
            ("Open SQL", "Open in Tab", IconName::Play, false),
            ("Copy SQL", "Copy", IconName::Copy, true),
        ] {
            let text = sql.full.clone();
            actions = actions.child(toolbar_button(
                self.press(id, move |v, w, cx| {
                    v.transfer_property_text(text.clone(), copy, w, cx)
                }),
                glyph,
                label,
                cx,
            ));
        }
        Some(
            div()
                .id("structure-preview")
                .test_support()
                .flex_shrink_0()
                .child(
                    code_frame(cx)
                        .child(code_bar("SQL Preview", None, actions, cx))
                        .child(
                            div()
                                .id("structure-preview-scroll")
                                .max_h(px(150.))
                                .overflow_y_scroll()
                                .p(px(12.))
                                .child(code_text(sql, cx)),
                        ),
                ),
        )
    }

    /// Like the source, the footer only appears once the draft has changes.
    fn structure_footer(&self, e: &Editor, cx: &App) -> Option<impl IntoElement> {
        let count = e.draft.change_count();
        if count == 0 {
            return None;
        }
        let blocked = e.reviewing || e.has_incomplete_fk_pairs();
        let change_label = if count == 1 {
            "1 change".into()
        } else {
            format!("{count} changes")
        };
        Some(
            line()
                .flex_shrink_0()
                .gap(px(8.))
                .pt(px(8.))
                .border_t_1()
                .border_color(cx.theme().border.opacity(0.2))
                .child(
                    div()
                        .text_size(px(10.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().muted_foreground)
                        .child(change_label),
                )
                .child(div().flex_1())
                .child(
                    self.button("Discard", |v, _, cx| {
                        v.edit_draft(cx, |d| d.discard());
                        v.properties
                            .as_mut()
                            .unwrap()
                            .editor
                            .as_mut()
                            .unwrap()
                            .input = None;
                    })
                    .ghost()
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground),
                )
                .child(
                    self.button(
                        if e.show_sql { "Hide SQL" } else { "Review SQL" },
                        |v, w, cx| v.review_structure(w, cx),
                    )
                    .outline()
                    .text_size(px(11.))
                    .icon(IconName::FileCode)
                    .disabled(blocked),
                )
                .child(
                    self.button("Apply", |v, w, cx| v.apply_structure_review(w, cx))
                        .primary()
                        .px(px(12.))
                        .text_size(px(11.))
                        .disabled(blocked),
                ),
        )
    }

    pub(super) fn field(&self, e: &Editor, id: EditId, field: Field, value: &str, cx: &App) -> Div {
        let cell = div().min_w_0().w_full().flex_1();
        if let Some(i) = e.input.as_ref().filter(|i| i.id == id && i.field == field) {
            return cell.child(
                Input::new(&i.input)
                    .id(format!("input-{id:?}-{field:?}"))
                    .with_size(px(28.))
                    .h(px(28.))
                    .rounded(px(6.))
                    .bg(cx.theme().background)
                    .border_color(cx.theme().border.opacity(0.3)),
            );
        }

        let placeholder = value.is_empty();
        let label = if placeholder {
            match field {
                Field::KeyName => "Constraint name",
                Field::Type => "Type",
                Field::Name => "Name",
                Field::Default => "NULL",
            }
            .to_owned()
        } else {
            value[..value.floor_char_boundary(value.len().min(120))].into()
        };
        let text = if placeholder {
            cx.theme().muted_foreground.opacity(0.3)
        } else {
            cx.theme().foreground
        };
        cell.child(
            self.press(format!("field-{id:?}-{field:?}"), move |v, w, cx| {
                let Some(draft) = v
                    .properties
                    .as_ref()
                    .and_then(|p| p.editor.as_ref())
                    .map(|e| &e.draft)
                else {
                    return;
                };
                let value = if field == Field::KeyName {
                    draft
                        .primary_key()
                        .filter(|k| k.id() == id)
                        .and_then(|k| k.current())
                        .or_else(|| {
                            draft
                                .added_objects()
                                .iter()
                                .find(|(i, _)| *i == id)
                                .map(|(_, o)| o.key())
                        })
                        .map(|k| SharedString::from(k.name.as_str()))
                } else {
                    draft
                        .columns()
                        .iter()
                        .find(|r| r.id() == id)
                        .and_then(|r| r.current())
                        .and_then(|c| match field {
                            Field::Name => Some(SharedString::from(c.name.as_str())),
                            Field::Type => Some(SharedString::from(c.data_type.as_str())),
                            Field::Default => Some(
                                c.default
                                    .as_ref()
                                    .map(SharedString::from)
                                    .unwrap_or_default(),
                            ),
                            Field::KeyName => None,
                        })
                };
                if let Some(value) = value {
                    v.edit_field(id, field, value, w, cx);
                }
            })
            .accessibility_label(label.clone())
            .w_full()
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .justify_start()
            .rounded(px(6.))
            .border_1()
            .border_color(cx.theme().border.opacity(0.3))
            .bg(cx.theme().background)
            .text_size(px(12.))
            .text_color(text)
            .child(div().min_w_0().truncate().child(label)),
        )
    }

    pub(super) fn remove_restore(
        &self,
        e: &Editor,
        id: EditId,
        cx: &App,
    ) -> gpui_kit::base::Button {
        let removed = e.draft.is_dropped(id);
        let action = if removed { "Restore" } else { "Remove" };
        let muted = cx.theme().muted_foreground;
        let hover = if removed {
            cx.theme().foreground
        } else {
            cx.theme().danger
        };
        self.drop_toggle(id, removed)
            .accessibility_label(action)
            .size(px(24.))
            .rounded(px(4.))
            .text_color(muted)
            .hover(move |s| s.text_color(hover).bg(hover.opacity(0.10)))
            .child(
                gpui_kit::component::Icon::new(if removed {
                    IconName::RefreshCw
                } else {
                    IconName::Trash
                })
                .size(px(14.)),
            )
    }

    pub(super) fn drop_toggle(&self, id: EditId, removed: bool) -> gpui_kit::base::Button {
        self.press(format!("remove-{id:?}"), move |v, _, cx| {
            v.edit_draft(cx, |d| {
                if removed {
                    let _ = d.restore(id);
                } else {
                    let _ = d.drop_entry(id);
                }
            });
            if let Some(e) = v.properties.as_mut().and_then(|p| p.editor.as_mut())
                && e.input.as_ref().is_some_and(|i| i.id == id)
            {
                e.input = None;
            }
        })
    }
}
