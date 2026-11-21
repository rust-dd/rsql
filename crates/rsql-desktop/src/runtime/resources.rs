use rsql_core::{
    ApplicationServices,
    metrics::{ResourceMonitor, SystemResourceUsage, with_pool_usage},
};
use std::{cell::RefCell, sync::Arc, time::Duration};
use tokio::{runtime::Handle, sync::watch, task::JoinHandle, time::Instant};

const PERIOD: Duration = Duration::from_secs(2);

#[derive(Clone, Default)]
pub(crate) struct Snapshot {
    pub usage: Option<SystemResourceUsage>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Request {
    visible: bool,
    stopped: bool,
    revision: u64,
}

pub(crate) struct Resources {
    request: watch::Sender<Request>,
    snapshot: watch::Sender<Snapshot>,
    worker: RefCell<Option<JoinHandle<()>>>,
}

impl Resources {
    pub(super) fn new(runtime: &Handle, services: Arc<ApplicationServices>) -> Self {
        Self::spawn(runtime, services, || {
            let mut monitor = None;
            move || {
                let monitor = match &mut monitor {
                    Some(current) => current,
                    None => monitor.insert(ResourceMonitor::new().map_err(|e| e.to_string())?),
                };
                Ok(monitor.sample())
            }
        })
    }

    fn spawn<S>(
        runtime: &Handle,
        services: Arc<ApplicationServices>,
        factory: impl FnMut() -> S + Send + 'static,
    ) -> Self
    where
        S: FnMut() -> Result<SystemResourceUsage, String> + Send + 'static,
    {
        let (request, updates) = watch::channel(Request {
            visible: false,
            stopped: false,
            revision: 0,
        });
        let snapshot = watch::channel(Snapshot::default()).0;
        let worker = runtime.spawn(poll(updates, snapshot.clone(), services, factory));
        Self {
            request,
            snapshot,
            worker: RefCell::new(Some(worker)),
        }
    }

    pub(crate) fn set_visible(&self, visible: bool) {
        self.request.send_if_modified(|r| {
            if r.stopped || r.visible == visible {
                return false;
            }
            r.visible = visible;
            r.revision += 1;
            true
        });
    }

    pub(crate) fn updates(&self) -> watch::Receiver<Snapshot> {
        self.snapshot.subscribe()
    }

    pub(crate) fn latest(&self) -> Snapshot {
        self.snapshot.borrow().clone()
    }

    pub(crate) async fn shutdown(&self) {
        self.request.send_modify(|r| r.stopped = true);
        if let Some(worker) = self.worker.take()
            && let Err(error) = worker.await
        {
            self.snapshot
                .send_modify(|s| s.error = Some(error.to_string()));
        }
    }
}

async fn poll<S>(
    mut requests: watch::Receiver<Request>,
    snapshot: watch::Sender<Snapshot>,
    services: Arc<ApplicationServices>,
    mut factory: impl FnMut() -> S,
) where
    S: FnMut() -> Result<SystemResourceUsage, String> + Send + 'static,
{
    let mut sample = factory();
    loop {
        let request = *requests.borrow_and_update();
        if request.stopped || requests.has_changed().is_err() {
            return;
        }
        if !request.visible {
            if requests.changed().await.is_err() {
                return;
            }
            continue;
        }

        let scheduled = Instant::now() + PERIOD;
        let gate = requests.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let still_visible = *gate.borrow() == request && gate.has_changed().is_ok();
            let result = still_visible.then(&mut sample);
            (sample, result)
        })
        .await;
        let (next, result) =
            joined.unwrap_or_else(|error| (factory(), Some(Err(error.to_string()))));
        sample = next;
        let Some(result) = result else { continue };
        let result = match result {
            Ok(usage) => tokio::select! {
                biased;
                change = requests.changed() => {
                    if change.is_err() { return; }
                    continue;
                }
                usage = with_pool_usage(usage, &services) => Ok(usage),
            },
            Err(error) => Err(error),
        };
        {
            let current = requests.borrow();
            if *current != request {
                continue;
            }
            snapshot.send_modify(|s| match result {
                Ok(usage) => {
                    s.usage = Some(usage);
                    s.error = None;
                }
                Err(error) => s.error = Some(error),
            });
        }
        let next = if scheduled > Instant::now() {
            scheduled
        } else {
            Instant::now() + PERIOD
        };
        tokio::select! {
            change = requests.changed() => if change.is_err() { return; },
            _ = tokio::time::sleep_until(next) => {}
        }
    }
}
