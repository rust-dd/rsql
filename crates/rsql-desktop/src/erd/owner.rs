use super::{
    DiagramState, TableFailure, load,
    worker::{Prepared, Worker},
};
use rsql_core::{ApplicationServices, services::ProjectTicket};
use std::sync::{Arc, Mutex};
use tokio::{
    runtime::Handle,
    sync::{OnceCell, Semaphore, oneshot, watch},
};

#[derive(Clone)]
pub(super) struct Request {
    pub project: String,
    pub schema: String,
    pub route: ProjectTicket,
    generation: watch::Receiver<()>,
    owner: watch::Receiver<()>,
    receipt: watch::Receiver<()>,
}
impl Request {
    pub fn check(&self) -> Result<(), String> {
        if self.route.is_current()
            && matches!(self.generation.has_changed(), Ok(false))
            && matches!(self.owner.has_changed(), Ok(false))
            && matches!(self.receipt.has_changed(), Ok(false))
        {
            Ok(())
        } else {
            Err("Diagram request retired".into())
        }
    }
    pub async fn retired(&self) {
        let mut generation = self.generation.clone();
        let mut owner = self.owner.clone();
        let mut receipt = self.receipt.clone();
        tokio::select! {
            _ = self.route.retired() => (),
            _ = generation.changed() => (),
            _ = owner.changed() => (),
            _ = receipt.changed() => (),
        }
    }
}

#[derive(Debug)]
pub enum LoadStatus {
    Idle,
    Loading,
    Retired,
    Ready,
    Partial(Arc<[TableFailure]>),
    Failed(String),
}

/// One retained ERD document, bound to a project connection and schema for its lifetime.
pub struct DiagramOwner {
    project: String,
    schema: String,
    route: ProjectTicket,
    active: Option<Pending>,
    lane: Arc<Semaphore>,
    worker: Arc<OnceCell<Worker>>,
    lifetime: watch::Sender<()>,
    current: Option<Prepared>,
    status: Option<LoadStatus>,
}

struct Pending {
    _owner: watch::Sender<()>,
    ready: Arc<Mutex<Option<Prepared>>>,
    request: Request,
}

pub struct DiagramLoad {
    receipt: watch::Sender<()>,
    request: Request,
    result: oneshot::Receiver<Result<(), String>>,
}

pub struct DiagramDelivery {
    _receipt: watch::Sender<()>,
    request: Request,
    result: Result<(), String>,
}

impl DiagramLoad {
    pub async fn wait(self) -> DiagramDelivery {
        let result = self
            .result
            .await
            .unwrap_or_else(|_| Err("Diagram worker unavailable".into()));
        DiagramDelivery {
            _receipt: self.receipt,
            request: self.request,
            result,
        }
    }
}

impl DiagramOwner {
    pub fn new(project: String, schema: String, route: ProjectTicket) -> Self {
        Self {
            project,
            schema,
            route,
            active: None,
            lane: Arc::new(Semaphore::new(1)),
            worker: Default::default(),
            lifetime: watch::channel(()).0,
            current: None,
            status: None,
        }
    }

    /// Pass the captured catalog generation. Refresh or owner closure cancels the previous request.
    pub fn refresh(
        &mut self,
        runtime: &Handle,
        services: Arc<ApplicationServices>,
        generation: watch::Receiver<()>,
    ) -> DiagramLoad {
        let (active, owner) = watch::channel(());
        let (receipt, observer) = watch::channel(());
        self.status = Some(LoadStatus::Loading);
        let request = Request {
            project: self.project.clone(),
            schema: self.schema.clone(),
            route: self.route.clone(),
            generation,
            owner,
            receipt: observer,
        };
        let ready = Arc::new(Mutex::new(None));
        self.active = Some(Pending {
            ready: ready.clone(),
            _owner: active,
            request: request.clone(),
        });
        let work = request.clone();
        let (send, result) = oneshot::channel();
        let lane = self.lane.clone();
        let worker = self.worker.clone();
        let lifetime = self.lifetime.clone();
        let previous = self
            .current
            .as_ref()
            .and_then(|c| c.snapshot().diagram.as_ref())
            .map(Arc::downgrade)
            .unwrap_or_default();
        runtime.spawn(async move {
            let _permit = tokio::select! {
                biased;
                _ = work.retired() => return,
                permit = lane.acquire() => permit.expect("owner lane remains open"),
            };
            let result = async {
                let builder = load::load(services, work.clone()).await?;
                work.check()?;
                let worker = worker
                    .get_or_try_init(|| async { Worker::start(lifetime.clone()) })
                    .await?;
                let prepared = worker.prepare(builder, &work, previous).await?;
                work.check()?;
                *ready.lock().unwrap() = Some(prepared);
                Ok(())
            }
            .await;
            let prepared = result.is_ok();
            let _ = send.send(result);
            if prepared {
                work.retired().await;
                ready.lock().unwrap().take();
            }
        });
        DiagramLoad {
            request,
            result,
            receipt,
        }
    }

    pub fn publish(&mut self, delivery: DiagramDelivery) -> bool {
        if self
            .active
            .as_ref()
            .is_none_or(|s| !s.request.owner.same_channel(&delivery.request.owner))
        {
            return false;
        }
        let pending = self.active.take().unwrap();
        if delivery.request.check().is_err() {
            self.status = Some(LoadStatus::Retired);
            return false;
        }
        match delivery.result {
            Ok(()) => {
                let Some(replacement) = pending.ready.lock().unwrap().take() else {
                    self.status = Some(LoadStatus::Retired);
                    return false;
                };
                if delivery.request.check().is_err() {
                    self.status = Some(LoadStatus::Retired);
                    return false;
                }
                self.current = Some(replacement);
                self.status = None;
            }
            Err(message) => self.status = Some(LoadStatus::Failed(message)),
        }
        true
    }

    /// Signals closure immediately. Await the receipt for metadata settlement and model disposal.
    pub fn retire(self) -> impl Future<Output = ()> {
        let mut stopped = self.lifetime.subscribe();
        drop(self);
        async move {
            let _ = stopped.changed().await;
        }
    }

    pub fn status(&self) -> &LoadStatus {
        if !self.route.is_current()
            || self
                .active
                .as_ref()
                .is_some_and(|active| active.request.check().is_err())
        {
            &LoadStatus::Retired
        } else {
            self.status
                .as_ref()
                .or_else(|| self.current.as_ref().map(|c| &c.snapshot().status))
                .unwrap_or(&LoadStatus::Idle)
        }
    }
    pub fn capture_svg(&self) -> Option<super::svg::Snapshot> {
        let (graph, positions) = self.diagram()?.export_positions();
        // A partial refresh can publish its failures alongside an older, complete graph.
        let failures = match &self.current.as_ref()?.snapshot().status {
            LoadStatus::Partial(failures) => failures.clone(),
            _ => graph.failures.clone(),
        };
        let load_error = match self.status() {
            LoadStatus::Failed(message) => Some(message.clone()),
            _ => None,
        };
        Some(super::svg::Snapshot {
            graph,
            positions,
            failures,
            load_error,
        })
    }

    pub fn diagram(&self) -> Option<&DiagramState> {
        self.current
            .as_ref()?
            .snapshot()
            .diagram
            .as_deref()
            .filter(|_| self.route.is_current())
    }
}
