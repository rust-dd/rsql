use super::*;
use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme, Icon, label::Label},
};
use rsql_desktop::{
    document::Tab,
    execution::Status,
    results::{ResultAction, ViewChoice},
};

impl Workspace {
    pub(super) fn sync_grids(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let docs = self.session.documents.borrow();
        self.field_cache
            .borrow_mut()
            .retain_for(docs.active().and_then(|t| t.main.view()));
        let mut restart = Vec::new();
        for (owner, live) in &mut self.editors {
            let pane = docs
                .tabs
                .iter()
                .find(|t| t.id == owner.0 && (owner.1 == Side::Main || t.split))
                .map(|t| t.pane(owner.1));
            let view = pane.and_then(|p| p.retained_view());
            let search = view
                .filter(|v| {
                    owner.1 == Side::Main
                        && v.edit.is_none()
                        && (v.allows(ResultAction::Search)
                            || live.search.as_ref().is_some_and(|s| s.id == v.data.id))
                })
                .map(|v| v.data.id);
            if live.search.as_ref().map(|s| s.id) != search {
                live.search = search.map(|id| results::SearchInput::new(owner.clone(), id, w, cx));
            }
            let Some(view) = view else {
                if let Some(map) = live.map.take() {
                    map.update(cx, |v, cx| v.suspend(w, cx));
                }
                live.grid_events.clear();
                live.grid = None;
                continue;
            };
            if owner.1 == Side::Main {
                let foreground = docs.active().is_some_and(|t| t.id == owner.0)
                    && pane.is_some_and(|p| !p.is_busy());
                if foreground && live.map.is_none() {
                    let workspace = cx.weak_entity();
                    live.map = Some(cx.new(|cx| {
                        map::ResultMap::new(self.session.clone(), workspace, owner.0.clone(), w, cx)
                    }));
                }
                if let Some(map) = &live.map {
                    map.update(cx, |m, cx| {
                        m.sync(view, pane.unwrap().generation(), foreground, w, cx)
                    });
                }
            }
            if view.restart_search
                && view.allows(ResultAction::Search)
                && pane.is_some_and(|p| !p.is_busy())
                && let Some(input) = &live.search
            {
                restart.push((owner.clone(), view.data.id, input.text(cx)));
            }
            let active = docs.active().is_some_and(|t| t.id == owner.0)
                && pane.is_some_and(|p| !p.is_busy())
                && view.choice() == ViewChoice::Grid;
            if live
                .grid
                .as_ref()
                .is_some_and(|g| g.read(cx).id() != view.data.id)
            {
                live.grid_events.clear();
                live.grid = None;
            }
            if let Some(g) = &live.grid {
                g.update(cx, |g, cx| g.sync(view, active, cx));
            } else if active {
                let grid = cx
                    .new(|cx| results::Grid::new(self.session.clone(), owner.clone(), view, w, cx));
                if owner.1 == Side::Main {
                    live.grid_events = Self::subscribe_edits(&grid, owner.clone(), w, cx);
                }
                live.grid = Some(grid);
            }
        }
        drop(docs);
        for (owner, id, text) in restart {
            self.search_rows(&owner, id, text, cx);
        }
    }

    pub(super) fn retry_result(&self, owner: &Owner, id: uuid::Uuid) {
        let docs = self.session.documents.borrow();
        if let Some(view) = docs
            .active()
            .filter(|tab| tab.id == owner.0)
            .and_then(|tab| tab.pane(owner.1).view())
            .filter(|view| view.data.id == id && view.allows(ResultAction::Retry))
        {
            view.loader.retry();
        }
    }

    pub(super) fn watch_result(
        owner: Owner,
        id: uuid::Uuid,
        mut changes: tokio::sync::watch::Receiver<u64>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |view, cx| {
            while changes.changed().await.is_ok() {
                let current = view.update(cx, |v, cx| {
                    let mut docs = v.session.documents.borrow_mut();
                    let current_view = docs
                        .tabs
                        .iter_mut()
                        .find(|t| t.id == owner.0)
                        .and_then(|t| t.pane_mut(owner.1).view_mut())
                        .filter(|view| view.data.id == id);
                    let Some(current_view) = current_view else {
                        return false;
                    };
                    current_view.sync_target();
                    docs.sync_views();
                    if docs.active().is_some_and(|t| t.id == owner.0) {
                        cx.notify();
                    }
                    true
                });
                if !matches!(current, Ok(true)) {
                    break;
                }
            }
        })
    }

    pub(super) fn rendered_result_grid(
        &self,
        tab: &Tab,
        side: Side,
        cx: &App,
    ) -> Option<&Entity<results::Grid>> {
        let pane = tab.pane(side);
        let view = pane.view()?;
        if pane.is_busy()
            || view.choice() != ViewChoice::Grid
            || view
                .filtered_rows
                .as_ref()
                .is_some_and(|rows| rows.is_empty())
            || (view.data.is_rowset && view.data.total_rows == 0)
        {
            return None;
        }
        self.editors
            .get(&(tab.id.clone(), side))?
            .grid
            .as_ref()
            .filter(|grid| grid.read(cx).id() == view.data.id)
    }

    pub(super) fn result_view(
        &self,
        tab: &Tab,
        side: Side,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let pane = tab.pane(side);
        let mut panel = div()
            .id(if side == Side::Main {
                "main-query-result"
            } else {
                "right-query-result"
            })
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .text_sm();
        if let Some(message) = self
            .editors
            .get(&(tab.id.clone(), side))
            .and_then(|e| e.controls.message.clone())
        {
            panel = panel.child(div().text_color(cx.theme().danger).child(message));
        }
        if let Some(notice) = pane.commit_notice() {
            panel = panel.child(
                div()
                    .text_color(cx.theme().success)
                    .child(notice.to_owned()),
            );
        }
        if let Some(error) = pane.view().and_then(|v| v.edit_error.clone()) {
            panel = panel.child(div().text_color(cx.theme().danger).child(error));
        }
        let elapsed = pane.elapsed().unwrap_or_default().as_secs_f64();
        let owner = (tab.id.clone(), side);
        let view = pane.view();
        let outcome = pane.outcome();
        let main = side == Side::Main;
        let plan = outcome.is_some_and(|o| matches!(o.status, Status::Plan(_)));
        let status = match outcome.map(|outcome| &outcome.status) {
            Some(Status::Cancelled) => Some(("Cancelled", IconName::Square)),
            Some(
                Status::ServerCancelled(_)
                | Status::TimedOut(_)
                | Status::Error(_)
                | Status::TrustRequired(_),
            ) => Some(("Failed", IconName::CircleX)),
            None if pane.was_cancelled() => Some(("Cancelled", IconName::Square)),
            _ => None,
        };
        if main {
            let toolbar = match view {
                Some(view) if !pane.is_busy() => {
                    self.inspection_toolbar(owner.clone(), view, plan, cx)
                }
                _ => {
                    let mut left = div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .child(self.empty_result_segments(tab, cx));
                    if let Some((label, icon)) =
                        status.filter(|_| pane.choice() != ViewChoice::History)
                    {
                        left = left.child(results::toolbar::status_line(
                            label.into(),
                            elapsed * 1000.,
                            false,
                            icon,
                            cx,
                        ));
                    }
                    if pane.is_busy() {
                        let owner = owner.clone();
                        let generation = pane.generation();
                        left = left.child(results::toolbar::result_stop(cx).on_click(cx.listener(
                            move |v, _, _, cx| {
                                let is_current =
                                    v.session.documents.borrow().active().is_some_and(|t| {
                                        t.id == owner.0
                                            && t.main.is_busy()
                                            && t.main.generation() == generation
                                    });
                                if is_current {
                                    v.stop_query(&owner, cx);
                                }
                            },
                        )));
                    }
                    results::toolbar::result_bar(cx).child(left)
                }
            };
            panel = panel.child(toolbar);
        }
        if main && pane.choice() == ViewChoice::History {
            return panel.p_0().overflow_hidden().flex().flex_col().children(
                self.editors
                    .get(&(tab.id.clone(), side))
                    .and_then(|e| e.history.clone()),
            );
        }
        if pane.is_busy() {
            return panel.child(
                div()
                    .id(if main {
                        "running-result"
                    } else {
                        "running-right-result"
                    })
                    .test_support()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(cx.theme().muted_foreground)
                    .children(main.then(|| Icon::new(IconName::LoaderCircle).size(px(20.))))
                    .child(if main {
                        format!("Executing query… {elapsed:.1} s")
                    } else {
                        format!("Running… {elapsed:.1} s")
                    }),
            );
        }
        let rerun = self.button("Run again", cx, move |v, w, cx| {
            v.run_query(owner.clone(), execution::Action::Run, w, cx)
        });
        let Some(outcome) = outcome else {
            return if pane.was_cancelled() {
                panel
                    .children((!main).then(|| format!("Query cancelled · {elapsed:.1} s")))
                    .child(rerun)
            } else {
                panel.child(idle_result(main, cx))
            };
        };
        let ms = outcome.elapsed.as_secs_f64() * 1000.;
        if let Status::Plan(output) = &outcome.status
            && pane.choice() == ViewChoice::Explain
        {
            return panel
                .p_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(self.explain_panel(
                    (tab.id.clone(), side),
                    pane.generation(),
                    &output.plan,
                    cx,
                ));
        }
        if let Some(view) = view {
            let rows = &view.data;
            if side == Side::Right {
                let theme = cx.theme();
                panel = panel.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .border_b_1()
                        .border_color(theme.border.opacity(0.3))
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(theme.muted_foreground)
                        .child(format!("{} rows", rows.total_rows))
                        .children(
                            rows.capped
                                .then(|| div().text_color(theme.warning).child("(truncated)")),
                        )
                        .children(
                            (rows.elapsed_ms > 0.).then(|| format!("· {:.1}ms", rows.elapsed_ms)),
                        ),
                );
            }
            let page_error = if view.choice() == ViewChoice::Map {
                rows.cache.error(0)
            } else {
                rows.cache.first_error()
            };
            if let Some(error) = page_error {
                panel = panel.child(div().text_color(cx.theme().danger).child(error.to_string()));
                if view.allows(ResultAction::Retry) {
                    let owner = (tab.id.clone(), side);
                    let id = rows.id;
                    panel = panel.child(self.button("Retry result pages", cx, move |v, _, cx| {
                        v.retry_result(&owner, id);
                        cx.notify();
                    }));
                }
            }
            panel = panel.overflow_hidden().flex().flex_col();
            return match view.choice() {
                ViewChoice::Map if side == Side::Main && view.field_detail.is_some() => {
                    let note = self
                        .editors
                        .get(&(tab.id.clone(), side))
                        .and_then(|e| e.map.as_ref())
                        .and_then(|map| map.read(cx).first_page_note(cx));
                    panel.children(note).children(self.field_detail(view, cx))
                }
                ViewChoice::Map if side == Side::Main => panel.children(
                    self.editors
                        .get(&(tab.id.clone(), side))
                        .and_then(|e| e.map.clone()),
                ),
                ViewChoice::Record => panel.child(self.record_panel(view, cx)),
                ViewChoice::Diff => panel.child(self.diff_panel(view, cx)),
                _ if view.filtered_rows.as_ref().is_some_and(|r| r.is_empty()) => {
                    panel.child(Label::new("No matching rows"))
                }
                _ if rows.is_rowset && rows.total_rows == 0 => {
                    panel.child(Label::new("No rows returned"))
                }
                _ => panel.children(self.rendered_result_grid(tab, side, cx).cloned()),
            };
        }
        match &outcome.status {
            Status::Rows(_) => panel,
            Status::Plan(_) => panel.child(idle_result(main, cx)),
            Status::Cancelled => panel
                .children((!main).then(|| {
                    div().px_4().py_2().child(results::toolbar::status_line(
                        "Cancelled".into(),
                        ms,
                        false,
                        IconName::Square,
                        cx,
                    ))
                }))
                .child(rerun),
            Status::ServerCancelled(e) | Status::TimedOut(e) | Status::Error(e) => panel
                .child(
                    div()
                        .px_4()
                        .py_2()
                        .children((!main).then(|| {
                            results::toolbar::status_line(
                                "Failed".into(),
                                ms,
                                false,
                                IconName::CircleX,
                                cx,
                            )
                        }))
                        .child(e.to_string()),
                )
                .child(rerun),
            Status::TrustRequired(c) => panel
                .child(format!(
                    "Verify the SSH key for {}:{} in Connection settings. · {ms:.1} ms",
                    c.host, c.port
                ))
                .child(rerun),
        }
    }
}

fn idle_result(main: bool, cx: &App) -> Div {
    let theme = cx.theme();
    let panel = div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .text_size(px(14.));
    if main {
        panel
            .gap_2()
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Run a query to see results"),
            )
            .child(
                Label::new("Choose a connection, write SQL, then select Execute.")
                    .text_xs()
                    .text_color(theme.muted_foreground),
            )
    } else {
        panel
            .text_color(theme.muted_foreground.opacity(0.4))
            .child("Run a query to see results")
    }
}
