use super::*;
use crate::runtime::history::millis;
use rsql_core::storage::history::NewHistoryEntry;
use rsql_desktop::{
    connection::ConnectionProfile,
    document::Request,
    execution::{Mode, Outcome, RunSpec, Status, TIMEOUTS_MS},
    sql::{TargetMode, execution_target},
};
use tokio::sync::{oneshot, watch};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Run,
    Script,
    Explain,
    Analyze,
}

impl Action {
    fn mode(self) -> (Mode, TargetMode) {
        match self {
            Self::Run => (Mode::Query, TargetMode::Run),
            Self::Script => (Mode::Query, TargetMode::Script),
            Self::Explain => (Mode::Explain, TargetMode::Explain),
            Self::Analyze => (Mode::Analyze, TargetMode::Explain),
        }
    }
}

pub(super) struct Preflight {
    pub selection: std::ops::Range<usize>,
    pub caret: usize,
    _cancel: watch::Sender<()>,
}

#[derive(Default)]
pub(super) struct Controls {
    sequence: u64,
    pub preflight: Option<Preflight>,
    pub format_sequence: u64,
    pub format: Option<oneshot::Sender<()>>,
    pub message: Option<String>,
}

impl Workspace {
    pub(super) fn keyboard_owner(&self, w: &mut Window, cx: &mut App) -> Option<Owner> {
        let owner = self.focused_owner(w, cx)?;
        let live = &self.editors[&owner];
        let composing = live
            .state
            .update(cx, |s, cx| s.marked_text_range(w, cx).is_some());
        (!composing && self.current_language_menu(live, w, cx).is_none()).then_some(owner)
    }
    pub(super) fn keyboard_run(&mut self, action: Action, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(owner) = self.keyboard_owner(w, cx)
            && (action != Action::Explain || owner.1 == Side::Main)
        {
            self.run_query(owner, action, w, cx);
        }
    }
    pub(super) fn cancel_tab_query(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        let side = self
            .session
            .documents
            .borrow()
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| {
                [Side::Main, Side::Right]
                    .into_iter()
                    .find(|side| t.pane(*side).is_busy())
            });
        if let Some(side) = side {
            self.stop_query(&(tab_id.to_owned(), side), cx);
        }
    }
    pub(super) fn stop_query(&mut self, owner: &Owner, cx: &mut Context<Self>) {
        if let Some(live) = self.editors.get_mut(owner) {
            live.controls.preflight.take();
            live.controls.sequence += 1;
            live.controls.message = None;
        }
        if let Some(tab) = self
            .session
            .documents
            .borrow_mut()
            .tabs
            .iter_mut()
            .find(|t| t.id == owner.0)
        {
            tab.cancel(owner.1);
        }
        cx.notify();
    }
    pub(super) fn profile_for(&self, id: &str) -> Option<ConnectionProfile> {
        let docs = self.session.documents.borrow();
        let project = docs.tabs.iter().find(|t| t.id == id)?.project.as_ref()?;
        self.session
            .connections
            .borrow()
            .profiles
            .iter()
            .find(|p| p.id == *project)
            .cloned()
    }

    pub(super) fn run_query(
        &mut self,
        owner: Owner,
        action: Action,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.analysis_for(&owner, w, cx);
        let profile = self.profile_for(&owner.0);
        let Some(live) = self.editors.get_mut(&owner) else {
            return;
        };
        live.controls.preflight.take();
        live.controls.sequence += 1;
        let sequence = live.controls.sequence;
        let Some(profile) = profile else {
            live.controls.message = Some("Choose a connection to run SQL.".into());
            cx.notify();
            return;
        };
        let state = live.state.read(cx);
        let text = state.text().clone();
        let selection = state.selected_range();
        let revision = live.language.revision;
        let entity = live.state.entity_id();
        let (mode, scope) = action.mode();
        let docs = self.session.documents.borrow();
        let Some(tab) = docs.tabs.iter().find(|t| t.id == owner.0) else {
            return;
        };
        let generation = tab.pane(owner.1).generation();
        let Ok(ticket) = self.session.services.capture_project_ticket(&profile.id) else {
            return;
        };
        let (cancel, cancelled) = watch::channel(());
        live.controls.preflight = Some(Preflight {
            selection: selection.clone(),
            caret: state.cursor(),
            _cancel: cancel,
        });
        let stopped = self.session.preflight_stop.subscribe();
        let scan_text = text.clone();
        let selected = selection.clone();
        let scan = self.session.runtime.spawn_blocking(move || {
            let check = || {
                if *stopped.borrow() || cancelled.has_changed().is_err() {
                    Err("Cancelled".to_owned())
                } else {
                    Ok(())
                }
            };
            check()?;
            let mut sql = String::with_capacity(scan_text.len());
            for chunk in scan_text.chunks() {
                check()?;
                sql.push_str(chunk);
            }
            execution_target(&sql, (selected.start, selected.end), scope, check)
                .map(|range| sql[range].to_owned())
        });
        cx.spawn(async move |view, cx| {
            let result = scan.await;
            let _ = view.update(cx, |v, cx| {
                let current_route = ticket.is_current()
                    && v.profile_for(&owner.0)
                        .is_some_and(|p| p.same_route(&profile));
                let Some(live) = v.editors.get_mut(&owner) else {
                    return;
                };
                let state = live.state.read(cx);
                if entity != live.state.entity_id() || sequence != live.controls.sequence {
                    return;
                }
                let Some(pending) = live.controls.preflight.take() else {
                    return;
                };
                if pending.caret != state.cursor()
                    || !current_route
                    || revision != live.language.revision
                    || text != *state.text()
                    || selection != state.selected_range()
                {
                    return;
                }
                let Ok(result) = result else { return };
                let mut docs = v.session.documents.borrow_mut();
                let Some(tab) = docs
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == owner.0 && t.pane(owner.1).generation() == generation)
                else {
                    return;
                };
                let sql = match result {
                    Ok(sql) => sql,
                    Err(e) => {
                        // The original returned silently for a blank editor and toasted other targets.
                        if !text.chars().all(char::is_whitespace) {
                            v.toasts.show(toast::Toast::error(&e), cx);
                        }
                        cx.notify();
                        return;
                    }
                };
                let timeout = tab
                    .timeout
                    .and_then(|ms| u32::try_from(ms).ok())
                    .filter(|ms| TIMEOUTS_MS.contains(&Some(*ms)));
                let generated = tab.pane(owner.1).view().and_then(|v| v.generated.clone());
                let Some((request, cancel)) = tab.begin_mode(owner.1, mode) else {
                    return;
                };
                live.controls.message = None;
                live.result_updates = None;
                drop(docs);
                let mut spec = RunSpec::new(profile, sql, mode, timeout).while_current(ticket);
                spec.generated = generated;
                spec.row_tools = owner.1 == Side::Main;
                v.dispatch_query(owner, request, spec, cancel, cx);
            });
        })
        .detach();
    }

    pub(super) fn dispatch_query(
        &mut self,
        owner: Owner,
        request: Request,
        spec: RunSpec,
        cancel: oneshot::Receiver<()>,
        cx: &mut Context<Self>,
    ) {
        self.tick_elapsed(cx);
        let profile = spec.profile.clone();
        let (notify, connected) = oneshot::channel();
        let run = self.session.runner.start(spec.on_connected(notify), cancel);
        cx.spawn(async move |view, cx| {
            if let Ok(ticket) = connected.await {
                let _ = view.update(cx, |v, cx| {
                    if v.session
                        .connections
                        .borrow()
                        .profiles
                        .iter()
                        .any(|p| p.same_route(&profile))
                    {
                        v.publish_connection(&profile.id, &ticket, cx);
                    }
                });
            }
            let result = run.await;
            let _ = view.update(cx, |v, cx| match result {
                Ok(outcome) => v.publish_query(&owner, request, outcome, &profile, cx),
                Err(error) => {
                    if v.session.documents.borrow_mut().retire(request)
                        && let Some(live) = v.editors.get_mut(&owner)
                    {
                        live.controls.message = Some(format!("Query worker stopped: {error}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn tick_elapsed(&mut self, cx: &mut Context<Self>) {
        if self.elapsed_timer.is_some() {
            return;
        }
        self.elapsed_timer = Some(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let running = view.update(cx, |v, cx| {
                    let docs = v.session.documents.borrow();
                    let running = docs
                        .tabs
                        .iter()
                        .any(|t| t.main.is_busy() || t.right.is_busy());
                    if docs
                        .active()
                        .is_some_and(|t| t.main.is_busy() || t.right.is_busy())
                    {
                        cx.notify();
                    }
                    if !running {
                        v.elapsed_timer = None;
                    }
                    running
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        }));
    }

    pub(super) fn publish_query(
        &mut self,
        owner: &Owner,
        request: Request,
        outcome: Outcome,
        profile: &ConnectionProfile,
        cx: &mut Context<Self>,
    ) {
        if request.owner() != (&owner.0, owner.1) {
            return;
        }
        let current = outcome
            .route_ticket
            .as_ref()
            .is_some_and(|t| t.is_current())
            && self
                .session
                .connections
                .borrow()
                .profiles
                .iter()
                .any(|p| p.same_route(profile));
        if current && outcome.may_change_catalog {
            self.refresh_catalog(&profile.id, cx);
        }
        let entry = {
            let mut docs = self.session.documents.borrow_mut();
            if docs.finish(request, outcome) {
                let pane = docs
                    .tabs
                    .iter()
                    .find(|t| t.id == owner.0)
                    .map(|tab| tab.pane(owner.1));
                if let Some(live) = self.editors.get_mut(owner) {
                    live.expansion = Default::default();
                    live.result_updates = pane
                        .and_then(|pane| pane.result())
                        .filter(|result| result.packed_source().is_some())
                        .map(|result| {
                            Self::watch_result(
                                owner.clone(),
                                result.id,
                                result.cache.subscribe(),
                                cx,
                            )
                        });
                }
                if current
                    && let Some(pane) = pane
                    && let Some(outcome) = pane.outcome()
                    && crate::desktop::notifications::eligible(outcome)
                {
                    self.notify_accepted(owner.clone(), pane.generation(), profile.clone(), cx);
                }
                pane.and_then(|p| p.outcome()).and_then(history_entry)
            } else {
                None
            }
        };
        if let Some(entry) = entry {
            self.session.accept_history(entry);
        }
        cx.notify();
    }

    fn notify_accepted(
        &self,
        owner: Owner,
        generation: u64,
        profile: ConnectionProfile,
        cx: &mut Context<Self>,
    ) {
        use crate::desktop::notifications;
        if !notifications::is_background(cx) {
            return;
        }
        let permission = notifications::permission(cx);
        cx.spawn(async move |view, cx| {
            let permission = permission.await;
            let _ = view.update(cx, |v, cx| {
                if *v.session.preflight_stop.borrow()
                    || !v
                        .profile_for(&owner.0)
                        .is_some_and(|p| p.same_route(&profile))
                {
                    return;
                }
                let docs = v.session.documents.borrow();
                let Some(pane) = docs
                    .tabs
                    .iter()
                    .find(|t| t.id == owner.0)
                    .map(|t| t.pane(owner.1))
                else {
                    return;
                };
                if pane.generation() == generation
                    && let Some(outcome) = pane.outcome()
                    && let Some(notice) = notifications::notification_for(
                        outcome,
                        !notifications::is_background(cx),
                        permission,
                    )
                {
                    notifications::deliver(notice, cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn publish_connection(
        &mut self,
        id: &str,
        ticket: &rsql_core::services::ProjectTicket,
        cx: &mut Context<Self>,
    ) {
        if !ticket.is_current() {
            return;
        }
        let was_connected = self
            .navigation
            .connected_routes
            .insert(id.into(), ticket.clone())
            .is_some_and(|old| old.is_current());
        self.navigation.status.insert(id.into(), "Connected".into());
        if !was_connected {
            self.refresh_catalog(id, cx);
        }
        cx.notify();
    }
}

fn history_entry(outcome: &Outcome) -> Option<NewHistoryEntry> {
    let (duration_ms, row_count, error) = match &outcome.status {
        Status::Rows(r) => (f64::from(r.elapsed_ms), r.total_rows as i64, None),
        Status::Error(e) | Status::ServerCancelled(e) | Status::TimedOut(e) => {
            (outcome.elapsed.as_secs_f64() * 1000., 0, Some(e.clone()))
        }
        Status::TrustRequired(c) => (
            outcome.elapsed.as_secs_f64() * 1000.,
            0,
            Some(format!(
                "SSH host key verification required for {}:{}",
                c.host, c.port
            )),
        ),
        Status::Plan(_) | Status::Cancelled => return None,
    };
    Some(NewHistoryEntry {
        project_id: outcome.project_id.clone(),
        database: outcome.database.clone(),
        sql: outcome.sql.to_string(),
        executed_at: millis(outcome.started),
        duration_ms,
        row_count,
        success: error.is_none(),
        error,
    })
}
