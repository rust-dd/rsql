use super::super::numbers::fixed;
use chrono::{DateTime, Local};
use rsql_core::{Error, storage::history::NewHistoryEntry};
use std::sync::Arc;

macro_rules! catalog_row {
    ($name:ident, $width:literal, $($field:ident => $label:literal),+ $(,)?) => {
        #[derive(Debug)]
        pub(super) struct $name { $(pub $field: String),+ }

        impl TryFrom<Vec<String>> for $name {
            type Error = Error;
            fn try_from(row: Vec<String>) -> Result<Self, Error> {
                let [$($field),+] = <[String; $width]>::try_from(row).map_err(|row| {
                    Error::SerializationError(format!("{} expected {} columns, received {}", stringify!($name), $width, row.len()))
                })?;
                Ok(Self { $($field),+ })
            }
        }

        impl $name {
            pub fn details(&self) -> Vec<(&'static str, &str)> {
                vec![$(($label, self.$field.as_str())),+]
            }
        }
    }
}

catalog_row!(
    Activity, 10,
    pid => "PID",
    user => "User",
    database => "Database",
    state => "State",
    wait_type => "Wait Event Type",
    wait_event => "Wait Event",
    query => "Returned query",
    seconds => "Duration (seconds)",
    backend => "Backend",
    client => "Client",
);
catalog_row!(
    TableStat, 14,
    schema => "Schema",
    table => "Table",
    seq_scan => "Sequential Scans",
    seq_read => "Tuples Read Sequentially",
    idx_scan => "Index Scans",
    idx_fetch => "Index Tuples Fetched",
    inserts => "Inserts",
    updates => "Updates",
    deletes => "Deletes",
    live => "Live Tuples",
    dead => "Dead Tuples",
    vacuum => "Last Vacuum",
    autovacuum => "Last Autovacuum",
    analyze => "Last Analyze",
);
catalog_row!(
    Lock, 10,
    pid => "PID",
    user => "User",
    mode => "Mode",
    lock_type => "Lock Type",
    status => "Status",
    relation => "Relation",
    schema => "Schema",
    query => "Returned query",
    seconds => "Duration (seconds)",
    wait_event => "Wait Event",
);
catalog_row!(
    IndexUsage, 9,
    schema => "Schema",
    table => "Table",
    index => "Index",
    size => "Size",
    scans => "Scans",
    tuples_read => "Tuples Read",
    tuples_fetched => "Tuples Fetched",
    status => "Status",
    definition => "Definition",
);
catalog_row!(
    Bloat, 10,
    schema => "Schema",
    table => "Table",
    live => "Live Tuples",
    dead => "Dead Tuples",
    pct => "Bloat %",
    size => "Total Size",
    vacuum => "Last Vacuum",
    autovacuum => "Last Autovacuum",
    analyze => "Last Analyze",
    autoanalyze => "Last Autoanalyze",
);

impl TableStat {
    pub fn dead_warning(&self) -> bool {
        matches!((self.live.parse::<u128>(), self.dead.parse::<u128>()), (Ok(live), Ok(dead)) if live > 0 && dead > live / 10)
    }
}

pub(super) fn number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|n| n.is_finite())
}

pub(super) fn seconds(text: &str) -> String {
    number(text).map_or_else(|| text.to_owned(), |n| format!("{}s", fixed(n, 1)))
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum Tone {
    Neutral,
    Good,
    Warning,
    Danger,
}

pub(super) fn bloat_tone(percent: Option<f64>) -> Tone {
    match percent {
        Some(n) if n > 30. => Tone::Danger,
        Some(n) if n > 10. => Tone::Warning,
        Some(_) => Tone::Good,
        None => Tone::Neutral,
    }
}

pub(super) struct Summary {
    pub total: usize,
    pub failed: usize,
    elapsed: f64,
    pub slowest: Vec<(u64, Arc<NewHistoryEntry>)>,
}
impl Summary {
    pub fn new(entries: impl Iterator<Item = (u64, Arc<NewHistoryEntry>)>, project: &str) -> Self {
        let mut rows = entries
            .filter(|(_, e)| e.project_id == project)
            .collect::<Vec<_>>();
        let total = rows.len();
        let failed = rows.iter().filter(|(_, e)| !e.success).count();
        let elapsed = rows.iter().map(|(_, e)| e.duration_ms).sum();
        rows.sort_by(|(_, a), (_, b)| b.duration_ms.total_cmp(&a.duration_ms));
        rows.truncate(10);
        Self {
            total,
            failed,
            elapsed,
            slowest: rows,
        }
    }
    pub fn average(&self) -> f64 {
        if self.total == 0 {
            0.
        } else {
            self.elapsed / self.total as f64
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lane {
    Stats,
    Activity,
    Tables,
    Locks,
    Indexes,
    Bloat,
}
impl Lane {
    pub const ALL: [Self; 6] = [
        Self::Stats,
        Self::Activity,
        Self::Tables,
        Self::Locks,
        Self::Indexes,
        Self::Bloat,
    ];
}

pub(super) struct Advised<T> {
    pub rows: Vec<T>,
    pub note: Option<gpui_kit::SharedString>,
}

pub(super) enum Rows {
    Stats(Vec<(String, String)>),
    Activity(Vec<Activity>),
    Tables(Vec<TableStat>),
    Locks(Advised<Lock>),
    Indexes(Advised<IndexUsage>),
    Bloat(Advised<Bloat>),
}

#[derive(Default)]
pub(super) struct Section {
    pub rows: Option<Rows>,
    pub error: Option<String>,
    pub success: Option<DateTime<Local>>,
    pub loading: bool,
    pub data_revision: u64,
}
impl Section {
    pub fn settle(&mut self, result: Result<Rows, String>) {
        self.loading = false;
        match result {
            Ok(rows) => {
                self.data_revision += 1;
                self.rows = Some(rows);
                self.success = Some(Local::now());
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

#[derive(Default)]
pub(super) struct MonitorSnapshot {
    pub sections: [Section; 6],
}
