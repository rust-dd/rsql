use super::*;
use gpui_kit::base::{ElementExt as _, Scrollbar};
use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;

impl Dialog {
    pub(super) fn preview_table(
        &self,
        preview: &Preview,
        w: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let count = preview.labels.len();
        let natural = *preview.edges.last().unwrap_or(&0.);
        let viewport = if self.table_width > px(0.) {
            self.table_width
        } else {
            dialog::frame_width(w).min(w.viewport_size().width - px(32.)) - px(44.)
        };
        let viewport = f64::from(viewport.max(px(1.)));
        let owner = cx.weak_entity();
        let total = natural.max(viewport);
        let scale = total / natural.max(1.);
        let left = f64::from(-self.table_scroll.offset().x).clamp(0., total - viewport);
        let first = preview
            .edges
            .partition_point(|edge| *edge * scale <= left)
            .saturating_sub(1);
        let end = preview
            .edges
            .partition_point(|edge| *edge * scale < left + viewport)
            .min(count);
        let t = cx.theme();
        let rows = std::iter::once(&preview.labels)
            .chain(preview.rows.iter())
            .enumerate()
            .map(|(r, values)| {
                let header = r == 0;
                div()
                    .id(("csv-preview-row", r))
                    .role(Role::Row)
                    .aria_row_index(r)
                    .w(px(total as f32))
                    .pl(px((preview.edges[first] * scale) as f32))
                    .flex()
                    .items_center()
                    .h(px(if header { 28. } else { 25. }))
                    .when(header, |row| {
                        row.bg(t.muted.opacity(0.3))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.muted_foreground)
                    })
                    .when(!header, |row| {
                        row.border_t_1().border_color(t.border.opacity(0.2))
                    })
                    .children((first..end).map(|i| {
                        let value = values.get(i).cloned().unwrap_or_default();
                        div()
                            .id(("csv-preview-cell", i))
                            .role(if header {
                                Role::ColumnHeader
                            } else {
                                Role::Cell
                            })
                            .aria_column_index(i)
                            .aria_label(value.clone())
                            .w(px(
                                ((preview.edges[i + 1] - preview.edges[i]) * scale) as f32
                            ))
                            .flex_shrink_0()
                            .px_3()
                            .truncate()
                            .child(value)
                    }))
            });
        div()
            .id("csv-preview")
            .relative()
            .role(Role::Table)
            .aria_label("CSV preview")
            .aria_row_count(preview.rows.len() + 1)
            .aria_column_count(count)
            .border_1()
            .border_color(t.border.opacity(0.4))
            .rounded(px(12.))
            .overflow_hidden()
            .child(
                div()
                    .id("csv-preview-scroll")
                    .overflow_x_scroll()
                    .track_scroll(&self.table_scroll)
                    .max_h(px(200.))
                    .on_prepaint(move |bounds, _, cx| {
                        let _ = owner.update(cx, |view, cx| {
                            if view.table_width != bounds.size.width {
                                view.table_width = bounds.size.width;
                                cx.notify();
                            }
                        });
                    })
                    .children(rows),
            )
            .child(Scrollbar::horizontal(&self.table_scroll))
    }
}
