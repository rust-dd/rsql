use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::wire::{CELL_SEP, ROW_SEP, packed_cell_len, push_cell};

const DEFAULT_BUDGET: usize = 256 * 1024 * 1024;
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub(crate) struct MemoryBudget {
    limit: usize,
    used: AtomicUsize,
}

impl MemoryBudget {
    fn reserve(&self, bytes: usize) -> bool {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.limit)
            })
            .is_ok()
    }

    fn release(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::AcqRel);
    }
}

/// The reservation moves with its page, including while a query is still running.
pub(crate) struct PackedPage {
    pub data: String,
    budget: Arc<MemoryBudget>,
    reserved: usize,
}

impl PackedPage {
    pub fn new(budget: Arc<MemoryBudget>) -> Self {
        Self {
            data: String::new(),
            budget,
            reserved: 0,
        }
    }

    pub fn push<'a>(
        &mut self,
        cells: impl Iterator<Item = Option<&'a str>> + Clone,
        separator: bool,
    ) -> bool {
        let mut count = 0usize;
        let bytes = cells
            .clone()
            .map(|cell| {
                count += 1;
                packed_cell_len(cell)
            })
            .sum::<usize>()
            + count.saturating_sub(1)
            + usize::from(separator);
        let Some(required) = self.data.len().checked_add(bytes) else {
            return false;
        };
        if required > self.data.capacity() {
            let mut target = required.saturating_add(64 * 1024).min(self.budget.limit);
            if target < required {
                return false;
            }
            if !self.budget.reserve(target - self.reserved) {
                target = required;
                if !self.budget.reserve(target - self.reserved) {
                    return false;
                }
            }
            if self
                .data
                .try_reserve_exact(target - self.data.len())
                .is_err()
            {
                self.budget.release(target - self.reserved);
                return false;
            }
            self.reserved = target;
        }
        if separator {
            self.data.push(ROW_SEP);
        }
        for (index, cell) in cells.enumerate() {
            if index > 0 {
                self.data.push(CELL_SEP);
            }
            push_cell(&mut self.data, cell);
        }
        true
    }
}

impl Drop for PackedPage {
    fn drop(&mut self) {
        drop(std::mem::take(&mut self.data));
        self.budget.release(self.reserved);
    }
}

pub(crate) struct CachedQuery {
    pub pages: Vec<Arc<PackedPage>>,
    pub page_size: usize,
    pub accessed: Instant,
}

pub struct VirtualCache {
    closed: bool,
    pub(crate) entries: BTreeMap<String, CachedQuery>,
    pub(crate) budget: Arc<MemoryBudget>,
}

impl Default for VirtualCache {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualCache {
    pub fn new() -> Self {
        Self::with_budget(DEFAULT_BUDGET)
    }

    pub(crate) fn with_budget(bytes: usize) -> Self {
        Self {
            closed: false,
            entries: BTreeMap::new(),
            budget: Arc::new(MemoryBudget {
                limit: bytes,
                used: AtomicUsize::new(0),
            }),
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.closed = true;
        self.entries.clear();
    }

    pub fn contains_live(&self, id: &str) -> bool {
        !self.closed
            && self
                .entries
                .get(id)
                .is_some_and(|entry| entry.accessed.elapsed() < IDLE_TIMEOUT)
    }

    pub(super) fn ensure_open(&self) -> Result<(), crate::Error> {
        if self.closed {
            return Err(crate::Error::QueryFailed("Result cache is closed".into()));
        }
        Ok(())
    }

    pub(super) fn publish(
        &mut self,
        id: String,
        result: CachedQuery,
    ) -> Result<Option<CachedQuery>, crate::Error> {
        self.ensure_open()?;
        Ok(self.entries.insert(id, result))
    }

    pub(crate) fn take_expired(&mut self, age: Duration) -> Vec<CachedQuery> {
        let expired = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.accessed.elapsed() >= age)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|id| self.entries.remove(&id))
            .collect()
    }
}

pub async fn evict_idle(cache: &tokio::sync::Mutex<VirtualCache>) {
    let expired = cache.lock().await.take_expired(IDLE_TIMEOUT);
    drop(expired);
}
