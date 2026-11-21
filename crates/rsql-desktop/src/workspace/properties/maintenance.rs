use super::*;
use gpui_kit::component::input::{InputEvent, InputState};
use rsql_core::services::{
    WriteOutcome,
    object_action::{ActionOutcome, ActionReview, ObjectAction},
};

#[derive(Clone, Copy)]
pub(super) enum ActionStatus {
    Success,
    Error,
    Warning,
    Indeterminate,
}

#[derive(Default)]
pub(super) struct Confirmation {
    pub selection: Option<ObjectAction>,
    pub input: Option<Entity<InputState>>,
    pub review: Option<ActionReview>,
    request: Option<(Uuid, oneshot::Sender<()>)>,
    _events: Option<Subscription>,
    pub retired: bool,
    pub effects: Option<&'static str>,
    pub notice: Option<(ActionStatus, String)>,
    pub running: Option<(Uuid, oneshot::Sender<()>)>,
}
impl Workspace {
    pub(super) fn choose_action(
        &mut self,
        action: ObjectAction,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.properties.as_mut().filter(|p| !p.preserve()) else {
            return;
        };
        if !ObjectAction::available(p.object.kind).contains(&action) {
            return;
        }
        let id = p.id;
        let object = p.object.clone();
        let input = cx.new(|cx| InputState::new(w, cx).placeholder(object.name.clone()));
        let events = cx.subscribe(&input, |_, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                cx.notify()
            }
        });
        let (cancel, rx) = oneshot::channel();
        let revision = Uuid::new_v4();
        p.action = Confirmation {
            selection: Some(action),
            input: Some(input.clone()),
            review: None,
            request: Some((revision, cancel)),
            _events: Some(events),
            retired: false,
            effects: None,
            notice: None,
            running: None,
        };
        p.status.clear();
        input.focus_handle(cx).focus(w, cx);
        let svc = self.session.services.clone();
        let route = p.capture.route.clone();
        let mut generation = p.capture.generation.clone();
        let mut stop = self.session.preflight_stop.subscribe();
        let job = self.session.runtime.spawn(async move {
            let closed = async {
                tokio::select! {
                    _ = rx => (),
                    _ = generation.changed() => (),
                    _ = stop.wait_for(|s|*s) => (),
                }
            };
            svc.review_object_action(&route, &object, action, closed)
                .await
        });
        cx.spawn(async move |v, cx| {
            let result = job
                .await
                .map_err(diagnostic::bounded_message)
                .and_then(|r| r.map_err(diagnostic::bounded_message));
            let _ = v.update(cx, |v, cx| {
                if !v.property_current(id) {
                    return;
                }
                let p = v.properties.as_mut().unwrap();
                if !p
                    .action
                    .request
                    .as_ref()
                    .is_some_and(|(r, _)| *r == revision)
                {
                    return;
                }
                p.action.request = None;
                match result {
                    Ok(review) => p.action.review = Some(review),
                    Err(err) => {
                        p.status.clear();
                        p.action.notice = Some((ActionStatus::Error, err));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn confirm_action(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.properties.as_ref() else {
            return;
        };
        if p.busy
            || !p.capture.current(self)
            || p.action
                .input
                .as_ref()
                .is_none_or(|i| i.read(cx).value().as_ref() != p.object.name)
        {
            return;
        }
        let p = self.properties.as_mut().unwrap();
        let Some(review) = p.action.review.take() else {
            return;
        };
        let action = p.action.selection.unwrap();
        let id = p.id;
        let route = p.capture.route.clone();
        let project = p.object.project.clone();
        let mut modal = p.modal.subscribe();
        let execution = Uuid::new_v4();
        let (cancel, cancelled) = oneshot::channel();
        let mut generation = p.capture.generation.clone();
        let mut stop = self.session.preflight_stop.subscribe();
        let svc = self.session.services.clone();
        let job = self.session.runtime.spawn(async move {
            let closed = async {
                tokio::select! {
                    _ = cancelled => (),
                    _ = modal.changed() => (),
                    _ = generation.changed() => (),
                    _ = stop.wait_for(|s| *s) => (),
                }
            };
            svc.run_object_action(&execution.to_string(), review, closed)
                .await
        });
        p.action = Default::default();
        p.action.running = Some((execution, cancel));
        p.busy = true;
        p.transfer = None;
        let label = action.label(p.object.kind);
        let top_level = action.requires_top_level(p.object.kind);
        p.status = format!("Running {label}…");
        cx.spawn_in(w, async move |view, cx| {
            let outcome = job.await.unwrap_or_else(|e| ActionOutcome {
                result: WriteOutcome::Indeterminate(rsql_core::Error::QueryFailed(e.to_string())),
                possible_effects: top_level,
                server_warnings: None,
                verification_warning: None,
            });
            let _ = view.update(cx, |v, cx| {
                let current = route.is_current()
                    && v.navigation
                        .connected_routes
                        .get(&project)
                        .is_some_and(|r| r.same_generation(&route));
                let refresh = current && outcome.should_refresh();
                if refresh {
                    v.refresh_catalog(&project, cx);
                }
                let capture = refresh.then(|| Capture::new(v, &project)).flatten();
                let Some(p) = v.properties.as_mut().filter(|p| p.id == id) else {
                    return;
                };
                if let Some(c) = capture {
                    p.capture = c;
                    p.sections.clear();
                    p.editor = None;
                }
                p.finish_action(action, current, outcome);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Properties {
    pub(super) fn finish_action(
        &mut self,
        action: ObjectAction,
        current: bool,
        outcome: ActionOutcome,
    ) {
        let label = action.label(self.object.kind);
        self.busy = false;
        self.action.running = None;
        self.action.retired = !current || outcome.possible_effects;
        self.action.effects = outcome.possible_effects.then(|| {
            if self.object.kind == rsql_desktop::metadata::ObjectKind::PartitionedTable {
                "Earlier partitions may already have changed."
            } else {
                "Maintenance may already have changed the table."
            }
        });
        let warning = outcome.server_warnings.is_some()
            || outcome.verification_warning.is_some()
            || outcome.possible_effects;
        let (status, mut message) = match outcome.result {
            WriteOutcome::Failed(error) => (
                if warning {
                    ActionStatus::Warning
                } else {
                    ActionStatus::Error
                },
                diagnostic::bounded_message(error),
            ),
            WriteOutcome::Indeterminate(error) => {
                self.action.retired = true;
                (
                    ActionStatus::Indeterminate,
                    format!(
                        "{label} completion is uncertain. Check the database before reopening properties. {}",
                        diagnostic::bounded_message(error)
                    ),
                )
            }
            WriteOutcome::Committed { cleanup_error, .. } => {
                self.action.retired |= action.drops()
                    || outcome.server_warnings.is_some()
                    || outcome.verification_warning.is_some()
                    || cleanup_error.is_some();
                let mut summary = if outcome.server_warnings.is_some() {
                    format!(
                        "PostgreSQL completed {label} with warnings. Some work may have been skipped."
                    )
                } else {
                    format!("PostgreSQL completed {label}.")
                };
                if let Some(error) = outcome.verification_warning {
                    summary.push_str(&format!(" {}", diagnostic::bounded_message(error)));
                }
                let warning = warning || cleanup_error.is_some();
                if let Some(error) = cleanup_error {
                    summary.push_str(&format!(
                        " Session cleanup: {}",
                        diagnostic::bounded_message(error)
                    ));
                }
                (
                    if warning {
                        ActionStatus::Warning
                    } else {
                        ActionStatus::Success
                    },
                    summary,
                )
            }
        };
        self.status.clear();
        if let Some(warnings) = outcome.server_warnings {
            message.push_str(&format!(" {warnings}"));
        }
        self.action.notice = Some((status, message));
    }
}
