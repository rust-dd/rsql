use super::*;
use crate::workspace::{
    results::{FIELD_COUNT, Preview},
    theme::controls::compact_button,
};
use gpui_kit::component::{ActiveTheme, button::Button, label::Label};
use rsql_desktop::results::FieldLocation;

impl ResultMap {
    pub(super) fn popup(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let rows = self.rows.as_ref()?;
        let selected = self.selected?;
        let row = rows.page.get(selected)?;
        let geom = self.detected.flatten()?;
        let key = rows.key;
        let count = rows.columns.len().saturating_sub(1);
        if count == 0 {
            return None;
        }
        let mut popup = div()
            .id("map-popup")
            .test_support()
            .absolute()
            .right_2()
            .top_2()
            .max_h(relative(0.65))
            .max_w(relative(0.65))
            .overflow_scroll()
            .flex()
            .flex_col()
            .occlude()
            .p_2()
            .rounded_md()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .text_xs();
        let start = self.field_start;
        if start > 0 {
            popup = popup.child(self.control(
                "Previous popup fields",
                "Previous fields",
                cx,
                |v, _, _| v.field_start = v.field_start.saturating_sub(FIELD_COUNT),
            ));
        }
        if start + FIELD_COUNT < count {
            popup = popup.child(
                self.control("Next popup fields", "Next fields", cx, |v, _, _| {
                    v.field_start += FIELD_COUNT
                }),
            );
        }
        for ordinal in start..(start + FIELD_COUNT).min(count) {
            let column = ordinal + usize::from(ordinal >= geom);
            let label = Preview::new(&rows.columns[column]);
            let value = row
                .get(column)
                .and_then(|cell| cell.as_deref())
                .map(Preview::new);
            let read = |label: bool| {
                compact_button(
                    Button::new((
                        if label {
                            "map-read-name"
                        } else {
                            "map-read-value"
                        },
                        column,
                    ))
                    .label(if label { "Read name" } else { "Read field" }),
                )
                .on_click(cx.listener(move |v, _, _, cx| {
                    let mut docs = v.session.documents.borrow_mut();
                    if let Some(view) = docs
                        .selected
                        .map(|i| &mut docs.tabs[i])
                        .filter(|t| t.id == v.tab)
                        .and_then(|t| t.main.view_mut())
                        .filter(|r| {
                            r.choice() == ViewChoice::Map
                                && r.key() == key
                                && v.selected == Some(selected)
                        })
                    {
                        view.field_detail = Some((
                            FieldLocation {
                                pinned: None,
                                row: selected,
                                column,
                                label,
                            },
                            0,
                        ));
                    }
                    drop(docs);
                    let _ = v.workspace.update(cx, |_, cx| cx.notify());
                }))
            };
            let mut field = div().flex().gap_1().flex_wrap().child(
                Label::new(format!("{}:", label.text))
                    .id(("map-field-name", column))
                    .font_weight(FontWeight::BOLD),
            );
            if label.partial {
                field = field.child(read(true));
            }
            field = match value {
                None => field.child(
                    Label::new("SQL NULL")
                        .id(("map-null", column))
                        .italic()
                        .text_color(cx.theme().muted_foreground),
                ),
                Some(value) if value.text.is_empty() => field.child(
                    Label::new("empty string")
                        .id(("map-empty", column))
                        .italic(),
                ),
                Some(value) => {
                    field = field.child(
                        Label::new(value.text)
                            .id(("map-field", column))
                            .whitespace_normal(),
                    );
                    if value.partial {
                        field = field.child(read(false));
                    }
                    field
                }
            };
            popup = popup.child(field);
        }
        Some(popup)
    }
}
