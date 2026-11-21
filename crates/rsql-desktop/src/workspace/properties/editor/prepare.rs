use super::*;
use rsql_core::{ApplicationServices, structure::TypeTargets};
use std::sync::Mutex;
use tokio::sync::{Semaphore, watch};

const CANCELLED: &str = "Structure review cancelled.";

pub(super) struct Inputs {
    pub draft: Arc<StructureDraft>,
    pub targets: BTreeMap<EditId, Arc<ForeignChoices>>,
}

pub(super) struct Interest {
    pub owner: StructureOwner,
    pub request: watch::Receiver<()>,
    pub stop: watch::Receiver<bool>,
}

impl Interest {
    fn current(&self) -> bool {
        self.owner.open() && matches!(self.request.has_changed(), Ok(false)) && !*self.stop.borrow()
    }
    async fn closed(&self) {
        let mut request = self.request.clone();
        let mut owner = self.owner.clone();
        let mut stop = self.stop.clone();
        tokio::select! { _ = request.changed() => (), _ = owner.closed(&mut stop) => () }
    }
}

impl Inputs {
    fn compute(
        self,
        types: TypeTargets,
        interest: &Interest,
    ) -> Result<(StructurePlan, PropertyText), String> {
        let current = || interest.current();
        let plan = self
            .draft
            .review_resolved_targets_while(&self.targets, &types, current)
            .map_err(|e| e.to_string())?;
        let mut sql = String::new();
        for stmt in plan.statements() {
            if !sql.is_empty() {
                sql.push('\n');
            }
            let mut remaining = stmt.as_str();
            while !remaining.is_empty() {
                if !current() {
                    return Err(CANCELLED.into());
                }
                let end = remaining.floor_char_boundary(remaining.len().min(65536));
                sql.push_str(&remaining[..end]);
                remaining = &remaining[end..];
            }
        }
        if !current() {
            return Err(CANCELLED.into());
        }
        Ok((plan, PropertyText::from(sql)))
    }
}

pub(super) async fn prepare(
    inputs: Inputs,
    interest: Interest,
    capacity: Arc<Semaphore>,
    services: Arc<ApplicationServices>,
) -> Result<(ApplyReview, PropertyText), String> {
    let interest = Arc::new(interest);
    let permit = tokio::select! {
        biased;
        _ = interest.closed() => return Err(CANCELLED.into()),
        permit = capacity.acquire_owned() => permit.map_err(|_| CANCELLED)?,
    };
    let names = inputs.draft.type_edit_names();
    let types = if names.is_empty() {
        TypeTargets::default()
    } else {
        services
            .run_metadata(
                &inputs.draft.snapshot().object().project,
                interest.closed(),
                async |client| {
                    if !interest.current() {
                        return Err(rsql_core::Error::QueryFailed(CANCELLED.into()));
                    }
                    TypeTargets::load(client, &names).await
                },
            )
            .await
            .map_err(|e| e.to_string())?
    };
    // The queued worker must not retain superseded drafts while waiting for a blocking thread.
    let queued = Arc::new(Mutex::new(Some((inputs, types, permit))));
    let worker = queued.clone();
    let active = interest.clone();
    let mut job = tokio::task::spawn_blocking(move || {
        let (inputs, types, _permit) = worker.lock().unwrap().take().ok_or(CANCELLED)?;
        inputs.compute(types, &active)
    });
    let (plan, text) = tokio::select! {
        biased;
        _ = interest.closed() => {
            if queued.lock().unwrap().take().is_some() { job.abort(); }
            let _ = job.await;
            return Err(CANCELLED.into());
        }
        result = &mut job => result.map_err(|e| e.to_string())??,
    };
    if !interest.current() {
        return Err(CANCELLED.into());
    }
    Ok((ApplyReview::new(interest.owner.clone(), plan)?, text))
}
