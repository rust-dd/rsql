mod activation;
use super::{
    DecodedBudget, PackedSource, PageCache, PageError, decode_columns, decode_page,
    pages::{CacheState, LoadedPage},
};
pub use activation::FirstPageRead;
use rsql_core::services::ProjectTicket;
use std::{
    collections::BTreeSet,
    ops::Range,
    sync::{Arc, Mutex, Weak},
};
use tokio::{runtime::Handle, sync::watch};

pub const MAX_CONCURRENT: usize = 6;
pub const MAX_QUEUED: usize = 32;

#[derive(Clone)]
pub struct PageService {
    runtime: Handle,
    budget: DecodedBudget,
    stop: watch::Sender<bool>,
}

impl PageService {
    pub(crate) fn new(runtime: Handle, budget: DecodedBudget) -> Self {
        Self {
            runtime,
            budget,
            stop: watch::channel(false).0,
        }
    }
    pub fn budget(&self) -> &DecodedBudget {
        &self.budget
    }
    pub(crate) async fn shutdown(&self) {
        self.stop.send_replace(true);
        self.stop.closed().await;
    }
}

#[derive(Default, Clone)]
pub(crate) struct PageLane(Arc<Mutex<LaneState>>);

#[derive(Default)]
struct LaneState {
    current: Weak<Loader>,
    active: usize,
}

struct Schedule {
    active: bool,
    disposed: bool,
    grid_viewport: Option<Range<usize>>,
    queue: Vec<usize>,
    loading: BTreeSet<usize>,
    // Manual retries advance outward on both sides of the viewport anchor.
    retried_range: Range<usize>,
}

impl Schedule {
    fn queue_untried_failures(&mut self, cache: &CacheState, limit: usize) -> bool {
        let before = cache
            .errors
            .range(..self.retried_range.start)
            .rev()
            .map(|(&index, _)| index)
            .filter(|index| !self.loading.contains(index))
            .take(limit);
        let after = cache
            .errors
            .range(self.retried_range.end..)
            .map(|(&index, _)| index)
            .filter(|index| !self.loading.contains(index))
            .take(limit);
        let previous = self.queue.len();
        self.queue.extend(before.chain(after));
        self.queue.len() > previous
    }
}

struct Loader {
    lane: PageLane,
    cache: PageCache,
    route: ProjectTicket,
    source: Option<Arc<PackedSource>>,
    generation: watch::Sender<u64>,
    schedule: Mutex<Schedule>,
}

pub struct PageLoader(Arc<Loader>);

impl PageLoader {
    pub(crate) fn new(
        lane: PageLane,
        cache: PageCache,
        route: ProjectTicket,
        source: Option<Arc<PackedSource>>,
    ) -> Self {
        cache.0.state.lock().unwrap().active = false;
        let inner = Arc::new(Loader {
            lane: lane.clone(),
            cache,
            route,
            source,
            generation: watch::channel(0).0,
            schedule: Mutex::new(Schedule {
                active: false,
                disposed: false,
                grid_viewport: None,
                queue: Vec::new(),
                loading: BTreeSet::new(),
                retried_range: 0..0,
            }),
        });
        lane.0.lock().unwrap().current = Arc::downgrade(&inner);
        Self(inner)
    }

    pub fn generation(&self) -> u64 {
        *self.0.generation.borrow()
    }
    pub fn visible_rows(&self) -> Range<usize> {
        self.0.cache.0.state.lock().unwrap().viewport.clone()
    }
    pub fn pending(&self) -> (usize, usize) {
        let lane = self.0.lane.0.lock().unwrap();
        (lane.active, self.0.schedule.lock().unwrap().queue.len())
    }
    pub fn viewport(&self, rows: Range<usize>) {
        let mut lane = self.0.lane.0.lock().unwrap();
        let mut s = self.0.schedule.lock().unwrap();
        if s.disposed || s.grid_viewport.is_some() {
            return;
        }
        let mut cache = self.0.cache.0.state.lock().unwrap();
        let rows = rows.start.min(self.0.cache.0.total)..rows.end.min(self.0.cache.0.total);
        if cache.viewport != rows {
            let anchor = rows.start / self.0.cache.0.size;
            s.retried_range = anchor..anchor;
            cache.viewport = rows;
        } else if !s.queue.is_empty() || !s.loading.is_empty() {
            return;
        }
        drop(cache);
        self.0.queue(&mut s, lane.active);
        drop(s);
        pump(&mut lane);
    }
    pub fn suspend(&self) {
        self.0.stop(false, None);
    }
    pub fn dispose(&self) {
        self.0.stop(true, None);
    }

    pub fn retry(&self) {
        let mut lane = self.0.lane.0.lock().unwrap();
        let mut s = self.0.schedule.lock().unwrap();
        if !s.active || s.disposed || self.0.source.is_none() {
            return;
        }
        let cache = self.0.cache.0.state.lock().unwrap();
        if s.grid_viewport.is_some() {
            if cache.errors.contains_key(&0) && !s.loading.contains(&0) && !s.queue.contains(&0) {
                s.queue.push(0);
            }
            drop(cache);
            drop(s);
            pump(&mut lane);
            return;
        }
        let anchor = cache.viewport.start / self.0.cache.0.size;
        let limit = MAX_QUEUED + MAX_CONCURRENT.saturating_sub(lane.active);
        if !s.queue_untried_failures(&cache, limit) && !s.retried_range.is_empty() {
            s.retried_range = anchor..anchor;
            s.queue_untried_failures(&cache, limit);
        }
        drop(cache);
        s.queue.sort_unstable_by_key(|p| (p.abs_diff(anchor), *p));
        s.queue.dedup();
        s.queue.truncate(limit);
        drop(s);
        pump(&mut lane);
    }
}

impl Drop for PageLoader {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl Loader {
    fn stop(&self, dispose: bool, expected: Option<u64>) {
        let mut s = self.schedule.lock().unwrap();
        if s.disposed
            || (!s.active && !dispose)
            || expected.is_some_and(|epoch| epoch != *self.generation.borrow())
        {
            return;
        }
        s.active = false;
        s.disposed = dispose;
        s.queue.clear();
        let mut cache = self.cache.0.state.lock().unwrap();
        cache.active = false;
        if let Some(viewport) = s.grid_viewport.take() {
            cache.viewport = viewport;
        }
        let anchor = cache.viewport.start / self.cache.0.size;
        s.retried_range = anchor..anchor;
        let mut interrupted_retry = false;
        for index in &s.loading {
            if let Some(failure) = cache.errors.get_mut(index) {
                failure.retrying = false;
                interrupted_retry = true;
            }
        }
        s.loading.clear();
        if interrupted_retry {
            self.cache.0.notify();
        }
        self.generation.send_modify(|n| *n += 1);
    }

    fn queue(&self, s: &mut Schedule, active: usize) {
        s.queue.clear();
        if !s.active || self.source.is_none() || !self.route.is_current() {
            return;
        }
        let data = &self.cache.0;
        let cache = data.state.lock().unwrap();
        if s.grid_viewport.is_some() || data.columns.get().is_none() || data.total == 0 {
            if !cache.pages.contains_key(&0)
                && !cache.errors.contains_key(&0)
                && !s.loading.contains(&0)
            {
                s.queue.push(0);
            }
            return;
        }
        if cache.viewport.is_empty() {
            return;
        }
        let start = cache.viewport.start / data.size;
        let end = (cache.viewport.end - 1) / data.size;
        let count = data.total.div_ceil(data.size);
        let visible = start..=end;
        let before = start.checked_sub(1);
        let after = end.saturating_add(1)..end.saturating_add(4).min(count);
        let limit = MAX_QUEUED + MAX_CONCURRENT.saturating_sub(active);
        s.queue.extend(
            visible
                .chain(before)
                .chain(after)
                .filter(|p| {
                    !s.loading.contains(p)
                        && !cache.pages.contains_key(p)
                        && !cache.errors.contains_key(p)
                })
                .take(limit),
        );
    }

    async fn read(
        &self,
        index: usize,
        source: &Arc<PackedSource>,
        mut generation: watch::Receiver<u64>,
        mut stop: watch::Receiver<bool>,
    ) -> Result<LoadedPage, PageError> {
        let offset = index
            .checked_mul(self.cache.0.size)
            .filter(|&i| i < self.cache.0.total || (i == 0 && self.cache.0.total == 0))
            .ok_or(PageError::Shape)?;
        let expected = self.cache.0.size.min(self.cache.0.total - offset);
        let cache = self.cache.clone();
        let packed = tokio::select! { biased;
            _ = generation.changed() => return Err(PageError::Cancelled),
            _ = stop.wait_for(|s| *s) => return Err(PageError::Cancelled),
            r = source.fetch_packed(offset) => r?,
        };
        let budget = source.service().budget().clone();
        let route = self.route.clone();
        let token = generation.clone();
        let stopped = stop.clone();
        let decoding_source = source.clone();
        let page = tokio::task::spawn_blocking(move || {
            let mut current = || {
                matches!(token.has_changed(), Ok(false))
                    && !*stopped.borrow()
                    && route.is_current()
                    && decoding_source.is_open()
            };
            let columns = match cache.0.columns.get() {
                Some(_) => None,
                None => Some(decode_columns(
                    cache.packed_header().ok_or(PageError::Shape)?,
                    &budget,
                    &mut current,
                )?),
            };
            let width = columns
                .as_ref()
                .map_or_else(|| cache.columns().len(), |names| names.len());
            let rows = decode_page(&packed, expected, width, &budget, &mut current)?;
            let labels = columns.as_ref().map_or_else(|| cache.columns(), |c| &c[..]);
            let target = cache.prepare_target(index, labels, &rows, &mut current);
            Ok::<_, PageError>(LoadedPage {
                columns,
                rows,
                target,
            })
        })
        .await
        .map_err(|e| PageError::Unavailable(e.to_string()))??;
        let available = tokio::select! { biased;
            _ = generation.changed() => return Err(PageError::Cancelled),
            _ = stop.wait_for(|s| *s) => return Err(PageError::Cancelled),
            available = source.available() => available,
        };
        if !available {
            return Err(PageError::Unavailable("This result has expired.".into()));
        }
        Ok(page)
    }
}

fn pump(lane: &mut LaneState) {
    let Some(loader) = lane.current.upgrade() else {
        return;
    };
    let Some(source) = &loader.source else { return };
    let service = source.service();
    let mut s = loader.schedule.lock().unwrap();
    while lane.active < MAX_CONCURRENT && !s.queue.is_empty() {
        let stop = service.stop.subscribe();
        if !s.active || !loader.route.is_current() || *stop.borrow() {
            s.queue.clear();
            break;
        }
        let index = s.queue.remove(0);
        s.loading.insert(index);
        let mut cache = loader.cache.0.state.lock().unwrap();
        if let Some(failure) = cache.errors.get_mut(&index) {
            failure.retrying = true;
            s.retried_range.start = s.retried_range.start.min(index);
            s.retried_range.end = s.retried_range.end.max(index + 1);
            loader.cache.0.notify();
        }
        drop(cache);
        lane.active += 1;
        let source = source.clone();
        let inner = loader.clone();
        let generation = inner.generation.subscribe();
        let epoch = *generation.borrow();
        service.runtime.spawn(async move {
            let mut result = retry_once(
                || *generation.borrow() == epoch && !*stop.borrow() && inner.route.is_current(),
                || inner.read(index, &source, generation.clone(), stop.clone()),
            )
            .await;
            let mut lane = inner.lane.0.lock().unwrap();
            let mut s = inner.schedule.lock().unwrap();
            if s.active
                && *generation.borrow() == epoch
                && !*stop.borrow()
                && inner.route.is_current()
            {
                s.loading.remove(&index);
                if !source.is_open() {
                    result = Err(PageError::Unavailable("This result has expired.".into()));
                }
                let recovered_header = result.as_ref().is_ok_and(|page| page.columns.is_some());
                inner.cache.publish(index, result);
                if recovered_header {
                    inner.queue(&mut s, lane.active - 1);
                }
            }
            lane.active -= 1;
            drop(s);
            pump(&mut lane);
        });
    }
}

async fn retry_once<T, F: Future<Output = Result<T, PageError>>>(
    current: impl Fn() -> bool,
    mut read: impl FnMut() -> F,
) -> Result<T, PageError> {
    let result = read().await;
    if result.is_err() && current() {
        read().await
    } else {
        result
    }
}
