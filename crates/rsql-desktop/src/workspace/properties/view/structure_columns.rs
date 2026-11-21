use super::super::editor::{Editor, Field};
use super::*;
use super::{
    header::icon,
    structure::{line, stack},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable, Sizable, checkbox::Checkbox};
use rsql_desktop::structure::{EditId, PG_COMMON_TYPES};
use std::sync::Arc;

fn track(row: &str, number: usize, child: impl IntoElement) -> impl IntoElement {
    let cell = div().min_w_0().overflow_hidden();
    let cell = match number {
        1 => cell.w(px(140.)).flex_shrink_0(),
        2 => cell.w(px(70.)).flex_shrink_0().flex().justify_center(),
        4 => cell.w(px(36.)).flex_shrink_0().flex().justify_center(),
        _ => cell.flex_1(),
    };
    cell.id(format!("column-{row}-{number}"))
        .test_support()
        .child(child)
}

fn grid() -> Div {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .flex_shrink_0()
        .px(px(4.))
        .border_1()
        .relative()
}

fn removed(value: &str, cx: &App) -> Div {
    div()
        .min_w_0()
        .h(px(28.))
        .px(px(8.))
        .truncate()
        .rounded(px(6.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.3))
        .bg(cx.theme().background)
        .text_color(cx.theme().muted_foreground)
        .line_height(px(26.))
        .child(
            Label::new(value.to_owned())
                .text_color(cx.theme().muted_foreground)
                .line_height(px(26.)),
        )
}

impl Panel {
    fn removed_default(&self, id: EditId, value: Option<&Arc<str>>, cx: &App) -> Div {
        let Some(value) = value else {
            return removed("", cx);
        };
        let end = value.floor_char_boundary(value.len().min(24));
        let end = value[..end].find('\n').unwrap_or(end);
        if end == value.len() {
            return removed(value, cx);
        }
        let preview = format!("{}…", &value[..end]);
        let full = value.clone();
        div()
            .flex()
            .min_w_0()
            .items_center()
            .gap(px(4.))
            .child(removed(&preview, cx).flex_1())
            .child(
                self.press(format!("copy-removed-default-{id:?}"), move |v, w, cx| {
                    v.transfer_property_text(full.clone(), true, w, cx);
                })
                .accessibility_label("Copy original default")
                .w(px(24.))
                .h(px(28.))
                .flex_shrink_0()
                .rounded(px(4.))
                .child(icon(IconName::Copy, 12., cx.theme().muted_foreground)),
            )
    }

    pub(super) fn structure_columns(&self, e: &Editor, cx: &App) -> Div {
        let header = grid()
            .border_color(cx.theme().transparent)
            .children(
                ["NAME", "TYPE", "NULLABLE", "DEFAULT", ""]
                    .into_iter()
                    .enumerate()
                    .map(|(number, label)| track("header", number, label)),
            )
            .text_size(px(9.))
            .line_height(px(13.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().muted_foreground.opacity(0.6));
        let mut rows = stack().child(header);
        for entry in e.draft.columns() {
            let id = entry.id();
            let c = entry.current().or(entry.original()).unwrap();
            let gone = entry.current().is_none();
            let edge = match (entry.original(), entry.current()) {
                (None, _) => Some(cx.theme().success),
                (_, None) => Some(cx.theme().danger),
                (Some(a), Some(b)) if a != b => Some(cx.theme().warning),
                _ => None,
            };
            let nullable = Checkbox::new(format!("nullable-{id:?}"))
                .with_size(px(14.))
                .checked(c.nullable)
                .disabled(gone)
                .accessibility_label(if c.nullable { "NULL" } else { "NOT NULL" })
                .on_change(self.listener(true, move |v, _, cx| {
                    v.edit_draft(cx, |d| {
                        if let Some(c) = d.column_mut(id) {
                            c.nullable = !c.nullable;
                        }
                    });
                }));
            let field = |field, value| {
                if gone {
                    removed(value, cx)
                } else {
                    self.field(e, id, field, value, cx)
                }
            };
            let row_id = format!("{id:?}");
            rows = rows.child(
                grid()
                    .id(format!("structure-column-{row_id}"))
                    .test_support()
                    .py(px(4.))
                    .rounded(px(8.))
                    .overflow_hidden()
                    .border_color(cx.theme().border.opacity(0.2))
                    .opacity(if gone { 0.6 } else { 1. })
                    .when_some(edge, |d, color| {
                        d.bg(color.opacity(0.05)).child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left_0()
                                .w(px(2.))
                                .bg(color),
                        )
                    })
                    .child(track(&row_id, 0, field(Field::Name, &c.name)))
                    .child(track(&row_id, 1, field(Field::Type, &c.data_type)))
                    .child(track(&row_id, 2, nullable))
                    .child(track(
                        &row_id,
                        3,
                        if gone {
                            self.removed_default(id, c.default.as_ref(), cx)
                        } else {
                            self.field(
                                e,
                                id,
                                Field::Default,
                                c.default.as_deref().unwrap_or(""),
                                cx,
                            )
                        },
                    ))
                    .child(track(&row_id, 4, self.remove_restore(e, id, cx))),
            );
        }
        if let Some(input) = e.input.as_ref().filter(|i| i.field == Field::Type) {
            let prefix = input.input.read(cx).value().to_lowercase();
            let mut suggestions = line();
            for ty in PG_COMMON_TYPES
                .iter()
                .copied()
                .filter(|ty| ty.starts_with(&prefix))
                .take(8)
            {
                let id = input.id;
                suggestions = suggestions.child(
                    self.button(format!("type-{ty}"), move |v, _, cx| {
                        v.edit_draft(cx, |d| {
                            if let Some(c) = d.column_mut(id) {
                                c.data_type = ty.into();
                            }
                        });
                        v.properties
                            .as_mut()
                            .unwrap()
                            .editor
                            .as_mut()
                            .unwrap()
                            .input = None;
                    })
                    .label(ty),
                );
            }
            rows = rows.child(suggestions);
        }
        rows
    }
}
