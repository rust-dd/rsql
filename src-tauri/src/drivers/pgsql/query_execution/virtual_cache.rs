use futures_util::{TryStreamExt, pin_mut};
use std::sync::Arc;
use std::time::Instant;
use tokio_postgres::{Client, SimpleQueryMessage};

use crate::common::enums::{AppError, query_failed};

use super::super::VirtualCache;
use super::super::result_memory::{CachedQuery, MemoryBudget, PackedPage, evict_idle};
use super::super::wire::{ROW_SEP, pack_columns};
use super::helpers::column_names;

const MAX_VIRTUAL_ROWS: usize = 1_000_000;

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
) -> Result<(String, usize, String, f32, bool), AppError> {
    let start = Instant::now();
    if page_size == 0 {
        return Err(AppError::QueryFailed("Page size must be positive".into()));
    }
    evict_idle(cache).await;
    let budget = Arc::clone(&cache.lock().await.budget);
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
            }
            SimpleQueryMessage::Row(row) => {
                if !accum.push((0..row.len()).map(|i| row.get(i)), page_size, &budget) {
                    capped = true;
                    break;
                }
            }
            SimpleQueryMessage::CommandComplete(n) => {
                if !accum.has_rowset() {
                    total_affected += n;
                }
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
    let old = cache.lock().await.entries.insert(
        query_id.to_string(),
        CachedQuery {
            pages: accum.pages,
            page_size,
            accessed: Instant::now(),
        },
    );
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
) -> Result<String, AppError> {
    let page = {
        let mut cache = cache.lock().await;
        let entry = cache.entries.get_mut(query_id).ok_or_else(|| {
            AppError::QueryFailed(
                "This result has expired. Run the query again to reload it.".into(),
            )
        })?;
        entry.accessed = Instant::now();
        entry.pages.get(offset / entry.page_size).cloned()
    };
    Ok(page.map(|page| page.data.clone()).unwrap_or_default())
}

pub async fn close_virtual(
    cache: &tokio::sync::Mutex<VirtualCache>,
    query_id: &str,
) -> Result<(), AppError> {
    let removed = cache.lock().await.entries.remove(query_id);
    drop(removed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::wire::CELL_SEP;
    use super::*;

    fn accumulate(rows: &[Vec<Option<&str>>], page_size: usize) -> PageAccumulator {
        let cache = VirtualCache::new();
        let mut accum = PageAccumulator::new(Arc::clone(&cache.budget));
        accum.columns = vec!["a".into()];
        for row in rows {
            assert!(accum.push(row.iter().copied(), page_size, &cache.budget));
        }
        accum.flush(&cache.budget);
        accum
    }

    #[test]
    fn rows_are_split_into_pages_of_the_requested_size() {
        let rows = (0..5).map(|_| vec![Some("x")]).collect::<Vec<_>>();
        let accum = accumulate(&rows, 2);
        assert_eq!(accum.pages.len(), 3);
        assert_eq!(accum.total_rows, 5);
    }

    #[test]
    fn a_partial_final_page_is_kept() {
        let rows = (0..3).map(|_| vec![Some("x")]).collect::<Vec<_>>();
        let accum = accumulate(&rows, 2);
        assert_eq!(accum.pages.len(), 2);
        assert_eq!(accum.pages[1].data.split(ROW_SEP).count(), 1);
    }

    #[test]
    fn an_exactly_full_page_produces_no_trailing_empty_page() {
        let rows = (0..4).map(|_| vec![Some("x")]).collect::<Vec<_>>();
        let accum = accumulate(&rows, 2);
        assert_eq!(accum.pages.len(), 2);
    }

    #[test]
    fn nulls_survive_page_packing() {
        let accum = accumulate(&[vec![None], vec![Some("null")]], 10);
        let page = &accum.pages[0].data;
        let mut rows = page.split(ROW_SEP);
        assert_eq!(rows.next().unwrap(), "\u{1D}N");
        assert_eq!(rows.next().unwrap(), "null");
    }

    #[test]
    fn separators_in_data_do_not_break_page_boundaries() {
        let accum = accumulate(&[vec![Some("a\u{1E}b")], vec![Some("c")]], 10);
        assert_eq!(accum.pages[0].data.split(ROW_SEP).count(), 2);
        assert_eq!(accum.pages[0].data.split(CELL_SEP).count(), 1);
    }

    #[test]
    fn an_empty_accumulator_has_no_rowset() {
        assert!(!PageAccumulator::new(VirtualCache::new().budget).has_rowset());
    }
}
