use super::{
    ACTIVE_LIMIT, Clock, DECODED_LIMIT, QUEUE_LIMIT, Result, TILE_BYTES, TileKey, TileState,
    cache::Disk,
    fetch::Backend,
    scheduler::{Entries, run},
    transport::{Http, TileProvider, TileTransport},
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::SystemTime,
};
use tokio::{
    runtime::Handle,
    sync::{Notify, OwnedSemaphorePermit, Semaphore, watch},
    task::JoinHandle,
};

const CONSUMER_LIMIT: usize = 32;

#[derive(Debug)]
pub struct TileStats {
    pub consumers: usize,
    /// Includes slots reserved by the scheduler while the queue is full.
    pub active: usize,
    /// Subset of active slots holding pending keys without a running job.
    pub reserved: usize,
    pub queued: usize,
    pub visible: usize,
    pub retained: usize,
    pub decoded_bytes: usize,
}

pub struct TileService {
    shared: Arc<Shared>,
    worker: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    provider: TileProvider,
}

pub(super) struct Shared {
    pub entries: Mutex<Entries>,
    pub wake: Notify,
    pub revision: watch::Sender<()>,
    budget: Arc<Semaphore>,
    clock: Clock,
}

pub struct TileConsumer {
    shared: Arc<Shared>,
    visible: HashSet<TileKey>,
    revision: watch::Receiver<()>,
}

fn wall_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    })
}

impl TileService {
    pub fn open(runtime: Handle, directory: PathBuf) -> Self {
        Self::with_provider(
            runtime,
            directory,
            TileProvider::openstreetmap(),
            wall_clock(),
        )
    }

    pub fn open_profile(
        runtime: Handle,
        profile: Arc<rsql_core::migration::ProfileLease>,
        provider: TileProvider,
    ) -> Self {
        Self::start(
            runtime,
            Backend {
                provider,
                disk: Arc::new(Disk::for_profile(profile, provider)),
                clock: wall_clock(),
                transport: Arc::new(Http::default()),
            },
        )
    }

    pub fn with_provider(
        runtime: Handle,
        directory: PathBuf,
        provider: TileProvider,
        clock: Clock,
    ) -> Self {
        Self::with_transport(
            runtime,
            directory,
            provider,
            clock,
            Arc::new(Http::default()),
        )
    }

    pub fn with_transport(
        runtime: Handle,
        directory: PathBuf,
        provider: TileProvider,
        clock: Clock,
        transport: Arc<dyn TileTransport>,
    ) -> Self {
        let backend = Backend {
            provider,
            disk: Arc::new(Disk::new(directory.join(provider.directory()))),
            clock: clock.clone(),
            transport,
        };
        Self::start(runtime, backend)
    }

    pub(super) fn start(runtime: Handle, backend: Backend) -> Self {
        let shared = Arc::new(Shared {
            entries: Mutex::default(),
            wake: Notify::new(),
            revision: watch::channel(()).0,
            budget: Arc::new(Semaphore::new(DECODED_LIMIT / TILE_BYTES)),
            clock: backend.clock.clone(),
        });
        let provider = backend.provider;
        let worker = runtime.spawn(run(shared.clone(), Arc::new(backend)));
        Self {
            shared,
            worker: tokio::sync::Mutex::new(Some(worker)),
            provider,
        }
    }

    pub fn provider(&self) -> TileProvider {
        self.provider
    }

    pub fn consumer(&self) -> Result<TileConsumer> {
        let mut entries = self.shared.entries.lock().unwrap();
        if entries.stopped {
            return Err("Tile service has stopped".into());
        }
        if entries.consumers == CONSUMER_LIMIT {
            return Err("Tile consumer capacity reached".into());
        }
        entries.consumers += 1;
        Ok(TileConsumer {
            shared: self.shared.clone(),
            visible: HashSet::new(),
            revision: self.shared.revision.subscribe(),
        })
    }

    pub fn stats(&self) -> TileStats {
        let entries = self.shared.entries.lock().unwrap();
        TileStats {
            consumers: entries.consumers,
            active: entries.active(),
            reserved: entries.reserved.len(),
            queued: entries.queue.len(),
            visible: entries.visible.len(),
            retained: entries.recent.len(),
            decoded_bytes: DECODED_LIMIT - self.shared.budget.available_permits() * TILE_BYTES,
        }
    }

    fn stop(&self) {
        let mut entries = self.shared.entries.lock().unwrap();
        entries.stopped = true;
        while let Some((k, _)) = entries.assigned.pop_front() {
            entries.running.remove(&k);
        }
        for stop in entries.running.values() {
            stop.send_replace(true);
        }
        entries.visible.clear();
        entries.recent.clear();
        entries.queue.clear();
        entries.reserved.clear();
        self.shared.wake.notify_one();
        self.shared.revision.send_replace(());
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.stop();
        let mut worker = self.worker.lock().await;
        let result = match worker.as_mut() {
            Some(active) => active
                .await
                .map_err(|e| format!("Tile worker stopped: {e}")),
            None => Ok(()),
        };
        worker.take();
        result
    }
}

impl Drop for TileService {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Shared {
    pub fn reserve(&self) -> Result<OwnedSemaphorePermit> {
        let mut entries = self.entries.lock().unwrap();
        loop {
            if let Ok(permit) = self.budget.clone().try_acquire_owned() {
                return Ok(permit);
            }
            if entries.recent.pop_front().is_none() {
                return Err("Decoded tile capacity reached".into());
            }
        }
    }
}

impl TileConsumer {
    /// Replaces the viewport atomically. An over-capacity viewport clears its old interests.
    pub fn set_visible(&mut self, keys: &[TileKey]) -> Result<()> {
        let mut entries = self.shared.entries.lock().unwrap();
        if entries.stopped {
            return Err("Tile service has stopped".into());
        }
        let now = (self.shared.clock)();
        let mut next = HashSet::new();
        let oversized = keys.len() > ACTIVE_LIMIT + QUEUE_LIMIT;
        if !oversized {
            next.extend(keys.iter().copied());
        }
        if !oversized && self.visible == next {
            return Ok(());
        }
        for &key in self.visible.difference(&next) {
            entries.remove(key, now);
        }
        self.visible.retain(|k| next.contains(k));
        let additions = next.difference(&self.visible).copied().collect::<Vec<_>>();
        let overflow = !entries.can_admit(&additions, now);
        if oversized || overflow {
            for key in self.visible.drain() {
                entries.remove(key, now);
            }
            self.shared.wake.notify_one();
            return Err("Visible tile queue capacity reached".into());
        }
        for key in additions {
            entries.add(key, now);
        }
        self.visible = next;
        self.shared.revision.send_replace(());
        self.shared.wake.notify_one();
        Ok(())
    }

    pub fn state(&self, key: TileKey) -> Option<TileState> {
        if !self.visible.contains(&key) {
            return None;
        }
        self.shared
            .entries
            .lock()
            .unwrap()
            .visible
            .get_mut(&key)
            .map(|entry| entry.state_at((self.shared.clock)()))
    }

    pub fn retry_status(&self) -> super::RetryStatus {
        self.shared
            .entries
            .lock()
            .unwrap()
            .retry_status(&self.visible)
    }

    pub fn retry(&self) -> Result<()> {
        let mut entries = self.shared.entries.lock().unwrap();
        if entries.stopped {
            return Err("Tile service has stopped".into());
        }
        entries.retry_errors(&self.visible)?;
        self.shared.wake.notify_one();
        self.shared.revision.send_replace(());
        Ok(())
    }

    pub fn revisions(&self) -> watch::Receiver<()> {
        self.revision.clone()
    }

    pub fn fallback_deadline(&self) -> Option<(u64, std::time::Duration)> {
        let now = (self.shared.clock)();
        let entries = self.shared.entries.lock().unwrap();
        self.visible
            .iter()
            .filter_map(|key| entries.visible.get(key)?.offline_deadline())
            .filter(|deadline| *deadline != u64::MAX)
            .min()
            .map(|deadline| {
                (
                    deadline,
                    std::time::Duration::from_secs(deadline.saturating_sub(now)),
                )
            })
    }

    /// Coalesced notification. Read current states again after each change.
    pub async fn changed(&mut self) -> Result<()> {
        if self.shared.entries.lock().unwrap().stopped {
            return Err("Tile service has stopped".into());
        }
        self.revision.changed().await.map_err(|e| e.to_string())
    }
}

impl Drop for TileConsumer {
    fn drop(&mut self) {
        let mut entries = self.shared.entries.lock().unwrap();
        let now = (self.shared.clock)();
        for key in self.visible.drain() {
            entries.remove(key, now);
        }
        entries.consumers -= 1;
        self.shared.wake.notify_one();
    }
}
