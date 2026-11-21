use super::{
    DiagramState, LoadStatus, SchemaGraph, Section, TableId, assemble::Builder, owner::Request,
};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use tokio::sync::{mpsc, oneshot, watch};

pub(super) struct Snapshot {
    pub diagram: Option<Arc<DiagramState>>,
    pub status: LoadStatus,
}

pub(super) struct Prepared {
    content: Option<(Snapshot, mpsc::OwnedPermit<Job>)>,
}
impl Prepared {
    pub fn snapshot(&self) -> &Snapshot {
        &self.content.as_ref().unwrap().0
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if let Some((snapshot, permit)) = self.content.take() {
            permit.send(Job::Retire(snapshot));
        }
    }
}

type Computation = Box<dyn FnOnce() + Send>;

enum Job {
    Admit(oneshot::Sender<oneshot::Sender<Computation>>),
    Retire(Snapshot),
}

pub(super) struct Worker {
    jobs: mpsc::Sender<Job>,
}

impl Worker {
    pub fn start(lifetime: watch::Sender<()>) -> Result<Self, String> {
        // A live snapshot reserves its own retirement slot before it can reach the owner.
        let (send, mut work) = mpsc::channel(3);
        std::thread::Builder::new()
            .name("erd-model".into())
            .spawn(move || {
                while let Some(job) = work.blocking_recv() {
                    match job {
                        Job::Admit(admit) => {
                            let (send, work) = oneshot::channel::<Computation>();
                            if admit.send(send).is_ok()
                                && let Ok(prepare) = work.blocking_recv()
                            {
                                prepare();
                            }
                        }
                        Job::Retire(snapshot) => drop(snapshot),
                    }
                }
                drop(lifetime);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { jobs: send })
    }

    pub async fn prepare(
        &self,
        builder: Option<Builder>,
        request: &Request,
        previous: Weak<DiagramState>,
    ) -> Result<Prepared, String> {
        let work = request.clone();
        let retired = "Diagram request retired";
        let closed = "Diagram worker unavailable";
        let retirement = tokio::select! {
            biased;
            _ = request.retired() => return Err(retired.into()),
            permit = self.jobs.clone().reserve_owned() => permit.map_err(|_| closed)?,
        };
        let (admit, slot) = oneshot::channel();
        tokio::select! {
            biased;
            _ = request.retired() => return Err(retired.into()),
            sent = self.jobs.send(Job::Admit(admit)) => sent.map_err(|_| closed)?,
        }
        let start = tokio::select! {
            biased;
            _ = request.retired() => return Err(retired.into()),
            sender = slot => sender.map_err(|_| closed)?,
        };
        let (send, ready) = oneshot::channel();
        start
            .send(Box::new(move || {
                let result = prepare(builder, &work, previous).map(|snapshot| Prepared {
                    content: Some((snapshot, retirement)),
                });
                let _ = send.send(result);
            }))
            .map_err(|_| closed)?;
        ready.await.map_err(|_| closed)?
    }
}

fn prepare(
    builder: Option<Builder>,
    request: &Request,
    previous: Weak<DiagramState>,
) -> Result<Snapshot, String> {
    request.check()?;
    let Some(builder) = builder else {
        return Ok(Snapshot {
            diagram: None,
            status: LoadStatus::Failed(format!(
                "Schema {} is missing or lacks USAGE permission",
                request.schema
            )),
        });
    };
    let old = previous.upgrade();
    let graph = builder.finish(|| request.check())?;
    let candidate = DiagramState::prepare(graph, old.as_deref(), || request.check())?;
    let graph = candidate.graph();
    let status = if graph.failures.is_empty() {
        LoadStatus::Ready
    } else {
        LoadStatus::Partial(graph.failures.clone())
    };
    if let Some(previous) = old
        && retain_after_failure(previous.graph(), graph, request)?
    {
        return Ok(Snapshot {
            diagram: Some(previous),
            status,
        });
    }
    Ok(Snapshot {
        diagram: Some(Arc::new(candidate)),
        status,
    })
}

fn retain_after_failure(
    previous: &SchemaGraph,
    current: &SchemaGraph,
    request: &Request,
) -> Result<bool, String> {
    if current.failures.is_empty()
        || !previous.failures.is_empty()
        || previous.schema() != current.schema()
    {
        return Ok(false);
    }
    let mut unread = HashMap::new();
    for failure in current.failures.iter() {
        request.check()?;
        match failure.section {
            Section::Visibility => return Ok(false),
            Section::Columns => {
                unread.insert(TableId(failure.table.oid), &failure.table);
            }
            Section::Constraints => (),
        }
    }
    for old in previous.tables() {
        request.check()?;
        let id = TableId(old.identity.oid);
        if unread.get(&id).copied() == Some(&old.identity) {
            continue;
        }
        if current.table(id).is_none_or(|t| t.identity != old.identity) {
            return Ok(false);
        }
        for column in &old.columns {
            request.check()?;
            if current
                .column(column.id)
                .is_none_or(|c| c.name != column.name)
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
