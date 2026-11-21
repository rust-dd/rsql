use super::pages::CacheStorage;
use rsql_core::wire::{self, Cell};
use std::{
    fmt,
    ops::Deref,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

pub const DISPLAY_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageError {
    Memory,
    Shape,
    Cancelled,
    Unavailable(String),
}

impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory => f.write_str("Result exceeds the available display memory."),
            Self::Shape => f.write_str("The retained page has an unexpected row or column count."),
            Self::Cancelled => f.write_str("Result page cancelled."),
            Self::Unavailable(message) => f.write_str(message),
        }
    }
}

#[derive(Clone)]
pub struct DecodedBudget(Arc<Budget>);

struct Budget {
    limit: usize,
    used: AtomicUsize,
    caches: Mutex<Vec<Weak<CacheStorage>>>,
}

impl Default for DecodedBudget {
    fn default() -> Self {
        Self::new(DISPLAY_BYTES)
    }
}

impl DecodedBudget {
    pub fn new(limit: usize) -> Self {
        Self(Arc::new(Budget {
            limit,
            used: AtomicUsize::new(0),
            caches: Mutex::default(),
        }))
    }

    pub fn used(&self) -> usize {
        self.0.used.load(Ordering::Acquire)
    }

    pub(super) fn register(&self, cache: &Arc<CacheStorage>) {
        let mut caches = self.0.caches.lock().unwrap();
        caches.retain(|w| w.strong_count() > 0);
        caches.push(Arc::downgrade(cache));
    }

    fn reserve(
        &self,
        bytes: usize,
        current: &mut impl FnMut() -> bool,
    ) -> Result<Reservation, PageError> {
        if bytes > self.0.limit {
            return Err(PageError::Memory);
        }
        loop {
            if !current() {
                return Err(PageError::Cancelled);
            }
            if self
                .0
                .used
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    used.checked_add(bytes).filter(|next| *next <= self.0.limit)
                })
                .is_ok()
            {
                return Ok(Reservation {
                    budget: self.clone(),
                    bytes,
                });
            }
            let caches = self.0.caches.lock().unwrap();
            let mut candidate = None;
            for cache in caches.iter().filter_map(Weak::upgrade) {
                if !current() {
                    return Err(PageError::Cancelled);
                }
                if let Some((rank, page)) = cache.victim()
                    && candidate.as_ref().is_none_or(|(best, _, _)| rank > *best)
                {
                    candidate = Some((rank, page, cache));
                }
            }
            let Some((_, index, cache)) = candidate else {
                return Err(PageError::Memory);
            };
            cache.evict(index);
        }
    }
}

struct Reservation {
    budget: DecodedBudget,
    bytes: usize,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub struct Decoded<T> {
    values: Vec<T>,
    _reservation: Reservation,
}

impl<T> Deref for Decoded<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.values
    }
}

pub type DecodedPage = Decoded<Vec<Cell>>;
pub type Columns = Decoded<String>;

fn allocation<T>(count: usize, text: usize) -> Result<usize, PageError> {
    count
        .checked_mul(size_of::<T>())
        .and_then(|n| n.checked_add(text))
        .ok_or(PageError::Memory)
}

fn reserve_buffer<T>(
    budget: &DecodedBudget,
    buffer_bytes: usize,
    current: &mut impl FnMut() -> bool,
) -> Result<Reservation, PageError> {
    let bytes = buffer_bytes
        .checked_add(size_of::<Decoded<T>>() + 2 * size_of::<usize>())
        .ok_or(PageError::Memory)?;
    budget.reserve(bytes, current)
}

fn capacity(raw: &str) -> usize {
    match raw {
        "\x1dN" | "\x1dE" => 0,
        _ => raw.len(),
    }
}

pub(crate) fn decode_columns(
    raw: &str,
    budget: &DecodedBudget,
    current: &mut impl FnMut() -> bool,
) -> Result<Arc<Columns>, PageError> {
    let mut count = 0;
    let mut bytes = 0usize;
    wire::visit_cells_checked(raw, false, current, |cell, _, _| {
        count += 1;
        bytes += capacity(cell);
        Some(())
    })
    .ok_or(PageError::Cancelled)?;
    let reservation =
        reserve_buffer::<String>(budget, allocation::<String>(count, bytes)?, current)?;
    let mut values = Vec::with_capacity(count);
    wire::visit_cells_checked(raw, false, current, |c, _, current| {
        values.push(wire::unpack_cell_checked(c, current)?.unwrap_or_default());
        Some(())
    })
    .ok_or(PageError::Cancelled)?;
    Ok(Arc::new(Decoded {
        values,
        _reservation: reservation,
    }))
}

pub fn decode_page(
    packed: &str,
    rows: usize,
    columns: usize,
    budget: &DecodedBudget,
    mut current: impl FnMut() -> bool,
) -> Result<Arc<DecodedPage>, PageError> {
    let mut width = 0;
    let mut height = 0;
    let mut text = 0;
    let mut valid = true;
    wire::visit_cells_checked(packed, true, &mut current, |raw, end, _| {
        width += 1;
        text += capacity(raw);
        if end {
            valid &= width == columns;
            width = 0;
            height += 1;
        }
        Some(())
    })
    .ok_or(PageError::Cancelled)?;
    if !valid || height != rows {
        return Err(PageError::Shape);
    }
    let cells = rows.checked_mul(columns).ok_or(PageError::Memory)?;
    let bytes = allocation::<Vec<Cell>>(rows, allocation::<Cell>(cells, text)?)?;
    let reservation = reserve_buffer::<Vec<Cell>>(budget, bytes, &mut current)?;
    let mut values = Vec::with_capacity(rows);
    let mut row = Vec::new();
    wire::visit_cells_checked(packed, true, &mut current, |raw, end, current| {
        if row.is_empty() {
            row.reserve_exact(columns);
        }
        row.push(wire::unpack_cell_checked(raw, current)?);
        if end {
            values.push(std::mem::take(&mut row));
        }
        Some(())
    })
    .ok_or(PageError::Cancelled)?;
    Ok(Arc::new(Decoded {
        values,
        _reservation: reservation,
    }))
}
