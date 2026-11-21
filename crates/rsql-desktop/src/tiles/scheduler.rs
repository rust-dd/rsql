use super::{
    ACTIVE_LIMIT, QUEUE_LIMIT, TileKey, TileState,
    fetch::{Backend, Outcome},
    service::Shared,
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::watch, time::Instant};

const RECENT_LIMIT: usize = 128;

pub(super) struct Entry {
    state: TileState,
    offline_until: Option<u64>,
    refs: usize,
    fresh_until: u64,
    failures: u8,
    retry_at: Option<Instant>,
}

impl Entry {
    fn loading() -> Self {
        Self {
            state: TileState::Loading,
            offline_until: None,
            refs: 1,
            fresh_until: 0,
            failures: 0,
            retry_at: None,
        }
    }

    pub fn state_at(&mut self, now: u64) -> TileState {
        self.expire_offline_image(now);
        self.state.clone()
    }

    pub fn offline_deadline(&self) -> Option<u64> {
        self.offline_until
    }

    fn expire_offline_image(&mut self, now: u64) {
        if self.offline_until.is_some_and(|deadline| now >= deadline)
            && let TileState::Error { image, .. } = &mut self.state
        {
            image.take();
            self.offline_until = None;
        }
    }

    fn reusable(&self, now: u64) -> bool {
        self.fresh_until > now || self.retry_at.is_some_and(|t| t > Instant::now())
    }
}

#[derive(Default)]
pub(super) struct Entries {
    pub stopped: bool,
    pub consumers: usize,
    pub queue: VecDeque<TileKey>,
    pub assigned: VecDeque<(TileKey, watch::Receiver<bool>)>,
    /// Active slots retaining a key when the bounded queue is full.
    pub reserved: VecDeque<TileKey>,
    pub running: HashMap<TileKey, watch::Sender<bool>>,
    pub visible: HashMap<TileKey, Entry>,
    pub recent: VecDeque<(TileKey, Entry)>,
}

impl Entries {
    fn requires_queue(&self, k: TileKey, now: u64) -> bool {
        if let Some(entry) = self.visible.get(&k) {
            return !matches!(entry.state, TileState::Loading)
                && !entry.reusable(now)
                && !self.queue.contains(&k)
                && !self.running.contains_key(&k)
                && !self.reserved.contains(&k);
        }
        self.recent
            .iter()
            .find(|(key, _)| *key == k)
            .is_none_or(|(_, e)| e.retry_at.is_some() || !e.reusable(now))
    }

    fn eligible(&self, k: TileKey) -> bool {
        if self.running.contains_key(&k) {
            return false;
        }
        self.visible
            .get(&k)
            .is_some_and(|entry| entry.retry_at.is_none_or(|at| at <= Instant::now()))
    }

    pub fn active(&self) -> usize {
        self.running.len() + self.reserved.len()
    }

    pub fn can_admit(&self, additions: &[TileKey], now: u64) -> bool {
        let required = additions
            .iter()
            .filter(|k| self.requires_queue(**k, now))
            .count();
        self.active() + self.queue.len() + required <= ACTIVE_LIMIT + QUEUE_LIMIT
    }

    fn manual_retry(&self, key: &TileKey) -> bool {
        self.visible.get(key).is_some_and(|entry| {
            matches!(entry.state, TileState::Error { .. }) && entry.retry_at.is_none()
        }) && !self.running.contains_key(key)
            && !self.queue.contains(key)
            && !self.reserved.contains(key)
    }

    pub fn retry_status(&self, keys: &std::collections::HashSet<TileKey>) -> super::RetryStatus {
        let mut status = super::RetryStatus::default();
        for key in keys {
            status.manual += usize::from(self.manual_retry(key));
            status.scheduled += usize::from(
                self.visible
                    .get(key)
                    .is_some_and(|entry| entry.retry_at.is_some()),
            );
        }
        status
    }

    pub fn retry_errors(&mut self, keys: &std::collections::HashSet<TileKey>) -> super::Result<()> {
        let retry = keys
            .iter()
            .filter(|key| self.manual_retry(key))
            .copied()
            .collect::<Vec<_>>();
        if self.active() + self.queue.len() + retry.len() > ACTIVE_LIMIT + QUEUE_LIMIT {
            return Err("Visible tile queue capacity reached".into());
        }
        for key in retry {
            let entry = self.visible.get_mut(&key).unwrap();
            entry.state = TileState::Loading;
            entry.offline_until = None;
            entry.fresh_until = 0;
            self.enqueue(key);
            self.assign();
        }
        Ok(())
    }

    fn enqueue(&mut self, k: TileKey) {
        if self.queue.len() < QUEUE_LIMIT {
            self.queue.push_back(k);
        } else {
            debug_assert!(self.active() < ACTIVE_LIMIT);
            self.reserved.push_back(k);
        }
    }

    pub fn assign(&mut self) {
        while self.running.len() < ACTIVE_LIMIT {
            let k = if let Some(i) = self.queue.iter().position(|k| self.eligible(*k)) {
                let k = self.queue.remove(i).unwrap();
                if self.active() == ACTIVE_LIMIT {
                    // Swap ownership so a long cooldown cannot block an eligible queued tile.
                    self.queue.push_back(self.reserved.pop_front().unwrap());
                }
                k
            } else if let Some(i) = self.reserved.iter().position(|k| self.eligible(*k)) {
                self.reserved.remove(i).unwrap()
            } else {
                break;
            };
            let (stop, rx) = watch::channel(false);
            self.running.insert(k, stop);
            self.assigned.push_back((k, rx));
        }
        while self.queue.len() < QUEUE_LIMIT
            && let Some(k) = self.reserved.pop_front()
        {
            self.queue.push_back(k);
        }
    }

    pub fn add(&mut self, k: TileKey, now: u64) {
        let queue = self.requires_queue(k, now);
        if let Some(entry) = self.visible.get_mut(&k) {
            entry.refs += 1;
        } else {
            let mut entry = self
                .recent
                .iter()
                .position(|(key, _)| *key == k)
                .and_then(|i| self.recent.remove(i))
                .map(|(_, e)| e)
                .unwrap_or_else(Entry::loading);
            entry.refs = 1;
            self.visible.insert(k, entry);
        }
        let entry = self.visible.get_mut(&k).unwrap();
        entry.expire_offline_image(now);
        if queue {
            if !entry.reusable(now) {
                entry.state = TileState::Loading;
                entry.offline_until = None;
                entry.retry_at = None;
            }
            self.enqueue(k);
            self.assign();
        }
    }

    pub fn remove(&mut self, k: TileKey, now: u64) {
        let Some(entry) = self.visible.get_mut(&k) else {
            return;
        };
        entry.refs -= 1;
        if entry.refs > 0 {
            return;
        }
        let mut entry = self.visible.remove(&k).unwrap();
        entry.expire_offline_image(now);
        self.reserved.retain(|key| *key != k);
        self.queue.retain(|key| *key != k);
        if let Some(i) = self.assigned.iter().position(|(key, _)| *key == k) {
            self.assigned.remove(i);
            self.running.remove(&k);
        } else if let Some(stop) = self.running.get(&k) {
            stop.send_replace(true);
        }
        if entry.reusable(now) {
            if self.recent.len() == RECENT_LIMIT {
                self.recent.pop_front();
            }
            self.recent.push_back((k, entry));
        }
    }

    fn next_retry(&self) -> Option<Instant> {
        if self.running.len() == ACTIVE_LIMIT {
            return None;
        }
        self.queue
            .iter()
            .chain(&self.reserved)
            .filter(|key| !self.running.contains_key(key))
            .filter_map(|key| self.visible.get(key)?.retry_at)
            .min()
    }

    fn finish(&mut self, k: TileKey, result: Outcome) {
        let stop = self.running.remove(&k).unwrap();
        if *stop.borrow() {
            return;
        }
        if let Some(entry) = self.visible.get_mut(&k) {
            entry.state = result.state;
            entry.offline_until = result.offline_until;
            entry.fresh_until = result.fresh_until;
            if let Some(minimum) = result.retry {
                entry.failures = entry.failures.saturating_add(1);
                let delay = Duration::from_secs((1u64 << entry.failures.min(6)).min(60));
                entry.retry_at = Some(Instant::now() + delay.max(minimum));
                self.enqueue(k);
            } else {
                entry.failures = 0;
                entry.retry_at = None;
            }
        }
    }
}

pub(super) async fn run(shared: Arc<Shared>, backend: Arc<Backend>) {
    let mut jobs = FuturesUnordered::new();
    loop {
        let next = {
            let mut entries = shared.entries.lock().unwrap();
            if entries.stopped && jobs.is_empty() {
                return;
            }
            entries.assign();
            let next = entries.next_retry();
            while let Some((k, rx)) = entries.assigned.pop_front() {
                let backend = backend.clone();
                let shared = shared.clone();
                jobs.push(async move {
                    let result = backend.load(&shared, k, rx).await;
                    (k, result)
                });
            }
            next
        };
        tokio::select! {
            Some((key, result)) = jobs.next(), if !jobs.is_empty() => {
                shared.entries.lock().unwrap().finish(key, result);
                shared.revision.send_replace(());
            }
            _ = shared.wake.notified() => {}
            _ = tokio::time::sleep_until(next.unwrap_or_else(Instant::now)), if next.is_some() => {}
        }
    }
}
