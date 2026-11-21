use super::*;
use gpui_kit::component::input::{InputEditsEvent, Rope, RopeExt};
use rsql_desktop::completion::{Analysis, worker::AnalysisRequest};
use std::{cell::Cell, sync::Arc};
use tokio::sync::watch;

use super::{editor_overlay::Menu, editor_provider::Provider, editor_snippet::Snippet};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Stamp {
    pub owner: Owner,
    pub entity: EntityId,
    pub revision: u64,
    pub caret: usize,
    selection: std::ops::Range<usize>,
    pub project: Option<String>,
    pub generation: u64,
    pub epoch: u64,
    sequence: u64,
}

pub(super) type Response = watch::Receiver<Option<Result<Arc<Analysis>, ()>>>;

struct Flight {
    stamp: Stamp,
    response: Response,
    _task: Task<()>,
}

pub(super) struct Services {
    pub presentation: Rc<super::editor_presentation::Presentation>,
    pub epoch: Rc<Cell<u64>>,
    pub revision: u64,
    text: Rope,
    pub(super) snapshot: Arc<str>,
    sequence: u64,
    flight: Option<Flight>,
    pub menu: Option<Menu>,
    pub snippet: Option<Snippet>,
    _subscriptions: Vec<Subscription>,
}

impl LiveEditor {
    pub(super) fn synchronize_snippet(&mut self, cx: &mut App) {
        if let Some(snippet) = &mut self.language.snippet
            && !self
                .state
                .update(cx, |state, cx| snippet.synchronize_selection(state, cx))
        {
            self.language.snippet = None;
        }
    }

    pub(super) fn invalidate_language(&mut self, cx: &mut App) {
        self.language.epoch.set(self.language.epoch.get() + 1);
        self.language.presentation.dismiss();
        self.language.flight = None;
        self.language.menu = None;
        let had_snippet = self.language.snippet.take().is_some();
        self.state.update(cx, |s, cx| {
            s.clear_hover_state(cx);
            if had_snippet {
                s.set_selected_range(s.selected_range(), cx);
            }
        });
    }
}

impl Workspace {
    pub(super) fn attach_services(
        owner: Owner,
        editor: &Entity<EditorState>,
        initial_snapshot: Option<Arc<str>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Services {
        let epoch = Rc::new(Cell::new(0));
        let presentation = Rc::new(super::editor_presentation::Presentation::default());
        let provider = Rc::new(Provider::new(
            cx.weak_entity(),
            owner.clone(),
            editor.entity_id(),
            epoch.clone(),
            presentation.clone(),
        ));
        editor.update(cx, |state, _| {
            state.lsp_mut().completion_provider = Some(provider.clone());
            state.lsp_mut().definition_provider = Some(provider);
        });
        let edits_owner = owner.clone();
        let edits = cx.subscribe(editor, move |view, _, event: &InputEditsEvent, cx| {
            if let Some(live) = view.editors.get_mut(&edits_owner)
                && let Some(snippet) = &mut live.language.snippet
            {
                if snippet.edited(event, live.state.read(cx).text()) {
                    // A selection notification can precede its edit event in the same input turn.
                    live.synchronize_snippet(cx);
                } else {
                    live.language.snippet = None;
                }
            }
        });
        let handle = editor.read(cx).focus_handle(cx);
        let blur_owner = owner.clone();
        let blur = cx.on_blur(&handle, window, move |view, _, cx| {
            if let Some(live) = view.editors.get_mut(&blur_owner) {
                live.invalidate_language(cx);
            }
            cx.notify();
        });
        let focus = cx.on_focus(&handle, window, move |view, w, cx| {
            let mut docs = view.session.documents.borrow_mut();
            if let Some(tab) = docs.selected.map(|i| &mut docs.tabs[i])
                && tab.id == owner.0
                && (owner.1 == Side::Main || tab.split)
            {
                tab.focused_side = owner.1;
            }
            drop(docs);
            view.analysis_for(&owner, w, cx);
        });
        Services {
            presentation,
            epoch,
            text: editor.read(cx).text().clone(),
            snapshot: initial_snapshot.unwrap_or_else(|| editor.read(cx).text().to_string().into()),
            revision: 0,
            sequence: 0,
            flight: None,
            menu: None,
            snippet: None,
            _subscriptions: vec![edits, blur, focus],
        }
    }

    pub(super) fn invalidate_project_language(&mut self, id: &str, cx: &mut App) {
        let docs = self.session.documents.borrow();
        for ((tab, _), live) in &mut self.editors {
            if docs
                .tabs
                .iter()
                .any(|t| t.id == *tab && t.project.as_deref() == Some(id))
            {
                live.invalidate_language(cx);
            }
        }
    }

    pub(super) fn stamp(&self, owner: &Owner, w: &Window, cx: &App) -> Option<Stamp> {
        let docs = self.session.documents.borrow();
        let tab = docs
            .active()
            .filter(|t| t.id == owner.0 && (owner.1 == Side::Main || t.split))?;
        let live = self.editors.get(owner)?;
        let state = live.state.read(cx);
        if self.palette.is_some()
            || self.navigation.modal_open
            || !state.focus_handle(cx).is_focused(w)
            || !same_text(&live.language.text, state.text())
        {
            return None;
        }
        let generation = tab
            .project
            .as_ref()
            .and_then(|id| self.navigation.catalogs.get(id))
            .map_or(0, |c| c.epoch);
        Some(Stamp {
            owner: owner.clone(),
            entity: live.state.entity_id(),
            revision: live.language.revision,
            caret: state.cursor(),
            selection: state.selected_range(),
            project: tab.project.clone(),
            generation,
            sequence: live.language.sequence,
            epoch: live.language.epoch.get(),
        })
    }

    pub(super) fn is_current(&self, stamp: &Stamp, w: &Window, cx: &App) -> bool {
        self.stamp(&stamp.owner, w, cx).as_ref() == Some(stamp)
    }

    pub(super) fn analysis_for(
        &mut self,
        owner: &Owner,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(Stamp, Response)> {
        let live = self.editors.get_mut(owner)?;
        let text = live.state.read(cx).text();
        live.language
            .presentation
            .changed(text, live.state.read(cx).cursor());
        let state = live.state.read(cx);
        if live.controls.preflight.as_ref().is_some_and(|scan| {
            scan.selection != state.selected_range() || scan.caret != state.cursor()
        }) {
            live.controls.preflight.take();
        }
        if !same_text(&live.language.text, text) {
            live.controls.format.take();
            live.controls.preflight.take();
            live.language.revision += 1;
            live.language.text = text.clone();
            live.language.snapshot = text.to_string().into();
        } else if live
            .language
            .snippet
            .as_ref()
            .is_some_and(|s| !s.contains(live.state.read(cx).cursor()))
        {
            live.language.snippet = None;
        }
        let mut stamp = self.stamp(owner, w, cx)?;
        let live = self.editors.get_mut(owner)?;
        if let Some(flight) = &live.language.flight
            && flight.stamp == stamp
            && (flight.response.borrow().as_ref().is_some_and(Result::is_ok)
                || flight.response.has_changed().is_ok())
        {
            return Some((stamp, flight.response.clone()));
        }
        live.language.sequence += 1;
        stamp.sequence = live.language.sequence;
        live.language.menu = None;
        let text = live.language.text.clone();
        let ticket = self.session.analysis.submit(AnalysisRequest {
            owner: owner.clone(),
            caret: stamp.caret,
            revision: stamp.revision,
            project_generation: stamp.generation,
            text: live.language.snapshot.clone(),
        });
        let (sender, response) = watch::channel(None);
        let key = stamp.clone();
        let task = cx.spawn_in(w, async move |view, cx| {
            let Some(parsed) = ticket.receive().await else {
                return;
            };
            let _ = view.update_in(cx, |view, w, cx| {
                if !view.is_current(&key, w, cx) {
                    return;
                }
                let Ok(parsed) = parsed else {
                    sender.send_replace(Some(Err(())));
                    return;
                };
                view.editors[&key.owner].state.update(cx, |state, cx| {
                    use gpui_kit::base::input::{Diagnostic, DiagnosticSeverity};
                    if let Some(set) = state.diagnostics_mut() {
                        set.reset(&text);
                        set.extend(parsed.diagnostics.iter().map(|d| {
                            Diagnostic::new(
                                text.offset_to_position(d.range.start)
                                    ..text.offset_to_position(d.range.end),
                                d.message.clone(),
                            )
                            .with_severity(DiagnosticSeverity::Error)
                        }));
                        cx.notify();
                    }
                });
                sender.send_replace(Some(Ok(Arc::new(parsed))));
            });
        });
        live.language.flight = Some(Flight {
            stamp: stamp.clone(),
            response: response.clone(),
            _task: task,
        });
        cx.notify();
        Some((stamp, response))
    }
}

pub(super) fn same_text(previous: &Rope, current: &Rope) -> bool {
    ropey::extra::esoterica::ropes_are_instances(previous, current) || previous == current
}

pub(super) async fn resolve(response: Response) -> Option<Arc<Analysis>> {
    reply(response).await?.ok()
}

pub(super) async fn reply(mut response: Response) -> Option<Result<Arc<Analysis>, ()>> {
    response.wait_for(Option::is_some).await.ok()?.clone()
}

impl Drop for Services {
    fn drop(&mut self) {
        self.presentation.dismiss();
    }
}
