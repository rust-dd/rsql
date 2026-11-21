use super::*;
use chrono::{DateTime, Local};
use gpui_kit::base::TestTextExt;
use gpui_kit::component::{Disableable, Icon, button::Button, tooltip::Tooltip};
use rsql_core::storage::history::HistoryEntry;

pub(super) struct Row {
    pub entry: HistoryEntry,
    preview: Vec<SharedString>,
    when: SharedString,
}

impl Row {
    pub(super) fn new(entry: HistoryEntry) -> Self {
        let mut chars = entry.sql.chars();
        let mut preview = chars.by_ref().take(240).collect::<String>();
        if chars.next().is_some() {
            preview.push('…');
        }
        let mut lines = preview.split('\n');
        let mut preview = lines
            .by_ref()
            .take(4)
            .map(|line| SharedString::from(line.trim_end_matches('\r').to_owned()))
            .collect::<Vec<_>>();
        if lines.next().is_some()
            && let Some(last) = preview.last_mut()
        {
            *last = format!("{last}…").into();
        }
        let when = DateTime::from_timestamp_millis(entry.executed_at)
            .map(|t| {
                t.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|| entry.executed_at.to_string());
        Self {
            entry,
            preview,
            when: when.into(),
        }
    }
}

impl Panel {
    pub(super) fn row(
        &self,
        row: &Row,
        generation: u64,
        panel: &Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let entry = &row.entry;
        let id = entry.id;
        let profiles = self.session.connections.borrow();
        let connection = profiles
            .profiles
            .iter()
            .find(|p| p.id == entry.project_id)
            .map(|p| p.display_name())
            .unwrap_or("Unknown connection");
        let route = if entry.database.is_empty() {
            connection.to_owned()
        } else {
            format!("{connection} · {}", entry.database)
        };
        let preview = div()
            .id(("history-preview", id as usize))
            .test_support()
            .w_full()
            .max_h(px(72.))
            .overflow_hidden()
            .flex()
            .flex_col()
            .text_size(px(12.))
            .line_height(px(18.))
            .children(row.preview.iter().enumerate().map(|(line, text)| {
                div()
                    .id(("sql-line", line))
                    .test_support()
                    .w_full()
                    .flex_shrink_0()
                    .min_h(px(18.))
                    .whitespace_normal()
                    .line_clamp(4)
                    .text_ellipsis()
                    .child(text.clone())
                    .test_text(text.clone())
            }));
        let mut content = div().flex_1().min_w_0().child(preview);
        if let Some(error) = &entry.error {
            content = content.child(
                div()
                    .mt_1()
                    .truncate()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        let mut details = div()
            .mt_1()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_3()
            .gap_y_0p5()
            .text_size(px(10.))
            .text_color(cx.theme().muted_foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(Icon::default().path("icons/clock-3.svg").size(px(10.)))
                    .child(row.when.clone()),
            )
            .child(
                div()
                    .max_w_full()
                    .min_w_0()
                    .truncate()
                    .child(route.clone())
                    .test_text(route),
            );
        if entry.success {
            details = details.child(format!("{} rows", entry.row_count));
        }
        details = details.child(format!("{:.0}ms", entry.duration_ms));
        if entry.sql_truncated {
            details = details.child(
                div()
                    .id(("history-truncated", id as usize))
                    .test_support()
                    .flex_shrink_0()
                    .child("truncated")
                    .test_text("truncated")
                    .tooltip(|w, cx| {
                        Tooltip::new(
                            "Only the beginning of this script was kept, so it cannot be restored",
                        )
                        .build(w, cx)
                    }),
            );
        }
        content = content.child(details);
        let mut el = div()
            .id(("history-row", id as usize))
            .test_support()
            .w_full()
            .min_w_0()
            .py_2p5()
            .flex()
            .items_start()
            .gap_2()
            .px_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Icon::default()
                    .path(if entry.success {
                        "icons/circle-check.svg"
                    } else {
                        "icons/circle-x.svg"
                    })
                    .size(px(14.))
                    .text_color(if entry.success {
                        cx.theme().success
                    } else {
                        cx.theme().danger
                    }),
            )
            .child(content);
        let restorable = !entry.sql_truncated && self.owns_idle_pane();
        if restorable {
            let panel = panel.downgrade();
            el = el
                .cursor_pointer()
                .hover(|el| el.bg(cx.theme().accent.opacity(0.5)))
                .on_click(move |_, w, cx| {
                    let _ = panel.update(cx, |p, cx| p.restore(id, generation, false, w, cx));
                });
        }
        for (label, action, icon, replace) in [
            (
                "Open in a new tab",
                "history-open",
                "icons/external-link.svg",
                false,
            ),
            (
                "Replace the current editor",
                "history-replace",
                "icons/rotate-ccw.svg",
                true,
            ),
        ] {
            let panel = panel.downgrade();
            el = el.child(
                compact_button(Button::new((action, id as usize)))
                    .ghost()
                    .icon(Icon::default().path(icon))
                    .accessibility_label(label)
                    .tooltip(label)
                    .disabled(!restorable)
                    .on_click(move |_, w, cx| {
                        cx.stop_propagation();
                        let _ = panel.update(cx, |p, cx| p.restore(id, generation, replace, w, cx));
                    }),
            );
        }
        el.into_any_element()
    }

    fn restore(
        &self,
        id: i64,
        generation: u64,
        replace: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.current(generation) || !self.owns_idle_pane() {
            return;
        }
        let Some(entry) = self
            .rows
            .iter()
            .map(|r| &r.entry)
            .find(|e| e.id == id && !e.sql_truncated)
        else {
            return;
        };
        let _ = self.workspace.update(cx, |v, cx| {
            v.session.apply_removals();
            if replace {
                let side = v
                    .session
                    .documents
                    .borrow()
                    .active()
                    .map(|t| t.focused_side)
                    .unwrap_or_default();
                if let Some(state) = v.editor(side) {
                    state.update(cx, |s, cx| {
                        s.set_selected_range(0..s.text().len(), cx);
                        s.replace(entry.sql.clone(), w, cx);
                        s.focus(w, cx);
                    });
                }
            } else {
                let project = v
                    .session
                    .connections
                    .borrow()
                    .profiles
                    .iter()
                    .find(|p| p.id == entry.project_id)
                    .map(|p| p.id.clone());
                v.update_documents(w, cx, |docs| {
                    docs.open(Kind::Query, project)
                        .main
                        .draft
                        .text
                        .clone_from(&entry.sql);
                });
            }
        });
    }
}
