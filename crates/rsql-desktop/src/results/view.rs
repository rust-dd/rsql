use super::{
    ForeignQuery, PageLane, PageLoader,
    diff::{Comparison, DiffKey, RowSnapshot, SnapshotKey},
};
use crate::execution::QueryResult;
use rsql_core::services::ProjectTicket;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum ViewChoice {
    #[default]
    Grid,
    Record,
    History,
    Explain,
    Diff,
    Map,
}

#[derive(Clone, Copy)]
pub enum ResultAction {
    Search,
    Record,
    Edit,
    Pin,
    Diff,
    Export,
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldLocation {
    pub pinned: Option<uuid::Uuid>,
    pub row: usize,
    pub column: usize,
    pub label: bool,
}

pub struct MapInput {
    pub rows: RowSnapshot,
    pub first_page_of: Option<usize>,
}

pub struct ResultView {
    pub data: Arc<QueryResult>,
    pub loader: PageLoader,
    pub route: ProjectTicket,
    pub generated: Option<Arc<ForeignQuery>>,
    pub record_index: usize,
    pub record_fields_start: usize,
    pub field_detail: Option<(FieldLocation, usize)>,
    pub search_revision: u64,
    pub applied_term: String,
    pub restart_search: bool,
    pub filter_revision: u64,
    pub search_owner: Option<watch::Sender<()>>,
    pub export_job: Option<super::export::ExportJob>,
    pub export_message: Option<String>,
    pub comparison: Option<DiffWork>,
    pub cell_access: Arc<super::cell_input::CellAccess>,
    pub edit: Option<super::edits::EditSession>,
    pub write_owner: Option<tokio::sync::watch::Sender<()>>,
    pub write_uncertain: bool,
    pub edit_error: Option<String>,
    pub navigation_owner: Option<(uuid::Uuid, tokio::sync::watch::Sender<()>)>,
    pub filtered_rows: Option<Arc<[usize]>>,
    browse_target: Option<std::sync::Weak<super::edits::EditTarget>>,
    choice: ViewChoice,
    last_map: Option<SnapshotKey>,
}

pub struct DiffWork {
    pub key: DiffKey,
    pub owner: watch::Sender<()>,
    pub result: Option<Arc<Comparison>>,
}

impl ResultView {
    pub(crate) fn new(data: Arc<QueryResult>, route: ProjectTicket, lane: PageLane) -> Self {
        let loader = PageLoader::new(
            lane,
            data.cache.clone(),
            route.clone(),
            data.packed_source().cloned(),
        );
        let target = data.cache.target();
        let access = target.as_ref().map(|t| t.browse()).unwrap_or_default();
        Self {
            browse_target: target.as_ref().map(Arc::downgrade),
            data,
            loader,
            route,
            generated: None,
            record_index: 0,
            record_fields_start: 0,
            field_detail: None,
            search_revision: 0,
            applied_term: String::new(),
            restart_search: false,
            filter_revision: 0,
            search_owner: None,
            comparison: None,
            export_job: None,
            export_message: None,
            cell_access: Arc::new(access),
            edit: None,
            write_owner: None,
            write_uncertain: false,
            edit_error: None,
            navigation_owner: None,
            filtered_rows: None,
            choice: ViewChoice::Grid,
            last_map: None,
        }
    }
    pub fn sync_target(&mut self) {
        if self.choice == ViewChoice::Map && self.map_input().is_none() && !self.can_read_map_page()
        {
            self.choose(ViewChoice::Grid);
        }
        if !self.allows(ResultAction::Search) {
            if self.edit.is_some() || self.data.total_rows > self.data.page_size {
                self.reset_filter();
            } else {
                self.interrupt_search();
            }
            if matches!(self.choice, ViewChoice::Record | ViewChoice::Diff) {
                self.choose(ViewChoice::Grid);
            }
        }
        let target = self.data.cache.target();
        let same = match (&self.browse_target, &target) {
            (None, None) => true,
            (Some(old), Some(t)) => old.as_ptr() == Arc::as_ptr(t),
            _ => false,
        };
        if !same {
            self.browse_target = target.as_ref().map(Arc::downgrade);
            self.navigation_owner = None;
            self.edit = None;
            self.cell_access = Arc::new(target.as_ref().map(|t| t.browse()).unwrap_or_default());
        }
    }
    pub fn choice(&self) -> ViewChoice {
        self.choice
    }
    pub fn choose(&mut self, requested: ViewChoice) {
        self.choose_available(requested, false, false);
    }
    pub fn choose_available(&mut self, requested: ViewChoice, plan: bool, pin: bool) {
        let choice = match requested {
            ViewChoice::History if self.edit.is_none() => requested,
            ViewChoice::Record if self.allows(ResultAction::Record) => requested,
            ViewChoice::Diff if pin && self.allows(ResultAction::Diff) => requested,
            ViewChoice::Explain if plan && self.edit.is_none() => requested,
            _ => ViewChoice::Grid,
        };
        self.set_choice(choice);
    }
    fn set_choice(&mut self, choice: ViewChoice) {
        let previous = std::mem::replace(&mut self.choice, choice);
        if self.choice != previous {
            self.field_detail = None;
        }
        if self.choice != ViewChoice::Grid {
            self.navigation_owner = None;
        }
        if self.choice != ViewChoice::Diff {
            self.comparison = None;
        }
    }
    pub fn key(&self) -> SnapshotKey {
        SnapshotKey {
            result: self.data.id,
            filter_revision: self.filter_revision,
        }
    }
    pub fn map_input(&self) -> Option<MapInput> {
        if self.edit.is_some() || !self.data.is_rowset || !self.route.is_current() {
            return None;
        }
        let first_page_of =
            (self.data.total_rows > self.data.page_size).then_some(self.data.total_rows);
        let mut rows = self.first_rows()?;
        if first_page_of.is_some() {
            rows.filtered = None;
        }
        Some(MapInput {
            rows,
            first_page_of,
        })
    }

    pub fn can_read_map_page(&self) -> bool {
        self.last_map == Some(self.key())
            && self.edit.is_none()
            && self.data.is_rowset
            && self.route.is_current()
            && self.data.retained().is_some_and(|source| source.is_open())
    }

    pub fn choose_main_map(&mut self, key: SnapshotKey) {
        if self.key() == key && (self.map_input().is_some() || self.can_read_map_page()) {
            self.last_map = Some(key);
            self.set_choice(ViewChoice::Map);
        }
    }

    pub fn snapshot(&self, action: ResultAction) -> Option<RowSnapshot> {
        if !self.allows(action) || self.data.total_rows > self.data.page_size {
            return None;
        }
        self.first_rows()
    }
    fn first_rows(&self) -> Option<RowSnapshot> {
        Some(RowSnapshot {
            key: self.key(),
            columns: self.data.cache.0.columns.get()?.clone(),
            page: self.data.cache.page(0)?,
            filtered: self.filtered_rows.clone(),
        })
    }
    pub fn install_filter(&mut self, term: String, rows: Option<Arc<[usize]>>) {
        self.search_owner = None;
        self.restart_search = false;
        self.applied_term = term;
        self.filtered_rows = rows;
        self.filter_revision += 1;
        if self.choice == ViewChoice::Map {
            self.last_map = Some(self.key());
        }
        self.record_index = 0;
        self.record_fields_start = 0;
        self.field_detail = None;
        self.comparison = None;
    }
    pub fn reset_filter(&mut self) {
        if self.search_owner.take().is_some() || self.restart_search || self.filtered_rows.is_some()
        {
            self.search_revision += 1;
            self.install_filter(String::new(), None);
        }
    }
    fn interrupt_search(&mut self) {
        if self.search_owner.take().is_some() {
            self.search_revision += 1;
            self.restart_search = true;
        }
    }

    pub fn prepare_explain(&mut self) {
        self.loader.suspend();
        self.field_detail = None;
        self.navigation_owner = None;
        if self.write_owner.take().is_some() {
            self.write_uncertain = true;
            self.edit_error = Some("Write interrupted. Run the query again before editing.".into());
        }
        self.edit = None;
        self.cell_access = Arc::new(
            self.data
                .cache
                .target()
                .map(|t| t.browse())
                .unwrap_or_default(),
        );
        self.interrupt_search();
        self.comparison = None;
    }

    pub fn allows(&self, action: ResultAction) -> bool {
        let local =
            self.data.total_rows <= self.data.page_size && self.data.cache.page(0).is_some();
        if self.edit.is_some() && !matches!(action, ResultAction::Edit | ResultAction::Retry) {
            return false;
        }
        match action {
            ResultAction::Retry => {
                self.data.packed_source().is_some() && self.data.cache.first_error().is_some()
            }
            ResultAction::Search => local && !self.data.columns().is_empty(),
            ResultAction::Export => {
                self.data.total_rows > 0
                    && (local
                        || (self.filtered_rows.is_none() && self.data.packed_source().is_some()))
            }
            _ => self.data.total_rows > 0 && local,
        }
    }
}
