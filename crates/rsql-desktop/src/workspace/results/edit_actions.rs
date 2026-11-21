use super::*;
use rsql_core::{
    postgres::mutation::apply_proven_mutations,
    services::{WriteOutcome, run_guarded_write},
};
use rsql_desktop::{
    execution::{Mode, RunSpec},
    results::edits::{EditSession, Phase},
};
use tokio::sync::watch;

mod verifier;

enum EditOutcome {
    Committed(String),
    Failed(String),
    Unknown(String),
}

impl EditOutcome {
    fn from_write(outcome: WriteOutcome<rsql_core::postgres::mutation::MutationReport>) -> Self {
        use crate::workspace::diagnostic::bounded_message;
        match outcome {
            WriteOutcome::Committed {
                value,
                cleanup_error,
            } => {
                let counts = format!(
                    "Committed {} updates and {} deletes.",
                    value.updated, value.deleted
                );
                Self::Committed(match cleanup_error {
                    Some(e) => bounded_message(format_args!("{counts} Connection cleanup: {e}")),
                    None => counts,
                })
            }
            WriteOutcome::Failed(e) => Self::Failed(bounded_message(e)),
            WriteOutcome::Indeterminate(e) => Self::Unknown(bounded_message(format_args!(
                "Commit outcome unknown. Run the query again before editing. {e}"
            ))),
        }
    }
}

pub(super) enum GridAction {
    Mark(usize, bool, Option<Uuid>),
    Navigate(Position, u64),
}

impl Workspace {
    pub(in crate::workspace) fn subscribe_edits(
        grid: &Entity<Grid>,
        owner: Owner,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let editor = owner.clone();
        vec![
            cx.subscribe(grid, move |v, g, e: &EditIntent, cx| {
                g.update(cx, |grid, _| grid.staging_inline = false);
                if g.read(cx).id() == e.result {
                    v.change_edit(&editor, e.result, cx, |view| {
                        if let Some(edit) = &mut view.edit {
                            edit.stage(e)
                        }
                    });
                }
            }),
            cx.subscribe_in(grid, w, move |v, g, e: &GridAction, w, cx| {
                let id = g.read(cx).id();
                match e {
                    GridAction::Mark(row, marked, session) => {
                        v.change_edit(&owner, id, cx, |view| {
                            if let Some(edit) = &mut view.edit
                                && edit.access.session == *session
                            {
                                edit.mark(*row, *marked)
                            }
                        })
                    }
                    GridAction::Navigate(pos, generation) => {
                        v.navigate_foreign_key(&owner, id, *pos, *generation, w, cx)
                    }
                }
            }),
        ]
    }

    pub(super) fn change_edit(
        &mut self,
        owner: &Owner,
        id: Uuid,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut ResultView),
    ) {
        if *self.session.preflight_stop.borrow() || owner.1 != Side::Main {
            return;
        }
        let mut docs = self.session.documents.borrow_mut();
        let Some(index) = docs.selected else { return };
        let tab = &mut docs.tabs[index];
        if tab.id != owner.0 {
            return;
        }
        let Some(view) = tab.main.view_mut().filter(|v| {
            v.data.id == id
                && v.data
                    .cache
                    .target()
                    .as_ref()
                    .is_some_and(|t| t.route.is_current())
        }) else {
            return;
        };
        view.sync_target();
        change(view);
        view.cell_access = match &view.edit {
            Some(edit) if edit.phase == Phase::Draft => edit.access.clone(),
            Some(edit) => Arc::new(CellAccess {
                eligibility: Some(Default::default()),
                ..(*edit.access).clone()
            }),
            None => Arc::new(
                view.data
                    .cache
                    .target()
                    .map(|t| t.browse())
                    .unwrap_or_default(),
            ),
        };
        cx.notify();
    }

    pub(super) fn confirm_changes(
        &mut self,
        owner: Owner,
        id: Uuid,
        reviewed: &Arc<CellAccess>,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.editors.get(&owner) else {
            return;
        };
        let state = live.state.read(cx);
        let entity = live.state.entity_id();
        let text = state.text().clone();
        let selection = state.selected_range();
        let caret = state.cursor();
        let revision = live.language.revision;
        let mut task = None;
        self.change_edit(&owner, id, cx, |view| {
            let Some(edit) = &mut view.edit else { return };
            if !Arc::ptr_eq(&edit.access, reviewed)
                || edit.phase != Phase::Review
                || edit.counts() == (0, 0)
            {
                return;
            }
            view.edit_error = None;
            edit.phase = Phase::Applying;
            let (sender, cancel) = watch::channel(());
            view.write_owner = Some(sender);
            task = Some((edit.target.clone(), edit.access.clone(), cancel));
        });
        let Some((target, access, cancel)) = task else {
            return;
        };
        let scan_text = text.clone();
        let selected = selection.clone();
        let checked = target.clone();
        let stopped = self.session.preflight_stop.subscribe();
        let worker = self.session.runtime.spawn_blocking(move || {
            let check = || {
                if *stopped.borrow() || cancel.has_changed().is_err() || !checked.route.is_current()
                {
                    Err("Cancelled".to_owned())
                } else {
                    Ok(())
                }
            };
            let relation =
                super::review_target::selected_relation(&scan_text, selected, &checked, check)?;
            Ok((EditSession::payload(&checked, &access, check)?, relation))
        });
        cx.spawn(async move |v, cx| {
            let result = worker.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = v.update(cx, |v, cx| {
                let Some(live) = v.editors.get(&owner) else {
                    return;
                };
                let state = live.state.read(cx);
                let active = v
                    .session
                    .documents
                    .borrow()
                    .active()
                    .is_some_and(|t| t.id == owner.0);
                let unchanged = active
                    && entity == live.state.entity_id()
                    && revision == live.language.revision
                    && state.selected_range() == selection
                    && state.cursor() == caret
                    && *state.text() == text;
                let (batch, relation) = match result {
                    Ok(batch) if unchanged => batch,
                    other => {
                        v.write_failed(
                            &owner,
                            id,
                            other
                                .err()
                                .unwrap_or_else(|| "Editor target changed. Review again.".into()),
                            false,
                            cx,
                        );
                        return;
                    }
                };
                let Some(profile) = v.profile_for(&owner.0) else {
                    v.write_failed(
                        &owner,
                        id,
                        "Choose the original connection and review again.".into(),
                        false,
                        cx,
                    );
                    return;
                };
                let mut docs = v.session.documents.borrow_mut();
                let Some(tab) = docs
                    .selected
                    .map(|i| &mut docs.tabs[i])
                    .filter(|t| t.id == owner.0)
                else {
                    return;
                };
                let Some(view) = tab.main.view().filter(|v| {
                    v.data.id == id
                        && v.edit.as_ref().is_some_and(|e| {
                            e.phase == Phase::Applying && Arc::ptr_eq(&e.target, &target)
                        })
                }) else {
                    return;
                };
                let mut cancel = view.write_owner.as_ref().unwrap().subscribe();
                let operation = tab.main.register_write(id, target.route.clone());
                drop(docs);
                let services = v.session.services.clone();
                let mut stopped = v.session.preflight_stop.subscribe();
                let target_for_write = target.clone();
                let project = profile.id.clone();
                let work = v.session.runtime.spawn(async move {
                    let target = target_for_write;
                    let (checks, observer) = verifier::observe(
                        services.clone(), project,
                    );
                    let write = async move {
                        let (checks, close) = checks;
                        let result = run_guarded_write(
                            &services,
                            &target.route,
                            &operation.to_string(),
                            30_000,
                            async |tx| {
                                apply_proven_mutations(
                                    tx, &target.origin.relation, &target.origin.output, &relation, &batch,
                                    |binding| verifier::confirm(checks.clone(), binding),
                                ).await
                            },
                            async {
                                tokio::select! {_=cancel.changed()=>(),_=stopped.wait_for(|s|*s)=>()}
                            },
                        ).await;
                        drop(checks);
                        drop(close);
                        result
                    };
                    let (result, ()) = tokio::join!(write, observer);
                    EditOutcome::from_write(result)
                });
                cx.spawn(async move |v, cx| {
                    let result = work.await;
                    let _ = v.update(cx, |v, cx| {
                        let accepted = v
                            .session
                            .documents
                            .borrow_mut()
                            .tabs
                            .iter_mut()
                            .find(|t| t.id == owner.0)
                            .is_some_and(|t| {
                                t.main.take_write_delivery(id, operation, &target.route)
                            });
                        if !accepted {
                            return;
                        }
                        match result {
                            Ok(EditOutcome::Committed(message)) => {
                                let mut docs = v.session.documents.borrow_mut();
                                let Some(tab) = docs.tabs.iter_mut().find(|t| t.id == owner.0) else {
                                    return;
                                };
                                if !tab.main.view().is_some_and(|v| v.write_owner.is_some()) {
                                    // An explicit Explain superseded this already accepted write.
                                    // Keep its committed notice without dispatching a stale refresh.
                                    tab.main.record_commit(target.route.clone(), message);
                                    cx.notify();
                                    return;
                                }
                                let timeout = tab.timeout.and_then(|n| u32::try_from(n).ok());
                                let Some((request, cancel)) = tab.begin(Side::Main) else {
                                    return;
                                };
                                tab.main.record_commit(target.route.clone(), message);
                                drop(docs);
                                if let Some(live) = v.editors.get_mut(&owner) {
                                    live.controls.message = None;
                                }
                                let mut spec =
                                    RunSpec::new(profile, target.sql.clone(), Mode::Query, timeout)
                                        .while_current(target.route.clone());
                                spec.generated = target.generated.clone();
                                v.dispatch_query(owner, request, spec, cancel, cx);
                            }
                            Ok(EditOutcome::Failed(e)) => v.write_failed(&owner,id,e,false,cx),
                            Ok(EditOutcome::Unknown(e)) => v.write_failed(&owner,id,e,true,cx),
                            Err(e) => v.write_failed(
                                &owner,
                                id,
                                if e.is_cancelled() { "Write worker cancelled. Check the database before editing." } else { "Write worker terminated unexpectedly. Check the database before editing." }.into(),
                                true,
                                cx,
                            ),
                        }
                    });
                })
                .detach();
            });
        })
        .detach();
    }

    fn write_failed(
        &mut self,
        owner: &Owner,
        id: Uuid,
        message: String,
        uncertain: bool,
        cx: &mut Context<Self>,
    ) {
        let mut docs = self.session.documents.borrow_mut();
        let Some(view) = docs
            .tabs
            .iter_mut()
            .find(|t| t.id == owner.0)
            .and_then(|t| t.main.view_mut())
            .filter(|v| v.data.id == id)
        else {
            return;
        };
        view.write_owner = None;
        if uncertain {
            view.write_uncertain = true;
            view.edit = None;
            view.cell_access = Arc::new(
                view.data
                    .cache
                    .target()
                    .map(|t| t.browse())
                    .unwrap_or_default(),
            );
        } else if let Some(edit) = &mut view.edit {
            edit.phase = Phase::Draft;
            view.cell_access = edit.access.clone();
        }
        view.edit_error = Some(message);
        cx.notify();
    }
}
