//! The workspace owns commit delivery independently of the modal's lifetime.

use super::*;
use rsql_core::services::{ProjectTicket, WriteOutcome};
use rsql_desktop::{
    metadata::{ObjectRef, load_structure},
    structure::{StructurePlan, StructureSnapshot},
};
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct StructureOwner {
    object: ObjectRef,
    route: ProjectTicket,
    epoch: u64,
    generation: watch::Receiver<()>,
    modal: watch::Receiver<()>,
}

impl StructureOwner {
    pub(crate) fn capture(
        v: &Workspace,
        object: &ObjectRef,
        modal: watch::Receiver<()>,
    ) -> Option<Self> {
        let route = v.navigation.connected_routes.get(&object.project)?;
        let catalog = v.navigation.catalogs.get(&object.project)?;
        (route.is_current()
            && matches!(modal.has_changed(), Ok(false))
            && !*v.session.preflight_stop.borrow())
        .then(|| Self {
            object: object.clone(),
            route: route.clone(),
            epoch: catalog.epoch,
            generation: catalog.generation.subscribe(),
            modal,
        })
    }

    fn route_current(&self, v: &Workspace) -> bool {
        self.route.is_current()
            && v.navigation
                .connected_routes
                .get(&self.object.project)
                .is_some_and(|r| r.same_generation(&self.route))
    }

    fn current(&self, v: &Workspace) -> bool {
        self.route_current(v)
            && !*v.session.preflight_stop.borrow()
            && matches!(self.modal.has_changed(), Ok(false))
            && matches!(self.generation.has_changed(), Ok(false))
            && v.navigation
                .catalogs
                .get(&self.object.project)
                .is_some_and(|c| c.epoch == self.epoch)
    }

    pub(in crate::workspace) fn open(&self) -> bool {
        self.route.is_current()
            && matches!(self.modal.has_changed(), Ok(false))
            && matches!(self.generation.has_changed(), Ok(false))
    }

    pub(in crate::workspace) async fn closed(&mut self, stop: &mut watch::Receiver<bool>) {
        if !self.route.is_current() {
            return;
        }
        tokio::select! {_=self.route.retired()=>(),_=self.modal.changed()=>(),_=self.generation.changed()=>(),_=stop.wait_for(|s|*s)=>()}
    }
}

pub(crate) struct ApplyReview {
    owner: StructureOwner,
    plan: Option<StructurePlan>,
}

pub(crate) enum ApplyResult {
    Failed(String),
    Indeterminate(String),
    Committed {
        statements: usize,
        cleanup_error: Option<String>,
        fresh: Result<std::sync::Arc<StructureSnapshot>, String>,
    },
}

pub(crate) struct Delivery {
    owner: StructureOwner,
    result: ApplyResult,
}

impl Delivery {
    pub(super) fn display_snapshot(&self) -> Option<std::sync::Arc<StructureSnapshot>> {
        match &self.result {
            ApplyResult::Committed {
                fresh: Ok(snapshot),
                ..
            } => Some(snapshot.clone()),
            _ => None,
        }
    }
    pub(crate) fn receive(self, v: &Workspace, object: &ObjectRef) -> Option<ApplyResult> {
        if self.owner.object != *object || !matches!(self.owner.modal.has_changed(), Ok(false)) {
            return None;
        }
        if self.owner.current(v) {
            return Some(self.result);
        }
        Some(match self.result {
            ApplyResult::Committed {
                statements,
                cleanup_error,
                ..
            } => ApplyResult::Committed {
                statements,
                cleanup_error,
                fresh: Err("The catalog or connection changed before metadata delivery.".into()),
            },
            result => result,
        })
    }
}

impl ApplyReview {
    pub(crate) fn new(owner: StructureOwner, plan: StructurePlan) -> Result<Self, String> {
        if plan.snapshot().object() != &owner.object || plan.statements().is_empty() {
            return Err("Review the changes for the selected table before applying.".into());
        }
        Ok(Self {
            owner,
            plan: Some(plan),
        })
    }

    pub(crate) fn start(
        &mut self,
        v: &Workspace,
        cx: &mut Context<Workspace>,
    ) -> Result<oneshot::Receiver<Delivery>, String> {
        if !self.owner.current(v) {
            return Err("The structure review closed or changed. Refresh and review again.".into());
        }
        let plan = self
            .plan
            .take()
            .ok_or("This review has already been submitted.")?;
        let owner = self.owner.clone();
        let services = v.session.services.clone();
        let runtime = v.session.runtime.clone();
        let stop = v.session.preflight_stop.subscribe();
        let namespace_oid = plan.snapshot().metadata().namespace_oid;
        let write = {
            let services = services.clone();
            let mut cancelled = owner.clone();
            let mut stop = stop.clone();
            let route = owner.route.clone();
            runtime.spawn(async move {
                services
                    .apply_structure(
                        &route,
                        &Uuid::new_v4().to_string(),
                        &plan,
                        30_000,
                        cancelled.closed(&mut stop),
                    )
                    .await
            })
        };
        let (send, delivery) = oneshot::channel();
        cx.spawn(async move |view, cx| {
            let outcome = write.await;
            let (statements, cleanup_error) = match outcome {
                Ok(WriteOutcome::Committed {
                    value,
                    cleanup_error,
                }) => (value, cleanup_error.map(diagnostic::bounded_message)),
                other => {
                    let result = match other {
                        Ok(WriteOutcome::Failed(e)) => {
                            ApplyResult::Failed(diagnostic::bounded_message(e))
                        }
                        Ok(WriteOutcome::Indeterminate(e)) => {
                            ApplyResult::Indeterminate(diagnostic::bounded_message(e))
                        }
                        Err(e) => ApplyResult::Indeterminate(diagnostic::bounded_message(e)),
                        Ok(WriteOutcome::Committed { .. }) => unreachable!(),
                    };
                    if matches!(result, ApplyResult::Indeterminate(_)) {
                        let _ = view.update(cx, |v, cx| {
                            if owner.route_current(v) {
                                v.refresh_catalog(&owner.object.project, cx);
                            }
                        });
                    }
                    let _ = send.send(Delivery { owner, result });
                    return;
                }
            };
            let next = view.update(cx, |v, cx| {
                let publish = owner.current(v);
                if owner.route_current(v) {
                    v.refresh_catalog(&owner.object.project, cx);
                }
                publish
                    .then(|| StructureOwner::capture(v, &owner.object, owner.modal.clone()))
                    .flatten()
            });
            let Ok(Some(next)) = next else {
                if matches!(owner.modal.has_changed(), Ok(false)) {
                    let _ =
                        send.send(Delivery {
                            owner,
                            result: ApplyResult::Committed {
                                statements,
                                cleanup_error,
                                fresh: Err(
                                    "The catalog or connection changed after submission.".into()
                                ),
                            },
                        });
                }
                return;
            };
            let mut cancelled = next.clone();
            let object = next.object.clone();
            let mut stop = stop;
            let fresh = runtime
                .spawn(async move {
                    load_structure(&services, &object, cancelled.closed(&mut stop))
                        .await
                        .and_then(|meta| {
                            if meta.namespace_oid == namespace_oid {
                                Ok(std::sync::Arc::new(StructureSnapshot::new(
                                    &object.project,
                                    meta,
                                )))
                            } else {
                                Err(rsql_core::Error::QueryFailed(
                                    "The reviewed schema changed. Refresh the catalog.".into(),
                                ))
                            }
                        })
                        .map_err(diagnostic::bounded_message)
                })
                .await
                .unwrap_or_else(|e| Err(diagnostic::bounded_message(e)));
            let result = ApplyResult::Committed {
                statements,
                cleanup_error,
                fresh,
            };
            let _ = send.send(Delivery {
                owner: next,
                result,
            });
        })
        .detach();
        Ok(delivery)
    }
}
