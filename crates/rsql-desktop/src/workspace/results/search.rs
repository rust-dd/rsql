use super::*;
use gpui_kit::component::input::{InputEvent, InputState, Rope};
use rsql_desktop::results::filter::matching_rows;
use tokio::sync::watch;
pub(in crate::workspace) struct SearchInput {
    pub id: Uuid,
    pub(super) state: Entity<InputState>,
    _events: Subscription,
}

impl SearchInput {
    pub(in crate::workspace) fn text(&self, cx: &App) -> Rope {
        self.state.read(cx).text().clone()
    }
    pub(in crate::workspace) fn new(
        owner: Owner,
        id: Uuid,
        w: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let state = cx.new(|cx| InputState::new(w, cx).placeholder("Search results…"));
        let events = cx.subscribe(&state, move |v, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                v.search_rows(&owner, id, input.read(cx).text().clone(), cx);
            }
        });
        Self {
            id,
            state,
            _events: events,
        }
    }
}

impl Workspace {
    pub(super) fn clear_search(
        &mut self,
        owner: &Owner,
        id: Uuid,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(input) = self
            .editors
            .get(owner)
            .and_then(|l| l.search.as_ref())
            .filter(|s| s.id == id)
        {
            input.state.update(cx, |s, cx| s.set_value("", w, cx));
            self.search_rows(owner, id, Rope::default(), cx);
        }
    }

    pub(in crate::workspace) fn search_rows(
        &mut self,
        owner: &Owner,
        id: Uuid,
        term: Rope,
        cx: &mut Context<Self>,
    ) {
        if *self.session.preflight_stop.borrow() {
            return;
        }
        let mut docs = self.session.documents.borrow_mut();
        let Some(view) = docs
            .tabs
            .iter_mut()
            .find(|t| t.id == owner.0)
            .and_then(|t| t.main.view_mut())
            .filter(|v| v.data.id == id && v.allows(ResultAction::Search))
        else {
            return;
        };
        view.search_revision += 1;
        let revision = view.search_revision;
        view.search_owner = None;
        view.restart_search = false;
        if term.len() <= 256 && term.chunks().all(rsql_desktop::sql::is_blank) {
            view.install_filter(String::new(), None);
            cx.notify();
            return;
        }
        let Some(rows) = view.snapshot(ResultAction::Search) else {
            return;
        };
        let route = view.route.clone();
        let (sender, mut cancel) = watch::channel(());
        view.search_owner = Some(sender);
        let mut stopped = self.session.preflight_stop.subscribe();
        let worker = self.session.runtime.spawn(async move {
            let check_stop = stopped.clone();
            let check_cancel = cancel.clone();
            let check_route = route.clone();
            let classified = tokio::task::spawn_blocking(move || {
                let current = || {
                    !*check_stop.borrow()
                        && check_cancel.has_changed().is_ok()
                        && check_route.is_current()
                };
                for chunk in term.chunks() {
                    current().then_some(())?;
                    if !rsql_desktop::sql::is_blank(chunk) {
                        return Some(Some(term));
                    }
                }
                Some(None)
            })
            .await
            .ok()??;
            let Some(term) = classified else {
                return Some((String::new(), None));
            };
            tokio::select! {
                _=cancel.changed()=>return None,
                _=stopped.wait_for(|s|*s)=>return None,
                _=tokio::time::sleep(Duration::from_millis(200))=>{}
            }
            tokio::task::spawn_blocking(move || {
                let current =
                    || !*stopped.borrow() && cancel.has_changed().is_ok() && route.is_current();
                current().then_some(())?;
                let mut text = String::with_capacity(term.len());
                for chunk in term.chunks() {
                    current().then_some(())?;
                    text.push_str(chunk);
                }
                let indices = matching_rows(&rows.page, &text, current)?;
                Some((text, Some(indices)))
            })
            .await
            .ok()
            .flatten()
        });
        let owner = owner.clone();
        cx.spawn(async move |v, cx| {
            let Ok(Some((term, rows))) = worker.await else {
                return;
            };
            let _ = v.update(cx, |v, cx| {
                let mut docs = v.session.documents.borrow_mut();
                if let Some(view) = docs
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == owner.0)
                    .and_then(|t| t.main.view_mut())
                    .filter(|v| {
                        v.data.id == id
                            && v.search_revision == revision
                            && v.allows(ResultAction::Search)
                    })
                {
                    view.install_filter(term, rows);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}
