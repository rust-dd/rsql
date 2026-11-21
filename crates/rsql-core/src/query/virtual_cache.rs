use futures_util::{TryStreamExt, pin_mut};
use std::sync::Arc;
use std::time::Instant;
use tokio_postgres::{Client, SimpleQueryMessage};

use crate::error::{Error, query_failed};

use super::VirtualCache;
use super::helpers::column_names;
use super::memory::{CachedQuery, MemoryBudget, PackedPage, evict_idle};
use crate::wire::{ROW_SEP, pack_columns};

pub const MAX_VIRTUAL_ROWS: usize = 1_000_000;

struct PageAccumulator {
    columns: Vec<String>,
    pages: Vec<Arc<PackedPage>>,
    current: PackedPage,
    rows_in_page: usize,
    total_rows: usize,
}

impl PageAccumulator {
    fn new(budget: Arc<MemoryBudget>) -> Self {
        Self {
            columns: Vec::new(),
            pages: Vec::new(),
            current: PackedPage::new(budget),
            rows_in_page: 0,
            total_rows: 0,
        }
    }

    fn push<'a>(
        &mut self,
        cells: impl Iterator<Item = Option<&'a str>> + Clone,
        page_size: usize,
        budget: &Arc<MemoryBudget>,
    ) -> bool {
        if self.total_rows >= MAX_VIRTUAL_ROWS {
            return false;
        }
        if self.rows_in_page == page_size {
            self.flush(budget);
        }
        if !self.current.push(cells, self.rows_in_page > 0) {
            return false;
        }
        self.rows_in_page += 1;
        self.total_rows += 1;
        true
    }

    fn flush(&mut self, budget: &Arc<MemoryBudget>) {
        if self.rows_in_page > 0 {
            let page = std::mem::replace(&mut self.current, PackedPage::new(Arc::clone(budget)));
            self.pages.push(Arc::new(page));
            self.rows_in_page = 0;
        }
    }

    fn has_rowset(&self) -> bool {
        !self.columns.is_empty()
    }
}

/// Retains the last rowset within one budget shared by completed and running queries.
/// The limit bounds retained data; it does not impose a server-side row limit.
pub async fn execute_virtual(
    client: &Client,
    cache: &tokio::sync::Mutex<VirtualCache>,
    sql: &str,
    query_id: &str,
    page_size: usize,
) -> Result<(String, usize, String, f32, bool), Error> {
    let start = Instant::now();
    if page_size == 0 {
        return Err(Error::QueryFailed("Page size must be positive".into()));
    }
    evict_idle(cache).await;
    let budget = {
        let cache = cache.lock().await;
        cache.ensure_open()?;
        Arc::clone(&cache.budget)
    };
    let stream = client.simple_query_raw(sql).await.map_err(query_failed)?;
    pin_mut!(stream);
    let mut accum = PageAccumulator::new(Arc::clone(&budget));
    let mut total_affected = 0u64;
    let mut capped = false;

    while let Some(message) = stream.try_next().await.map_err(query_failed)? {
        match message {
            SimpleQueryMessage::RowDescription(columns) => {
                accum = PageAccumulator::new(Arc::clone(&budget));
                accum.columns = column_names(&columns);
                capped = false;
            }
            SimpleQueryMessage::Row(row)
                if !capped
                    && !accum.push((0..row.len()).map(|i| row.get(i)), page_size, &budget) =>
            {
                capped = true;
            }
            SimpleQueryMessage::CommandComplete(n) if !accum.has_rowset() => {
                total_affected += n;
            }
            _ => {}
        }
    }
    accum.flush(&budget);
    let elapsed = start.elapsed().as_millis() as f32;
    if !accum.has_rowset() {
        let fallback = if total_affected > 0 {
            format!("Result{ROW_SEP}{total_affected} rows affected")
        } else {
            String::new()
        };
        return Ok((String::new(), 0, fallback, elapsed, false));
    }
    let columns_packed = pack_columns(&accum.columns);
    let first_page_packed = accum
        .pages
        .first()
        .map(|page| page.data.clone())
        .unwrap_or_default();
    let total_rows = accum.total_rows;
    let old = cache.lock().await.publish(
        query_id.to_string(),
        CachedQuery {
            pages: accum.pages,
            page_size,
            accessed: Instant::now(),
        },
    )?;
    drop(old);
    Ok((
        columns_packed,
        total_rows,
        first_page_packed,
        elapsed,
        capped,
    ))
}

pub async fn fetch_virtual_page(
    cache: &tokio::sync::Mutex<VirtualCache>,
    query_id: &str,
    _col_count: usize,
    offset: usize,
    _limit: usize,
) -> Result<String, Error> {
    Ok(shared_virtual_page(cache, query_id, offset)
        .await?
        .map(|p| p.as_ref().to_owned())
        .unwrap_or_default())
}

/// The backend reservation stays alive while a consumer reads this immutable page.
pub struct SharedPage(Arc<PackedPage>);
impl AsRef<str> for SharedPage {
    fn as_ref(&self) -> &str {
        &self.0.data
    }
}

pub async fn shared_virtual_page(
    cache: &tokio::sync::Mutex<VirtualCache>,
    query_id: &str,
    offset: usize,
) -> Result<Option<SharedPage>, Error> {
    let page = {
        let mut cache = cache.lock().await;
        cache.ensure_open()?;
        let live = cache.contains_live(query_id);
        let entry = cache
            .entries
            .get_mut(query_id)
            .filter(|_| live)
            .ok_or_else(|| {
                Error::QueryFailed(
                    "This result has expired. Run the query again to reload it.".into(),
                )
            })?;
        entry.accessed = Instant::now();
        entry.pages.get(offset / entry.page_size).cloned()
    };
    Ok(page.map(SharedPage))
}

pub async fn close_virtual(
    cache: &tokio::sync::Mutex<VirtualCache>,
    query_id: &str,
) -> Result<(), Error> {
    let removed = cache.lock().await.entries.remove(query_id);
    drop(removed);
    Ok(())
}
