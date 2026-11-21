use super::{
    review_dialog,
    review_target::{ReviewStatus, ReviewTarget},
    *,
};
use gpui_kit::component::{ActiveTheme, Disableable, button::ButtonVariants};
use rsql_desktop::results::edits::{EditSession, Phase};

impl Workspace {
    pub(in crate::workspace) fn edit_toolbar(
        &self,
        owner: Owner,
        view: &ResultView,
        cx: &Context<Self>,
    ) -> Div {
        let mut bar = div().flex().items_center().gap_1().text_size(px(12.));
        let id = view.data.id;
        let Some(edit) = &view.edit else {
            if view.export_job.is_none()
                && view.allows(ResultAction::Edit)
                && view
                    .data
                    .cache
                    .target()
                    .as_ref()
                    .is_some_and(|t| t.can_edit())
                && !view.write_uncertain
            {
                bar = bar.child(super::toolbar::result_action(
                    self.button("Edit rows", cx, move |v, _, cx| {
                        v.change_edit(&owner, id, cx, |view| {
                            if view.export_job.is_some()
                                || !view.allows(ResultAction::Edit)
                                || view.choice() != rsql_desktop::results::ViewChoice::Grid
                                || view.edit.is_some()
                                || view.write_uncertain
                            {
                                return;
                            }
                            view.choose(rsql_desktop::results::ViewChoice::Grid);
                            view.reset_filter();
                            view.edit_error = None;
                            view.navigation_owner = None;
                            view.edit = view.data.cache.target().and_then(EditSession::begin);
                        });
                    })
                    .label("Edit"),
                    gpui_kit::assets::IconName::SquarePen,
                    cx,
                ));
            } else if view.export_job.is_none()
                && view.data.total_rows > 0
                && !view.data.columns().is_empty()
            {
                bar = bar.child(
                    view.data
                        .cache
                        .target()
                        .as_ref()
                        .map_or("Read only · row identity unavailable", |t| {
                            t.readonly_reason()
                        }),
                );
            }
            return bar;
        };
        let (updates, deletes) = edit.counts();
        let actions = owner.clone();
        bar = bar
            .gap_2()
            .text_color(cx.theme().muted_foreground)
            .child("NULL = SQL NULL · \"NULL\" = text")
            .child(format!("{updates} to update · {deletes} to delete"));
        match edit.phase {
            Phase::Draft => {
                bar = bar.child(super::toolbar::result_action(
                    self.button("Discard", cx, move |v, _, cx| {
                        v.change_edit(&actions, id, cx, |view| {
                            view.edit = None;
                            view.edit_error = None;
                        })
                    }),
                    gpui_kit::assets::IconName::X,
                    cx,
                ));
                let has_changes = self
                    .editors
                    .get(&owner)
                    .and_then(|l| l.grid.as_ref())
                    .map(|g| g.read(cx))
                    .filter(|g| g.id() == id)
                    .map_or((updates, deletes) != (0, 0), |g| {
                        g.has_review_changes(edit, cx)
                    });
                bar.child(
                    super::toolbar::result_action(
                        self.button("Review changes", cx, move |v, w, cx| {
                            v.review_changes(owner.clone(), id, w, cx)
                        }),
                        gpui_kit::assets::IconName::Check,
                        cx,
                    )
                    .success()
                    .disabled(!has_changes),
                )
            }
            Phase::Review => bar.child("Reviewing changes…"),
            Phase::Applying => bar.child("Applying…"),
        }
    }

    fn review_changes(&mut self, owner: Owner, id: Uuid, w: &mut Window, cx: &mut Context<Self>) {
        let Some(grid) = self.editors.get(&owner).and_then(|l| l.grid.clone()) else {
            return;
        };
        if grid.read(cx).id() != id || !grid.update(cx, |g, cx| g.finish_editor(true, true, w, cx))
        {
            return;
        }
        let view = cx.weak_entity();
        // The cell completion's typed intent must be delivered before freezing the batch.
        w.defer(cx, move |w, cx| {
            let _ = view.update(cx, |v, cx| {
                let Some(profile) = v.profile_for(&owner.0) else {
                    return;
                };
                let mut counts = None;
                v.change_edit(&owner, id, cx, |view| {
                    if let Some(edit) = &mut view.edit
                        && edit.phase == Phase::Draft
                        && edit.counts() != (0, 0)
                    {
                        counts = Some((edit.counts(), edit.access.clone(), edit.target.clone()));
                        edit.phase = Phase::Review;
                    }
                });
                if let Some(((updates, deletes), reviewed, target)) = counts {
                    let this = cx.weak_entity();
                    let eligibility = Rc::new(ReviewTarget::default());
                    w.open_dialog(cx, move |dialog, w, cx| {
                        let status = this
                            .update(cx, |v, cx| {
                                eligibility.verify(v, &owner, &profile.id, &target, w, cx)
                            })
                            .unwrap_or(ReviewStatus::Mismatch);
                        let allowed = status == ReviewStatus::Matches;
                        let confirmed = reviewed.clone();
                        let dismissed = reviewed.clone();
                        let apply = this.clone();
                        let close = this.clone();
                        let close_check = eligibility.clone();
                        let confirm_owner = owner.clone();
                        let close_owner = owner.clone();
                        theme::dialog::surface(
                            dialog,
                            review_dialog::TITLE,
                            px(review_dialog::WIDTH),
                            cx,
                        )
                        .child(review_dialog::content(
                            (updates, deletes),
                            status,
                            eligibility.clone(),
                            cx,
                        ))
                        .on_ok(move |_, _, cx| {
                            if !allowed {
                                return false;
                            }
                            let _ = apply.update(cx, |v, cx| {
                                v.confirm_changes(confirm_owner.clone(), id, &confirmed, cx)
                            });
                            true
                        })
                        .on_close(move |_, _, cx| {
                            close_check.cancel();
                            let _ = close.update(cx, |v, cx| {
                                let mut docs = v.session.documents.borrow_mut();
                                if let Some(view) = docs
                                    .tabs
                                    .iter_mut()
                                    .find(|t| t.id == close_owner.0)
                                    .and_then(|t| t.main.view_mut())
                                    .filter(|v| v.data.id == id)
                                    && let Some(edit) = &mut view.edit
                                    && edit.phase == Phase::Review
                                    && Arc::ptr_eq(&edit.access, &dismissed)
                                {
                                    edit.phase = Phase::Draft;
                                    view.cell_access = edit.access.clone();
                                    cx.notify();
                                }
                            });
                        })
                    });
                }
            });
        });
    }
}
