use super::edits::{EditTarget, OriginRecipe};
use super::{Columns, DecodedBudget, DecodedPage, PackedSource, PageError};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::watch;

pub const MAX_CACHED: usize = 24;

pub enum CellState {
    Loading,
    Value(CellValue),
    Error(Arc<PageError>),
}

pub struct CellValue {
    page: Arc<DecodedPage>,
    row: usize,
    column: usize,
}

impl CellValue {
    pub fn raw(&self) -> &rsql_core::wire::Cell {
        &self.page[self.row][self.column]
    }
}

pub(super) struct Failure {
    error: Arc<PageError>,
    pub retrying: bool,
}

impl From<PageError> for Failure {
    fn from(error: PageError) -> Self {
        Self {
            error: Arc::new(error),
            retrying: false,
        }
    }
}

pub(super) struct CacheState {
    target: Option<Arc<EditTarget>>,
    pub pages: BTreeMap<usize, Arc<DecodedPage>>,
    pub errors: BTreeMap<usize, Failure>,
    pub viewport: Range<usize>,
    pub active: bool,
}

impl CacheState {
    fn remove_page(&mut self, index: usize) {
        self.pages.remove(&index);
        if index == 0 {
            self.target = None;
        }
    }
    fn error(&self, index: usize) -> Option<Arc<PageError>> {
        self.errors
            .get(&index)
            .filter(|failure| !failure.retrying)
            .map(|failure| failure.error.clone())
    }
}

pub(crate) struct LoadedPage {
    pub columns: Option<Arc<Columns>>,
    pub rows: Arc<DecodedPage>,
    pub target: Option<Arc<EditTarget>>,
}

pub(super) struct CacheStorage {
    pub state: Mutex<CacheState>,
    pub size: usize,
    pub total: usize,
    origin: OnceLock<OriginRecipe>,
    pub columns: OnceLock<Arc<Columns>>,
    pub packed_header: Option<String>,
    source: OnceLock<Weak<PackedSource>>,
    changed: watch::Sender<u64>,
}

impl CacheStorage {
    fn rank(&self, s: &CacheState, page: usize) -> (bool, usize) {
        let start = s.viewport.start / self.size;
        let end = s.viewport.end.saturating_sub(1) / self.size;
        (
            !s.active,
            start.saturating_sub(page).max(page.saturating_sub(end)),
        )
    }
    pub(super) fn victim(&self) -> Option<((bool, usize), usize)> {
        if !self.source.get()?.upgrade()?.can_evict() {
            return None;
        }
        let state = self.state.lock().unwrap();
        state
            .pages
            .iter()
            .filter(|(_, page)| Arc::strong_count(page) == 1)
            .map(|(&p, _)| (self.rank(&state, p), p))
            .filter(|((inactive, distance), _)| *inactive || *distance > 0)
            .max()
    }
    pub(super) fn evict(&self, page: usize) {
        let Some(source) = self.source.get().and_then(Weak::upgrade) else {
            return;
        };
        if !source.can_evict() {
            return;
        }
        let mut state = self.state.lock().unwrap();
        let (inactive, distance) = self.rank(&state, page);
        if (inactive || distance > 0)
            && state
                .pages
                .get(&page)
                .is_some_and(|p| Arc::strong_count(p) == 1)
        {
            state.remove_page(page);
            self.notify();
        }
    }
    pub(super) fn notify(&self) {
        self.changed
            .send_modify(|version| *version = version.wrapping_add(1));
    }
}

#[derive(Clone)]
pub struct PageCache(pub(super) Arc<CacheStorage>);

impl PageCache {
    pub(crate) fn new(
        budget: &DecodedBudget,
        size: usize,
        total: usize,
        columns: Option<Arc<Columns>>,
        packed_header: Option<String>,
        first: Result<Arc<DecodedPage>, PageError>,
    ) -> Self {
        let mut pages = BTreeMap::new();
        let mut errors = BTreeMap::new();
        match first {
            Ok(page) => {
                pages.insert(0, page);
            }
            Err(error) => {
                errors.insert(0, error.into());
            }
        }
        let storage = Arc::new(CacheStorage {
            origin: OnceLock::new(),
            size,
            total,
            columns: columns.map(OnceLock::from).unwrap_or_default(),
            packed_header,
            source: OnceLock::new(),
            changed: watch::channel(0).0,
            state: Mutex::new(CacheState {
                target: None,
                pages,
                errors,
                active: true,
                viewport: 0..total.min(1),
            }),
        });
        budget.register(&storage);
        Self(storage)
    }

    pub(crate) fn attach_source(&self, source: &Arc<PackedSource>) {
        let _ = self.0.source.set(Arc::downgrade(source));
    }

    pub(crate) fn columns(&self) -> &[String] {
        self.0.columns.get().map_or(&[], |columns| &columns[..])
    }

    pub(crate) fn packed_header(&self) -> Option<&str> {
        self.0.packed_header.as_deref()
    }

    pub fn page(&self, index: usize) -> Option<Arc<DecodedPage>> {
        self.0.state.lock().unwrap().pages.get(&index).cloned()
    }
    pub(crate) fn retain_origin(&self, recipe: OriginRecipe, current: impl FnMut() -> bool) {
        let _ = self.0.origin.set(recipe);
        if let Some(rows) = self.page(0) {
            let target = self.prepare_target(0, self.columns(), &rows, current);
            self.publish(
                0,
                Ok(LoadedPage {
                    columns: None,
                    rows,
                    target,
                }),
            );
        }
    }
    pub(super) fn prepare_target(
        &self,
        index: usize,
        labels: &[String],
        page: &Arc<DecodedPage>,
        current: impl FnMut() -> bool,
    ) -> Option<Arc<EditTarget>> {
        if index != 0 {
            return None;
        }
        EditTarget::prepare(self.0.origin.get()?, labels, page, current).map(Arc::new)
    }
    pub fn target(&self) -> Option<Arc<EditTarget>> {
        self.0.state.lock().unwrap().target.clone()
    }
    pub fn foreign_target(&self, row: usize, col: usize) -> Option<Arc<EditTarget>> {
        let state = self.0.state.lock().unwrap();
        let target = state.target.as_ref()?;
        if target.has_foreign_key(col) && state.pages.get(&0)?.get(row)?.get(col)?.is_some() {
            Some(target.clone())
        } else {
            None
        }
    }
    pub fn cached_pages(&self) -> Vec<usize> {
        self.0.state.lock().unwrap().pages.keys().copied().collect()
    }
    pub fn error(&self, index: usize) -> Option<Arc<PageError>> {
        self.0.state.lock().unwrap().error(index)
    }
    pub fn first_error(&self) -> Option<Arc<PageError>> {
        let state = self.0.state.lock().unwrap();
        state
            .errors
            .values()
            .find(|failure| !failure.retrying)
            .map(|failure| failure.error.clone())
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.changed.subscribe()
    }

    pub fn cell(&self, row: usize, column: usize) -> CellState {
        let state = self.0.state.lock().unwrap();
        let Some(columns) = self.0.columns.get() else {
            return state.error(0).map_or(CellState::Loading, CellState::Error);
        };
        if row >= self.0.total || column >= columns.len() {
            return CellState::Error(Arc::new(PageError::Shape));
        }
        let index = row / self.0.size;
        if let Some(error) = state.error(index) {
            return CellState::Error(error);
        }
        match state.pages.get(&index) {
            Some(page) => CellState::Value(CellValue {
                page: page.clone(),
                row: row % self.0.size,
                column,
            }),
            None => CellState::Loading,
        }
    }
    pub(crate) fn publish(&self, index: usize, result: Result<LoadedPage, PageError>) {
        let mut state = self.0.state.lock().unwrap();
        match result {
            Ok(page) => {
                if let Some(columns) = page.columns {
                    let _ = self.0.columns.set(columns);
                }
                state.errors.remove(&index);
                if index == 0 {
                    state.target = page.target;
                }
                state.pages.insert(index, page.rows);
                if state.pages.len() > MAX_CACHED {
                    let far = state
                        .pages
                        .keys()
                        .copied()
                        .max_by_key(|&p| {
                            (
                                self.0.rank(&state, p),
                                p.abs_diff(state.viewport.start / self.0.size),
                            )
                        })
                        .unwrap();
                    state.remove_page(far);
                }
            }
            Err(error) => {
                state.remove_page(index);
                state.errors.insert(index, error.into());
            }
        }
        self.0.notify();
    }
}
