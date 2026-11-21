use super::*;
use rsql_desktop::execution::{Mode, RunSpec};
use tokio::sync::watch;

impl Workspace {
    pub(super) fn navigate_foreign_key(
        &mut self,
        owner: &Owner,
        id: Uuid,
        pos: Position,
        generation: u64,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if owner.1 != Side::Main {
            return;
        }
        let ticket = Uuid::new_v4();
        let (sender, cancel) = watch::channel(());
        let mut docs = self.session.documents.borrow_mut();
        let Some(index) = docs.selected else { return };
        let tab = &mut docs.tabs[index];
        if tab.id != owner.0 {
            return;
        }
        let Some(view) = tab.main.view_mut().filter(|v| {
            v.data.id == id
                && v.loader.generation() == generation
                && v.choice() == rsql_desktop::results::ViewChoice::Grid
                && v.edit.is_none()
        }) else {
            return;
        };
        let Some(target) = view
            .data
            .cache
            .foreign_target(pos.row, pos.col)
            .filter(|t| t.route.same_generation(&view.route))
        else {
            return;
        };
        view.navigation_owner = Some((ticket, sender));
        drop(docs);
        let stopped = self.session.preflight_stop.subscribe();
        let route = target.route.clone();
        let original = target.clone();
        let make_sql = self.session.runtime.spawn_blocking(move || {
            let query = target.navigate(pos.row, pos.col, || {
                !*stopped.borrow() && cancel.has_changed().is_ok() && target.route.is_current()
            })?;
            let editor_text = query.sql().to_string();
            Some((editor_text, Arc::new(query)))
        });
        let owner = owner.clone();
        cx.spawn_in(w, async move |this, cx| {
            let Ok(Some((sql, generated))) = make_sql.await else {
                return;
            };
            let _ = this.update_in(cx, |v, w, cx| {
                let Some(profile) = v.profile_for(&owner.0) else {
                    return;
                };
                let docs = v.session.documents.borrow();
                let valid = route.is_current()
                    && docs
                        .active()
                        .filter(|t| t.id == owner.0 && t.project.as_deref() == Some(&profile.id))
                        .and_then(|t| t.main.view())
                        .is_some_and(|view| {
                            view.data.id == id
                                && view.loader.generation() == generation
                                && view.choice() == rsql_desktop::results::ViewChoice::Grid
                                && view.route.same_generation(&route)
                                && view
                                    .data
                                    .cache
                                    .foreign_target(pos.row, pos.col)
                                    .is_some_and(|t| Arc::ptr_eq(&t, &original))
                                && view.edit.is_none()
                                && view
                                    .navigation_owner
                                    .as_ref()
                                    .is_some_and(|(t, _)| *t == ticket)
                        });
                drop(docs);
                if !valid {
                    return;
                }
                v.update_documents(w, cx, |docs| {
                    docs.open(Kind::Query, Some(profile.id.clone()))
                        .main
                        .draft
                        .text = sql;
                });
                let mut docs = v.session.documents.borrow_mut();
                let index = docs.selected.expect("opened tab");
                let tab = &mut docs.tabs[index];
                let new_owner = (tab.id.clone(), Side::Main);
                let Some((request, cancel)) = tab.begin(Side::Main) else {
                    return;
                };
                drop(docs);
                let mut spec = RunSpec::new(profile, generated.sql().clone(), Mode::Query, None)
                    .while_current(route);
                spec.generated = Some(generated);
                v.dispatch_query(new_owner, request, spec, cancel, cx);
            });
        })
        .detach();
    }
}
