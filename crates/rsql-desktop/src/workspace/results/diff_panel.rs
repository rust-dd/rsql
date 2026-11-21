use super::*;
use gpui_kit::component::{label::Label, tooltip::Tooltip};
use rsql_desktop::results::{
    DiffWork, FieldLocation, ViewChoice,
    diff::{Comparison, DiffKey, compare},
};
use tokio::sync::watch;

impl Workspace {
    pub(in crate::workspace) fn sync_diff(&mut self, cx: &mut Context<Self>) {
        if *self.session.preflight_stop.borrow() {
            return;
        }
        let mut docs = self.session.documents.borrow_mut();
        let Some(tab) = docs.selected.map(|i| &mut docs.tabs[i]) else {
            return;
        };
        let tab_id = tab.id.clone();
        let Some(view) = tab
            .main
            .view_mut()
            .filter(|v| v.choice() == ViewChoice::Diff)
        else {
            return;
        };
        let Some(pin) = self.session.pin.borrow().clone() else {
            view.choose(ViewChoice::Grid);
            docs.sync_views();
            return;
        };
        let key = DiffKey {
            pin: pin.id,
            original: pin.rows.key,
            current: view.key(),
        };
        if view.comparison.as_ref().is_some_and(|d| d.key == key) {
            return;
        }
        let Some(rows) = view.snapshot(ResultAction::Diff) else {
            return;
        };
        let (owner, cancel) = watch::channel(());
        let stopped = self.session.preflight_stop.subscribe();
        let route = view.route.clone();
        view.comparison = Some(DiffWork {
            key,
            owner,
            result: None,
        });
        let worker = self.session.runtime.spawn_blocking(move || {
            let current =
                || cancel.has_changed().is_ok() && !*stopped.borrow() && route.is_current();
            compare(&pin.rows, &rows, current)
        });
        cx.spawn(async move |v, cx| {
            let Ok(Some(result)) = worker.await else {
                return;
            };
            let _ = v.update(cx, |v, cx| {
                let mut docs = v.session.documents.borrow_mut();
                if v.session
                    .pin
                    .borrow()
                    .as_ref()
                    .is_some_and(|p| p.id == key.pin)
                    && let Some(d) = docs
                        .tabs
                        .iter_mut()
                        .find(|t| t.id == tab_id)
                        .and_then(|t| t.main.view_mut())
                        .filter(|v| v.key() == key.current && v.choice() == ViewChoice::Diff)
                        .and_then(|v| v.comparison.as_mut())
                        .filter(|d| d.key == key)
                {
                    d.result = Some(Arc::new(result));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(in crate::workspace) fn diff_panel(&self, view: &ResultView, cx: &Context<Self>) -> Div {
        if let Some(detail) = self.field_detail(view, cx) {
            return detail;
        }
        let panel = div()
            .id("diff-panel")
            .flex()
            .flex_col()
            .min_h_0()
            .flex_1()
            .text_xs()
            .p_2();
        let Some(result) = view.comparison.as_ref().and_then(|d| d.result.clone()) else {
            return div().child("Computing diff…");
        };
        let Some(pin) = self.session.pin.borrow().clone() else {
            return div();
        };
        let Comparison::Rows {
            added,
            removed,
            unchanged,
        } = &*result
        else {
            let labels = |columns: &[String]| {
                columns
                    .iter()
                    .map(|v| clipped(v))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            return div()
                .child(Label::new("Column structures differ"))
                .child(Label::new(format!("Pinned: {}", labels(&pin.rows.columns))))
                .child(Label::new(format!(
                    "Current: {}",
                    labels(view.data.columns())
                )));
        };
        let Some(rows) = view.snapshot(ResultAction::Diff) else {
            return div();
        };
        let summary = format!(
            "{} added · {} removed · {} unchanged",
            added.len(),
            removed.len(),
            unchanged
        );
        let count = added.len() + removed.len();
        let width = px((rows.columns.len() * 128 + 24) as f32);
        let mut table = div().flex().gap_2().child("±");
        for label in rows.columns.iter() {
            table = table.child(div().w(px(120.)).truncate().child(clipped(label)));
        }
        let owner = super::fields::FieldOwner::new(view);
        let workspace = cx.weak_entity();
        let list = uniform_list("result-differences", count, move |range, _, cx| {
            let Comparison::Rows { added, removed, .. } = &*result else {
                unreachable!()
            };
            range
                .map(|i| {
                    let (page, index, sign, color) = if let Some(&index) = removed.get(i) {
                        (&pin.rows.page, index, "−", cx.theme().danger)
                    } else {
                        (
                            &rows.page,
                            added[i - removed.len()],
                            "+",
                            cx.theme().success,
                        )
                    };
                    let mut line = div()
                        .id(("diff-row", i))
                        .test_support()
                        .flex()
                        .gap_2()
                        .text_color(color)
                        .child(sign);
                    for (column, cell) in page[index].iter().enumerate() {
                        let mut preview = super::fields::Preview::new(match cell.as_deref() {
                            None => "SQL NULL",
                            Some("") => "empty string",
                            Some(s) => s,
                        });
                        if cell.as_ref().is_some_and(|s| !s.is_empty()) {
                            preview.text = format!("Text: {}", preview.text).into();
                        }
                        let mut field = div()
                            .id(("diff-cell", column))
                            .test_support()
                            .w(px(120.))
                            .truncate();
                        let tip = if preview.partial {
                            let workspace = workspace.clone();
                            let location = FieldLocation {
                                pinned: removed.get(i).map(|_| pin.id),
                                row: index,
                                column,
                                label: false,
                            };
                            field = field.cursor_pointer().on_click(move |_, _, cx| {
                                let _ = workspace.update(cx, |v, cx| {
                                    v.change_field(owner, cx, |r| {
                                        r.field_detail = Some((location, 0))
                                    })
                                });
                            });
                            format!("{}… · Click to read the full field", preview.text).into()
                        } else {
                            preview.text.clone()
                        };
                        let display = if preview.partial {
                            format!("{}…", preview.text).into()
                        } else {
                            preview.text
                        };
                        line = line.child(
                            field
                                .role(Role::Cell)
                                .aria_label(display.clone())
                                .child(display)
                                .tooltip(move |w, cx| {
                                    Tooltip::element({
                                        let tip = tip.clone();
                                        move |_, _| Label::new(tip.clone()).id("diff-tooltip-text")
                                    })
                                    .build(w, cx)
                                }),
                        );
                    }
                    line
                })
                .collect::<Vec<_>>()
        })
        .flex_1();
        div().flex().flex_1().min_h_0().child(
            panel.child(Label::new(summary)).child(
                div()
                    .id("diff-horizontal-scroll")
                    .overflow_x_scroll()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .flex_1()
                    .child(
                        div()
                            .w(width)
                            .min_w_full()
                            .min_h_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(table)
                            .child(list),
                    ),
            ),
        )
    }
}
