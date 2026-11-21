use super::*;
use rsql_desktop::{
    results::{
        edits::{EditTarget, Phase},
        provenance::candidate_relation,
    },
    sql::{TargetMode, execution_target},
};
use std::cell::RefCell;
use tokio::sync::watch;

#[derive(PartialEq)]
struct Stamp(EntityId, u64, Range<usize>, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReviewStatus {
    Checking,
    Matches,
    Mismatch,
    Unavailable,
}

struct Check {
    stamp: Stamp,
    status: ReviewStatus,
    _cancel: watch::Sender<()>,
}

#[derive(Default)]
pub(super) struct ReviewTarget(RefCell<Option<Check>>);

impl ReviewTarget {
    pub fn cancel(&self) {
        self.0.take();
    }

    pub fn retry(&self) {
        if self
            .0
            .borrow()
            .as_ref()
            .is_some_and(|check| check.status == ReviewStatus::Unavailable)
        {
            self.cancel();
        }
    }

    pub fn verify(
        self: &Rc<Self>,
        workspace: &Workspace,
        owner: &Owner,
        project: &str,
        target: &Arc<EditTarget>,
        w: &mut Window,
        cx: &mut App,
    ) -> ReviewStatus {
        let current = workspace
            .session
            .documents
            .borrow()
            .active()
            .filter(|tab| tab.id == owner.0)
            .and_then(|tab| tab.pane(owner.1).view())
            .filter(|view| view.data.id == target.result)
            .and_then(|view| view.edit.as_ref())
            .is_some_and(|edit| edit.phase == Phase::Review && Arc::ptr_eq(&edit.target, target));
        let Some(live) = workspace.editors.get(owner).filter(|_| {
            current && target.route.is_current() && !*workspace.session.preflight_stop.borrow()
        }) else {
            self.cancel();
            return ReviewStatus::Mismatch;
        };
        let state = live.state.read(cx);
        let selection = state.selected_range();
        let stamp = Stamp(
            live.state.entity_id(),
            live.language.revision,
            selection.clone(),
            state.cursor(),
        );
        if let Some(check) = self.0.borrow().as_ref().filter(|c| c.stamp == stamp) {
            return check.status;
        }
        self.cancel();
        let project = project.to_owned();
        let (cancel, cancelled) = watch::channel(());
        let stop = workspace.session.preflight_stop.subscribe();
        let text = state.text().clone();
        let target = target.clone();

        let oid = target.origin.relation.oid;
        let scan_cancel = cancelled.clone();
        let scan_stop = stop.clone();
        let runtime = &workspace.session.runtime;
        let scan = runtime.spawn_blocking(move || {
            selected_relation(&text, selection, &target, || {
                if *scan_stop.borrow()
                    || scan_cancel.has_changed().is_err()
                    || !target.route.is_current()
                {
                    Err("Cancelled".into())
                } else {
                    Ok(())
                }
            })
        });
        let services = workspace.session.services.clone();
        let lookup = runtime.spawn(async move {
            let name = match scan.await {
                Ok(Ok(name)) => name,
                Ok(Err(_)) => return ReviewStatus::Mismatch,
                Err(_) => return ReviewStatus::Unavailable,
            };
            let mut stop = stop;
            let mut cancelled = cancelled;
            let found = services
                .run_metadata(
                    &project,
                    async {
                        tokio::select! { _=cancelled.changed()=>(), _=stop.changed()=>() }
                    },
                    async |client| {
                        Ok(client
                            .query_one("SELECT pg_catalog.to_regclass($1)::oid", &[&name])
                            .await
                            .map_err(rsql_core::error::query_failed)?
                            .get::<_, Option<u32>>(0)
                            == Some(oid))
                    },
                )
                .await;
            match found {
                Ok(true) => ReviewStatus::Matches,
                Ok(false) => ReviewStatus::Mismatch,
                Err(_) => ReviewStatus::Unavailable,
            }
        });
        let weak = Rc::downgrade(self);
        let generation = cancel.subscribe();
        self.0.replace(Some(Check {
            stamp,
            status: ReviewStatus::Checking,
            _cancel: cancel,
        }));
        w.spawn(cx, async move |cx| {
            let status = lookup.await.unwrap_or(ReviewStatus::Unavailable);
            let published = if generation.has_changed().is_ok()
                && let Some(cache) = weak.upgrade()
                && let Some(current) = cache.0.borrow_mut().as_mut()
            {
                current.status = status;
                true
            } else {
                false
            };
            if published {
                let _ = cx.update(|w, _| w.refresh());
            }
        })
        .detach();
        ReviewStatus::Checking
    }
}

pub(super) fn selected_relation(
    text: &gpui_kit::component::input::Rope,
    range: Range<usize>,
    target: &EditTarget,
    check: impl Fn() -> Result<(), String>,
) -> Result<String, String> {
    let mut sql = String::with_capacity(text.len());
    for chunk in text.chunks() {
        check()?;
        sql.push_str(chunk);
    }
    let range = execution_target(&sql, (range.start, range.end), TargetMode::Run, &check)?;
    let relation = candidate_relation(&sql[range], target.generated.as_deref(), &target.route)
        .ok_or_else(|| {
            "The current editor statement changed. Select the original query to apply.".to_owned()
        })?;
    check()?;
    Ok(relation)
}
