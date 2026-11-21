pub mod install;
mod package;
mod resolver;
pub use resolver::dispatch as dispatch_resolver;
mod panel;
pub use panel::{InstallUpdate, panel};
mod source;
mod transport;
mod verify;

use rsql_core::update::ReleaseCandidate;
use source::Source;
use std::thread::JoinHandle;
use std::{
    cell::{Cell, RefCell},
    sync::{Arc, Mutex},
};
use tokio::{runtime::Handle, sync::watch};
pub use verify::VerifiedArtifact;

pub type Candidate = Arc<ReleaseCandidate>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Startup,
    Manual,
}

#[derive(Debug, Clone)]
pub enum UpdateState {
    Idle,
    Checking {
        manual: bool,
        already: bool,
    },
    Available(Candidate),
    Downloading {
        candidate: Candidate,
        bytes: u64,
        total: Option<u64>,
    },
    Verifying(Candidate),
    Cancelling,
    Ready(Candidate),
    Installing(Candidate),
    Failed {
        message: String,
        visible: bool,
    },
    NoUpdate {
        message: String,
        visible: bool,
    },
}

pub fn notes(input: &str) -> String {
    let clean = input.trim();
    if clean.is_empty() {
        return "No release notes were provided.".into();
    }
    if clean.encode_utf16().take(501).count() <= 500 {
        return clean.into();
    }
    let mut units = 0;
    let mut text: String = clean
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 497
        })
        .collect();
    text.push_str("...");
    text
}

struct Worker {
    stop: watch::Sender<bool>,
    join: JoinHandle<()>,
}

pub struct UpdateController {
    runtime: Handle,
    source: Option<Arc<Source>>,
    state: watch::Sender<UpdateState>,
    ready: Arc<Mutex<Option<VerifiedArtifact>>>,
    worker: RefCell<Option<Worker>>,
    startup: Cell<bool>,
    closed: Cell<bool>,
}

impl UpdateController {
    /// Build on the Session's blocking loader, where bundle inspection and TLS setup can run.
    pub fn packaged(runtime: Handle) -> Self {
        Self::new(runtime, source::packaged())
    }

    fn new(runtime: Handle, source: Option<Source>) -> Self {
        Self {
            runtime,
            source: source.map(Arc::new),
            state: watch::channel(UpdateState::Idle).0,
            ready: Default::default(),
            worker: Default::default(),
            startup: Cell::new(false),
            closed: Cell::new(false),
        }
    }

    pub fn observe_state(&self) -> watch::Receiver<UpdateState> {
        self.state.subscribe()
    }

    fn run(&self, work: impl FnOnce(watch::Receiver<bool>) + Send + 'static) {
        let previous = self.worker.take();
        let (stop, stopped) = watch::channel(false);
        let runtime = self.runtime.clone();
        let join = std::thread::spawn(move || {
            if let Some(previous) = previous {
                let _ = previous.join.join();
            }
            let _runtime = runtime.enter();
            work(stopped);
        });
        self.worker.replace(Some(Worker { stop, join }));
    }

    pub fn check(&self, origin: Origin) {
        let manual = origin == Origin::Manual;
        if self.closed.get() || (!manual && (self.startup.replace(true) || self.source.is_none())) {
            return;
        }
        let mut start = false;
        self.state.send_modify(|s| match s {
            UpdateState::Checking {
                manual: visible,
                already,
            } => {
                *visible |= manual;
                *already |= manual;
            }
            UpdateState::Downloading { .. }
            | UpdateState::Verifying(_)
            | UpdateState::Cancelling
            | UpdateState::Installing(_) => {}
            _ => {
                start = true;
                *s = UpdateState::Checking {
                    manual,
                    already: false,
                };
            }
        });
        if !start {
            return;
        }
        let source = self.source.clone();
        let state = self.state.clone();
        let old = self.ready.lock().unwrap().take();
        self.run(move |mut stop| {
            drop(old);
            let result = source
                .as_ref()
                .ok_or_else(|| "Update checks require a configured RSQL package.".to_owned())
                .and_then(|s| Handle::current().block_on(s.check(&mut stop)));
            state.send_modify(|current| {
                let UpdateState::Checking { manual, .. } = *current else {
                    return;
                };
                *current = match result {
                    Ok(source::Outcome::Available(c)) => UpdateState::Available(c),
                    Ok(source::Outcome::NoUpdate(message)) => UpdateState::NoUpdate {
                        message,
                        visible: manual,
                    },
                    Ok(source::Outcome::Guidance(message)) => UpdateState::NoUpdate {
                        message,
                        visible: true,
                    },
                    Err(message) => UpdateState::Failed {
                        message,
                        visible: manual,
                    },
                };
            });
        });
    }

    pub fn accept(&self, selected: &Candidate) {
        let Some(source) = &self.source else { return };
        if self.closed.get() {
            return;
        }
        let accepted = self.state.send_if_modified(|s| {
            if !matches!(s, UpdateState::Available(now) if Arc::ptr_eq(selected, now)) {
                return false;
            }
            *s = UpdateState::Downloading {
                candidate: selected.clone(),
                bytes: 0,
                total: None,
            };
            true
        });
        if !accepted {
            return;
        }
        let candidate = selected.clone();
        let source = source.clone();
        let ready = self.ready.clone();
        let state = self.state.clone();
        self.run(move |mut stop| {
            let result = Handle::current().block_on(source.download(&candidate, &state, &mut stop));
            state.send_modify(|s| {
                *s = if *stop.borrow() || matches!(s, UpdateState::Cancelling) {
                    UpdateState::Idle
                } else {
                    match result {
                        Ok(verified) => {
                            *ready.lock().unwrap() = Some(verified);
                            UpdateState::Ready(candidate)
                        }
                        Err(message) => UpdateState::Failed {
                            message,
                            visible: true,
                        },
                    }
                };
            });
        });
    }

    /// Transfer the verified staging file only for the candidate still displayed as ready.
    pub fn take_ready(&self, selected: &Candidate) -> Option<VerifiedArtifact> {
        let mut file = None;
        self.state.send_if_modified(|s| {
            if !matches!(s, UpdateState::Ready(current) if Arc::ptr_eq(current, selected)) {
                return false;
            }
            file = self.ready.lock().unwrap().take();
            *s = UpdateState::Idle;
            true
        });
        file
    }

    /// Hand the ready file to the installer, keeping the panel on the candidate until restart.
    pub fn begin_install(&self) -> Option<VerifiedArtifact> {
        let mut file = None;
        self.state.send_if_modified(|s| {
            let UpdateState::Ready(candidate) = s else {
                return false;
            };
            file = self.ready.lock().unwrap().take();
            if file.is_none() {
                return false;
            }
            *s = UpdateState::Installing(candidate.clone());
            true
        });
        file
    }

    pub fn install_failed(&self, message: String) {
        self.state.send_if_modified(|s| {
            if !matches!(s, UpdateState::Installing(_)) {
                return false;
            }
            *s = UpdateState::Failed {
                message,
                visible: true,
            };
            true
        });
    }

    pub fn cancel_download(&self, candidate: &Candidate) {
        if self.state.send_if_modified(|s| {
            if !matches!(s, UpdateState::Downloading { candidate: c, .. } | UpdateState::Verifying(c) if Arc::ptr_eq(c, candidate)) {
                return false;
            }
            *s = UpdateState::Cancelling;
            true
        }) && let Some(worker) = &*self.worker.borrow() {
            worker.stop.send_replace(true);
        }
    }

    pub fn dismiss(&self, shown: &UpdateState) {
        self.state.send_if_modified(|s| {
            let same = match (&*s, shown) {
                (UpdateState::Available(a), UpdateState::Available(b))
                | (UpdateState::Ready(a), UpdateState::Ready(b)) => Arc::ptr_eq(a, b),
                (
                    UpdateState::Failed { message: a, .. },
                    UpdateState::Failed { message: b, .. },
                )
                | (
                    UpdateState::NoUpdate { message: a, .. },
                    UpdateState::NoUpdate { message: b, .. },
                ) => a == b,
                _ => false,
            };
            if same {
                if let Some(file) = self.ready.lock().unwrap().take() {
                    self.runtime.spawn_blocking(move || drop(file));
                }
                *s = UpdateState::Idle
            }
            same
        });
    }

    pub async fn shutdown(&self) {
        self.closed.set(true);
        let worker = self.worker.take();
        if let Some(worker) = &worker {
            worker.stop.send_replace(true);
        }
        let ready = self.ready.clone();
        self.runtime
            .spawn_blocking(move || {
                if let Some(worker) = worker {
                    let _ = worker.join.join();
                }
                ready.lock().unwrap().take();
            })
            .await
            .expect("update cleanup worker stopped");
        self.state.send_replace(UpdateState::Idle);
    }
}

impl Drop for UpdateController {
    fn drop(&mut self) {
        // Construction can succeed before a window opens. Also drain that exit path.
        if let Some(worker) = self.worker.take() {
            worker.stop.send_replace(true);
            let _ = worker.join.join();
        }
        let owned = self.ready.lock().unwrap().take();
        if owned.is_some() {
            let _ = std::thread::spawn(move || drop(owned)).join();
        }
    }
}
