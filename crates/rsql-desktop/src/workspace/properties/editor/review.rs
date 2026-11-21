use super::*;
use crate::workspace::toast::Toast;
impl Workspace {
    pub(in crate::workspace::properties) fn review_structure(
        &mut self,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.properties.as_mut().filter(|p| !p.busy) else {
            return;
        };
        let Some(e) = p
            .editor
            .as_mut()
            .filter(|e| !e.recover && !e.has_incomplete_fk_pairs())
        else {
            return;
        };
        if e.review.is_some() {
            e.show_sql = !e.show_sql;
            p.transfer = None;
            cx.notify();
            return;
        }
        self.prepare_structure(false, w, cx);
    }

    fn prepare_structure(&mut self, submit: bool, w: &mut Window, cx: &mut Context<Self>) {
        let p = self.properties.as_mut().unwrap();
        let e = p.editor.as_mut().unwrap();
        if e.reviewing || e.recover || p.busy || e.has_incomplete_fk_pairs() {
            return;
        }
        if submit {
            p.busy = true;
            p.status = "Preparing changes…".into();
        }
        let id = p.id;
        let rev = Uuid::new_v4();
        e.revision = rev;
        e.reviewing = true;
        let targets = e
            .targets
            .iter()
            .filter_map(|(id, state)| {
                if let LoadState::Ready(target) = state {
                    Some((*id, target.clone()))
                } else {
                    None
                }
            })
            .collect();
        let draft = e.draft.clone();
        let owner = e.owner.clone();
        let (cancel, request) = tokio::sync::watch::channel(());
        e.review_cancel = Some(cancel);
        let inputs = prepare::Inputs { draft, targets };
        let interest = prepare::Interest {
            owner,
            request,
            stop: self.session.preflight_stop.subscribe(),
        };
        let job = self.session.runtime.spawn(prepare::prepare(
            inputs,
            interest,
            self.structure_review_slot.clone(),
            self.session.services.clone(),
        ));
        cx.spawn_in(w, async move |v, cx| {
            let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = v.update_in(cx, |v, w, cx| {
                if !v.property_current(id) {
                    if submit && let Some(p) = v.properties.as_mut().filter(|p| p.id == id) {
                        p.busy = false;
                        p.status = "Catalog changed while preparing the action.".into();
                        cx.notify();
                    }
                    return;
                }
                let p = v.properties.as_mut().unwrap();
                let Some(e) = p.editor.as_mut().filter(|e| e.revision == rev) else {
                    return;
                };
                e.reviewing = false;
                e.review_cancel = None;
                p.busy = false;
                match result {
                    Ok(review) => {
                        e.review = Some(review);
                        e.show_sql = !submit;
                        p.status.clear();
                        if submit {
                            v.apply_structure_review(w, cx);
                        }
                    }
                    Err(err) => p.status = diagnostic::bounded_message(err),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(in crate::workspace::properties) fn apply_structure_review(
        &mut self,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.properties.as_mut().filter(|p| !p.busy) else {
            return;
        };
        let Some(e) = p.editor.as_mut().filter(|e| {
            !e.recover && !e.reviewing && !e.has_incomplete_fk_pairs() && e.draft.change_count() > 0
        }) else {
            return;
        };
        let Some((mut review, _)) = e.review.take() else {
            self.prepare_structure(true, w, cx);
            return;
        };
        e.show_sql = false;
        let id = p.id;
        let rx = match review.start(self, cx) {
            Ok(rx) => rx,
            Err(e) => {
                self.properties.as_mut().unwrap().status = e;
                cx.notify();
                return;
            }
        };
        let p = self.properties.as_mut().unwrap();
        p.busy = true;
        p.transfer = None;
        p.status = "Applying changes…".into();
        let runtime = self.session.runtime.clone();
        cx.spawn_in(w, async move |v, cx| {
            let delivery = rx.await.ok();
            let rows = if let Some(snapshot) = delivery.as_ref().and_then(|d| d.display_snapshot())
            {
                Some(
                    runtime
                        .spawn_blocking(move || {
                            Arc::new(Arc::unwrap_or_clone(snapshot).draft())
                        })
                        .await
                        .map_err(|error| {
                            format!(
                                "Metadata display failed: {}",
                                diagnostic::bounded_message(error)
                            )
                        }),
                )
            } else {
                None
            };
            let _ = v.update(cx, |v, cx| {
                let Some(p) = v.properties.as_ref().filter(|p| p.id == id) else {
                    return;
                };
                let object = p.object.clone();
                let fresh_capture = Capture::new(v, &object.project)
                    .filter(|c| c.route.same_generation(&p.capture.route));
                let outcome = delivery.and_then(|d| d.receive(v, &object));
                let p = v.properties.as_mut().unwrap();
                p.busy = false;
                match outcome {
                    Some(ApplyResult::Failed(err)) => p.status = err,
                    Some(ApplyResult::Committed {
                        statements,
                        cleanup_error,
                        fresh,
                    }) => {
                        v.toasts.show(Toast::success("Table structure updated"), cx);
                        let noun = if statements == 1 {
                            "statement"
                        } else {
                            "statements"
                        };
                        p.status = format!("Committed {statements} {noun}.");
                        if let Some(err) = cleanup_error {
                            p.status.push_str(&format!(" Session cleanup: {err}"));
                        }
                        if let Some(c) = fresh_capture {
                            p.capture = c;
                        }
                        p.sections.clear();
                        let e = p.editor.as_mut().unwrap();
                        e.recover = true;
                        match fresh
                            .map_err(|e| format!("Metadata refresh failed: {e}"))
                            .and_then(|_| {
                                rows.unwrap_or_else(|| {
                                    Err("Metadata display was unavailable.".into())
                                })
                            }) {
                            Ok(rows) => {
                                p.editor = None;
                                v.install_property(
                                    Section::Structure,
                                    Ok(Content::Structure(Some(rows))),
                                    cx,
                                );
                            }
                            Err(error) => {
                                p.recovery_summary = Some(p.status.clone());
                                p.status.push_str(&format!(" {error}"));
                            }
                        }
                    }
                    other => {
                        let reason = match other {
                            Some(ApplyResult::Indeterminate(err)) => err,
                            _ => "The connection or catalog changed before delivery.".into(),
                        };
                        p.status = format!("The commit outcome is unknown. Check the database before editing again. {reason}");
                        p.recovery_summary = Some(p.status.clone());
                        p.editor.as_mut().unwrap().recover = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
