use super::super::editor::{Editor, Field};
use super::{header::icon, structure::stack, *};
use gpui_kit::{
    assets::IconName,
    component::{Sizable, checkbox::Checkbox},
};
use rsql_desktop::structure::KeyColumns;

impl Panel {
    pub(super) fn primary_key_rows(&self, e: &Editor, cx: &App) -> Div {
        let entry = e.draft.primary_key();
        let active = entry.and_then(|k| k.current());
        let primary = cx.theme().primary;
        let muted = cx.theme().muted;
        let caption = div()
            .px(px(4.))
            .text_size(px(10.))
            .line_height(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().muted_foreground.opacity(0.6))
            .child("SELECT COLUMNS FOR PRIMARY KEY");
        let mut choices = stack();
        for column in e.draft.columns() {
            let Some(value) = column.current() else {
                continue;
            };
            let id = column.id();
            let control = entry.map_or_else(
                || format!("pk-choice-{id:?}"),
                |key| format!("key-{:?}-{id:?}", key.id()),
            );
            let enabled = active.is_some_and(|key| key.columns.contains(&id));
            choices = choices.child(
                Checkbox::new(control)
                    .with_size(px(14.))
                    .checked(enabled)
                    .accessibility_label(value.name.clone())
                    .w_full()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if enabled {
                        primary.opacity(0.2)
                    } else {
                        cx.theme().border.opacity(0.2)
                    })
                    .when(enabled, |d| d.bg(primary.opacity(0.05)))
                    .hover(move |s| {
                        s.bg(if enabled {
                            primary.opacity(0.05)
                        } else {
                            muted.opacity(0.2)
                        })
                    })
                    .on_change(self.listener(true, move |v, _, cx| {
                        v.edit_draft(cx, |draft| {
                            let entry = draft.primary_key();
                            let mut key =
                                entry.and_then(|k| k.current()).cloned().unwrap_or_else(|| {
                                    KeyColumns {
                                        name: entry.and_then(|k| k.original()).map_or_else(
                                            || {
                                                default_name(
                                                    &draft.snapshot().object().name,
                                                    "pkey",
                                                )
                                            },
                                            |k| k.name.clone(),
                                        ),
                                        columns: Vec::new(),
                                    }
                                });
                            if let Some(index) = key.columns.iter().position(|&column| column == id)
                            {
                                key.columns.remove(index);
                            } else {
                                key.columns.push(id);
                            }
                            if key.columns.is_empty() {
                                if let Some(existing) = entry {
                                    let _ = draft.drop_entry(existing.id());
                                }
                            } else {
                                draft.set_primary_key(key);
                            }
                        });
                    }))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(12.))
                            .child(value.name.clone()),
                    )
                    .when(enabled, |d| d.child(icon(IconName::Key, 12., primary))),
            );
        }
        let mut rows = stack()
            .gap(px(12.))
            .py(px(4.))
            .child(caption)
            .child(choices);
        if let Some((key, value)) = entry
            .zip(active)
            .filter(|(_, value)| !value.columns.is_empty())
        {
            let names = value
                .columns
                .iter()
                .filter_map(|id| e.draft.columns().iter().find(|c| c.id() == *id))
                .filter_map(|c| c.current())
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(4.))
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child(div().flex_shrink_0().child("Constraint:"))
                    .child(div().flex_1().min_w_0().max_w(px(320.)).child(self.field(
                        e,
                        key.id(),
                        Field::KeyName,
                        &value.name,
                        cx,
                    )))
                    .child(div().min_w_0().truncate().child(format!("— ({names})"))),
            );
        }
        rows
    }
}

/// Source-style `{table}_{suffix}` name, shortened on a character boundary to PostgreSQL's 63 bytes.
pub(super) fn default_name(table: &str, suffix: &str) -> String {
    let room = 63_usize.saturating_sub(suffix.len() + 1);
    let end = table.floor_char_boundary(table.len().min(room));
    format!("{}_{suffix}", &table[..end])
}
