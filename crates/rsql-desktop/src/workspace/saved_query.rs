mod dialog;

use super::*;
use gpui_kit::component::input::InputState;
use rsql_desktop::{
    saved::{SavedQuery, SavedQueryKey},
    sql::is_blank,
};
use std::sync::Arc;

pub(super) struct SaveTarget {
    owner: Owner,
    entity: EntityId,
    profile: rsql_desktop::connection::ConnectionProfile,
    ticket: rsql_core::services::ProjectTicket,
    sql: String,
}

impl Workspace {
    pub(super) fn capture_saved_query(&self, owner: &Owner, cx: &App) -> Option<SaveTarget> {
        if self.saving_query || self.navigation.modal_open {
            return None;
        }
        let profile = self.profile_for(&owner.0)?;
        let live = self.editors.get(owner)?;
        let sql = live.state.read(cx).text().to_string();
        if is_blank(&sql) {
            return None;
        }
        Some(SaveTarget {
            owner: owner.clone(),
            entity: live.state.entity_id(),
            ticket: self
                .session
                .services
                .capture_project_ticket(&profile.id)
                .ok()?,
            profile,
            sql,
        })
    }

    pub(super) fn submit_saved_query(&self, target: &SaveTarget, title: &str) -> bool {
        let title = title.trim();
        if title.is_empty() {
            return false;
        }
        let profile = &target.profile;
        if target.ticket.is_current()
            && self.profile_for(&target.owner.0).as_ref() == Some(profile)
            && self
                .editors
                .get(&target.owner)
                .is_some_and(|live| live.state.entity_id() == target.entity)
        {
            let key = SavedQueryKey {
                project: profile.id.clone(),
                database: profile.database.clone(),
                driver: profile.driver.clone(),
                title: title.into(),
            };
            if let Some(query) = SavedQuery::decode(&key.encode(), target.sql.clone()) {
                self.session.save_query(Arc::new(query));
            }
        }
        true
    }

    pub(super) fn save_query(&mut self, owner: &Owner, w: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.capture_saved_query(owner, cx) else {
            return;
        };
        self.saving_query = true;
        let input = cx.new(|cx| InputState::new(w, cx).placeholder("e.g. Active users report"));
        let content = cx.new(|cx| dialog::SaveDialog::new(input.clone(), &target.sql, cx));
        let target = Rc::new(target);
        let first = std::cell::Cell::new(true);
        let view = cx.weak_entity();
        w.open_dialog(cx, move |host, w, cx| {
            if first.replace(false) {
                input.focus_handle(cx).focus(w, cx);
            }
            let title = input.clone();
            let save = view.clone();
            let close = view.clone();
            let target = target.clone();
            let content = content.clone();
            theme::dialog::surface(host, dialog::TITLE, theme::dialog::compact_width(w), cx)
                .content(move |body, _, _| body.min_h_0().child(content.clone()))
                .on_close(move |_, _, cx| {
                    let _ = close.update(cx, |v, cx| {
                        v.saving_query = false;
                        cx.notify();
                    });
                })
                .on_ok(move |_, _, cx| {
                    let name = title.read(cx).value();
                    save.update(cx, |v, _| v.submit_saved_query(&target, &name))
                        .unwrap_or(true)
                })
        });
        cx.notify();
    }

    pub(super) fn format_query(&mut self, owner: Owner, w: &mut Window, cx: &mut Context<Self>) {
        let project = self
            .session
            .documents
            .borrow()
            .tabs
            .iter()
            .find(|t| t.id == owner.0)
            .and_then(|t| t.project.clone());
        let ticket = project
            .as_deref()
            .and_then(|id| self.session.services.capture_project_ticket(id).ok());
        self.analysis_for(&owner, w, cx);
        let Some(live) = self.editors.get_mut(&owner) else {
            return;
        };
        live.controls.format.take();
        live.controls.format_sequence += 1;
        let sequence = live.controls.format_sequence;
        let entity = live.state.entity_id();
        let revision = live.language.revision;
        let text = live.state.read(cx).text().clone();
        let sql = text.to_string();
        if is_blank(&sql) {
            return;
        }
        let (cancel, result) = match self.session.formatter.borrow().submit(sql) {
            Ok(job) => job,
            Err(e) => {
                live.controls.message = Some(e);
                cx.notify();
                return;
            }
        };
        live.controls.format = Some(cancel);
        cx.spawn_in(w, async move |view, cx| {
            let Ok(result) = result.await else { return };
            let _ = view.update_in(cx, |v, w, cx| {
                let Some(live) = v.editors.get_mut(&owner) else {
                    return;
                };
                if live.state.entity_id() != entity || live.controls.format_sequence != sequence {
                    return;
                }
                live.controls.format = None;
                if ticket.as_ref().is_some_and(|t| !t.is_current())
                    || !v
                        .session
                        .documents
                        .borrow()
                        .tabs
                        .iter()
                        .any(|t| t.id == owner.0 && t.project == project)
                    || revision != live.language.revision
                    || text != *live.state.read(cx).text()
                {
                    return;
                }
                match result {
                    Ok(formatted) => {
                        live.controls.message = None;
                        if text != formatted.as_str() {
                            live.state.update(cx, |s, cx| {
                                s.set_selected_range(0..text.len(), cx);
                                s.replace(formatted, w, cx);
                            });
                        }
                    }
                    Err(e) => live.controls.message = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
