mod retained;
mod runner;

pub use retained::RetainedResult;
pub use runner::Runner;

use crate::results::{
    DecodedBudget, PackedSource, PageCache, PageError, decode_columns, decode_page,
};
use crate::{connection::ConnectionProfile, profile_service::Challenge};
use rsql_core::wire;
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Query,
    Explain,
    Analyze,
}

pub const TIMEOUTS_MS: &[Option<u32>] = &[
    None,
    Some(5_000),
    Some(10_000),
    Some(30_000),
    Some(60_000),
    Some(300_000),
    Some(600_000),
];

pub fn page_size(raw: Option<&str>) -> usize {
    raw.and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n >= 100.)
        .map_or(2_000, |n| n.floor() as usize)
}

pub struct RunSpec {
    pub generated: Option<Arc<crate::results::ForeignQuery>>,
    pub profile: ConnectionProfile,
    pub sql: Arc<str>,
    pub mode: Mode,
    pub timeout_ms: Option<u32>,
    pub page_size: usize,
    pub row_tools: bool,
    execution_id: String,

    connected: Option<tokio::sync::oneshot::Sender<rsql_core::services::ProjectTicket>>,
    guard: Option<rsql_core::services::ProjectTicket>,
}

impl RunSpec {
    pub fn new(
        profile: ConnectionProfile,
        sql: impl Into<Arc<str>>,
        mode: Mode,
        timeout_ms: Option<u32>,
    ) -> Self {
        Self {
            profile,
            generated: None,
            sql: sql.into(),
            mode,
            timeout_ms,
            page_size: page_size(std::env::var("RSQL_PAGE_SIZE").ok().as_deref()),
            row_tools: true,
            execution_id: Uuid::new_v4().to_string(),

            guard: None,
            connected: None,
        }
    }
    /// Binds deferred editor targeting to its original connection lifetime.
    pub fn while_current(mut self, guard: rsql_core::services::ProjectTicket) -> Self {
        self.guard = Some(guard);
        self
    }

    pub fn on_connected(
        mut self,
        sender: tokio::sync::oneshot::Sender<rsql_core::services::ProjectTicket>,
    ) -> Self {
        self.connected = Some(sender);
        self
    }

    pub fn eager(mut self) -> Self {
        self.page_size = rsql_core::query::MAX_VIRTUAL_ROWS;
        self
    }
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }
}

pub struct Outcome {
    pub generated: Option<Arc<crate::results::ForeignQuery>>,
    pub sql: Arc<str>,
    pub project_id: String,
    /// Dispatch-time connection lifetime for pane results and late catalog changes.
    pub route_ticket: Option<rsql_core::services::ProjectTicket>,
    pub database: String,
    pub mode: Mode,
    pub started: SystemTime,
    pub elapsed: Duration,
    pub may_change_catalog: bool,
    pub status: Status,
}

pub enum Status {
    Rows(Arc<QueryResult>),
    Plan(Box<crate::explain::PlanOutput>),
    /// The local owner cancelled or lost its connection lifetime.
    Cancelled,
    /// PostgreSQL reported cancellation. Its primary message may leave the cause ambiguous.
    ServerCancelled(String),
    TimedOut(String),
    Error(String),
    TrustRequired(Challenge),
}

pub struct QueryResult {
    pub sql: Arc<str>,
    pub id: Uuid,
    pub cache: PageCache,
    pub elapsed_ms: f32,
    pub total_rows: usize,
    pub page_size: usize,
    pub capped: bool,
    pub is_rowset: bool,
    source: Option<Arc<PackedSource>>,
}

impl QueryResult {
    fn retain_origin(
        &self,
        route: rsql_core::services::ProjectTicket,
        origin: rsql_core::postgres::provenance::ResultOrigin,
        generated: Option<Arc<crate::results::ForeignQuery>>,
        current: impl FnMut() -> bool,
    ) {
        if !self.capped && self.total_rows <= self.page_size {
            self.cache.retain_origin(
                crate::results::edits::OriginRecipe {
                    generated,
                    result: self.id,
                    sql: self.sql.clone(),
                    route,
                    origin,
                },
                current,
            );
        }
    }

    pub fn packed_source(&self) -> Option<&Arc<PackedSource>> {
        self.source.as_ref()
    }

    pub fn retained(&self) -> Option<&RetainedResult> {
        self.source.as_ref()?.retained()
    }

    fn attach_source(&mut self, source: PackedSource) {
        let source = Arc::new(source);
        self.cache.attach_source(&source);
        self.source = Some(source);
    }

    pub fn columns(&self) -> &[String] {
        self.cache.columns()
    }

    /// Original export labels retained when first-page header admission was refused.
    pub fn packed_header(&self) -> Option<&str> {
        self.cache.packed_header()
    }

    pub fn packed(
        sql: Arc<str>,
        page_size: usize,
        data: (String, usize, String, f32, bool),
        budget: &DecodedBudget,
        current: impl FnMut() -> bool,
    ) -> Option<Self> {
        Self::decode_packed(sql, page_size, data, budget, current).map(|(result, _)| result)
    }

    fn decode_packed(
        sql: Arc<str>,
        page_size: usize,
        data: (String, usize, String, f32, bool),
        budget: &DecodedBudget,
        mut current: impl FnMut() -> bool,
    ) -> Option<(Self, Option<String>)> {
        let (header, total, packed, elapsed_ms, capped) = data;
        let is_rowset = !header.is_empty();
        let (labels, body) = wire::page_parts_checked(&header, &packed, &mut current)?;
        let total_rows = if header.is_empty() {
            usize::from(!body.is_empty())
        } else {
            total
        };
        let columns = decode_columns(labels, budget, &mut current);
        let first = match &columns {
            Ok(names) => decode_page(
                body,
                total_rows.min(page_size),
                names.len(),
                budget,
                &mut current,
            ),
            Err(e) => Err(e.clone()),
        };
        if matches!(first, Err(PageError::Cancelled)) || !current() || page_size == 0 {
            return None;
        }
        let inline = header.is_empty() && first.is_err();
        let body_start = packed.len() - body.len();
        let packed_header = if columns.is_err() && header.is_empty() {
            Some(labels.to_owned())
        } else {
            columns.is_err().then_some(header)
        };
        let inline_body = inline.then(|| {
            let mut packed = packed;
            packed.drain(..body_start);
            packed
        });
        let cache = PageCache::new(
            budget,
            page_size,
            total_rows,
            columns.ok(),
            packed_header,
            first,
        );
        Some((
            Self {
                sql,
                id: Uuid::new_v4(),
                cache,
                elapsed_ms,
                total_rows,
                page_size,
                capped,
                is_rowset,
                source: None,
            },
            inline_body,
        ))
    }
}
