use super::{GeometryError, Resource};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_RETAINED: usize = 64 * 1024 * 1024;
const MAX_PARSER: usize = 192 * 1024 * 1024;
static APPLICATION: Counters = Counters::new(MAX_RETAINED, MAX_PARSER);

#[derive(Clone, Copy, Debug)]
pub(super) enum Pool {
    Retained,
    Parser,
}

impl Pool {
    fn error(self) -> GeometryError {
        GeometryError::Resource(match self {
            Self::Retained => Resource::RetainedBytes,
            Self::Parser => Resource::ParserBytes,
        })
    }
}

#[derive(Debug)]
struct Counter {
    used: AtomicUsize,
    limit: usize,
}

impl Counter {
    const fn new(limit: usize) -> Self {
        Self {
            used: AtomicUsize::new(0),
            limit,
        }
    }

    fn reserve(&self, bytes: usize) -> bool {
        self.used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.limit)
            })
            .is_ok()
    }

    fn release(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct Counters {
    retained: Counter,
    parser: Counter,
}

impl Counters {
    const fn new(retained: usize, parser: usize) -> Self {
        Self {
            retained: Counter::new(retained),
            parser: Counter::new(parser),
        }
    }

    fn pool(&self, pool: Pool) -> &Counter {
        match pool {
            Pool::Retained => &self.retained,
            Pool::Parser => &self.parser,
        }
    }
}

/// Clones share a scope. All scopes also draw from the same application limits.
#[derive(Clone, Debug)]
pub struct GeometryBudget(Arc<Counters>);

impl Default for GeometryBudget {
    fn default() -> Self {
        Self::with_limits(MAX_RETAINED, MAX_PARSER)
    }
}

impl GeometryBudget {
    pub fn with_limits(retained_bytes: usize, parser_bytes: usize) -> Self {
        Self(Arc::new(Counters::new(retained_bytes, parser_bytes)))
    }

    pub fn retained_bytes(&self) -> usize {
        self.0.retained.used.load(Ordering::Relaxed)
    }

    pub fn parser_bytes(&self) -> usize {
        self.0.parser.used.load(Ordering::Relaxed)
    }

    pub(super) fn reserve(&self, pool: Pool, bytes: usize) -> Result<Reservation, GeometryError> {
        if !APPLICATION.pool(pool).reserve(bytes) {
            return Err(pool.error());
        }
        if !self.0.pool(pool).reserve(bytes) {
            APPLICATION.pool(pool).release(bytes);
            return Err(pool.error());
        }
        Ok(Reservation {
            budget: self.clone(),
            pool,
            bytes,
        })
    }
}

#[derive(Debug)]
pub(super) struct Reservation {
    budget: GeometryBudget,
    pool: Pool,
    bytes: usize,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.0.pool(self.pool).release(self.bytes);
        APPLICATION.pool(self.pool).release(self.bytes);
    }
}
