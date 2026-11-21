use super::super::theme::controls::compact_button;
use super::fields::{FIELD_COUNT, FieldOwner};
use super::*;
use gpui_kit::{
    component::{Disableable, label::Label},
    prelude::FluentBuilder,
};
use rsql_desktop::results::{FieldLocation, ViewChoice};

impl Workspace {
    pub(in crate::workspace) fn record_panel(&self, view: &ResultView, cx: &Context<Self>) -> Div {
        if let Some(detail) = self.field_detail(view, cx) {
            return detail;
        }
        let mut panel = div()
            .id("record-panel")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .text_xs()
            .line_height(px(16.));
        let Some(rows) = view.snapshot(ResultAction::Record) else {
            return div().child("Rows unavailable");
        };
        if rows.is_empty() {
            return div().child(Label::new("No rows to display"));
        }
        let n = rows.len();
        let index = view.record_index.min(n - 1);
        let key = view.key();
        let step = |forward: bool| {
            compact_button(self.button(
                if forward { "Next" } else { "Prev" },
                cx,
                move |v, _, cx| {
                    let mut docs = v.session.documents.borrow_mut();
                    if let Some(view) = docs
                        .selected
                        .map(|i| &mut docs.tabs[i])
                        .and_then(|t| t.main.view_mut())
                        .filter(|v| v.key() == key && v.choice() == ViewChoice::Record)
                    {
                        view.record_index = if forward {
                            view.record_index.saturating_add(1).min(n - 1)
                        } else {
                            view.record_index.min(n - 1).saturating_sub(1)
                        };
                        view.record_fields_start = 0;
                        cx.notify();
                    }
                },
            ))
            .outline()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .h(px(32.))
            .text_size(px(14.))
            .disabled(if forward { index + 1 >= n } else { index == 0 })
        };
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .mb_2()
                .child(step(false))
                .child(
                    Label::new(format!("Row {} of {}", index + 1, n))
                        .text_size(px(14.))
                        .text_color(cx.theme().muted_foreground),
                )
                .child(step(true)),
        );
        let row = rows.source_index(index).unwrap();
        let start = view
            .record_fields_start
            .min(rows.columns.len().saturating_sub(1));
        let owner = FieldOwner::new(view);
        if rows.columns.len() > FIELD_COUNT {
            let move_fields = |label: &'static str, to: usize| {
                compact_button(self.button(label, cx, move |v, _, cx| {
                    v.change_field(owner, cx, |r| r.record_fields_start = to)
                }))
                .disabled(to == start)
            };
            let next = if start + FIELD_COUNT < rows.columns.len() {
                start + FIELD_COUNT
            } else {
                start
            };
            panel = panel.child(
                div()
                    .flex()
                    .gap_1()
                    .child(move_fields(
                        "Previous fields",
                        start.saturating_sub(FIELD_COUNT),
                    ))
                    .child(Label::new(format!(
                        "Fields {}–{} of {}",
                        start + 1,
                        (start + FIELD_COUNT).min(rows.columns.len()),
                        rows.columns.len()
                    )))
                    .child(move_fields("Next fields", next)),
            );
        }
        let fields = self.field_cache.borrow_mut().record(&rows, row, start);
        for (offset, field) in fields.iter().enumerate() {
            let column = start + offset;
            let open = |label: bool| {
                let location = FieldLocation {
                    pinned: None,
                    row,
                    column,
                    label,
                };
                let name = if label {
                    "Read column name"
                } else {
                    "Read field"
                };
                compact_button(
                    gpui_kit::component::button::Button::new((
                        if label { "read-label" } else { "read-field" },
                        column,
                    ))
                    .label(name)
                    .on_click(cx.listener(move |v, _, _, cx| {
                        v.change_field(owner, cx, |r| r.field_detail = Some((location, 0)))
                    })),
                )
            };
            let value = div().min_w_0().flex_1().px_3().py_2().whitespace_normal();
            let value = match &field.value {
                None => value.child(
                    div().flex().items_start().child(
                        div()
                            .rounded_sm()
                            .bg(cx.theme().muted)
                            .px_1p5()
                            .py_0p5()
                            .child(
                                Label::new("NULL")
                                    .id(("record-null", column))
                                    .text_color(cx.theme().muted_foreground),
                            ),
                    ),
                ),
                Some(p) if p.text.is_empty() => value
                    .italic()
                    .text_color(cx.theme().muted_foreground)
                    .child(Label::new("empty string").id(("record-empty", column))),
                Some(p) => value
                    .when(p.partial, |cell| {
                        cell.child(open(false)).child(" · preview")
                    })
                    .child(Label::new(p.text.clone()).id(("record-value", column))),
            };
            let mut label = div()
                .w_1_3()
                .min_w_0()
                .flex_shrink_0()
                .font_weight(FontWeight::SEMIBOLD)
                .px_3()
                .py_2();
            if field.label.partial {
                label = label.child(open(true)).child(" · preview");
            }
            label = label.child(Label::new(field.label.text.clone()).id(("record-label", column)));
            panel = panel.child(
                div()
                    .flex()
                    .items_start()
                    .when(offset % 2 == 1, |row| row.bg(cx.theme().muted.opacity(0.3)))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(label)
                    .child(value),
            );
        }
        div().flex().flex_col().flex_1().min_h_0().child(panel)
    }
}
