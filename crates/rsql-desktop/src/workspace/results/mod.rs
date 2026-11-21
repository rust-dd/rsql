mod cell_editor;
mod clipboard;
mod delegate;
mod diff_panel;
mod edit_actions;
mod edit_toolbar;
mod export_jobs;
mod export_menu;
pub(super) use export_menu::ExportMenu;
mod fields;
mod record;
mod review_dialog;
mod review_target;
pub(super) use fields::{FIELD_COUNT, FieldCache, Preview};
mod search;
pub(super) mod toolbar;
pub(super) use search::SearchInput;
mod foreign_keys;
mod grid;
pub(crate) mod selection;
mod tsv;

pub(super) use grid::Grid;
pub(crate) use grid::bind_keys;

use super::*;
use gpui_kit::component::{
    ActiveTheme,
    table::{TableDelegate, TableState},
};
use rsql_core::wire::Cell;
use rsql_desktop::{
    execution::QueryResult,
    results::{
        CellState, ResultAction, ResultView,
        cell_input::{CellAccess, EditIntent},
    },
};
use selection::{Position, Selection};
use std::{collections::HashSet, ops::Range, sync::Arc};
use uuid::Uuid;

#[derive(Clone)]
struct GridOwner {
    session: Rc<Session>,
    pane: Owner,
    result: Uuid,
}

impl GridOwner {
    fn can_edit(&self) -> bool {
        self.with_view(|v| v.allows(ResultAction::Edit))
            .unwrap_or(false)
    }

    fn with_view<T>(&self, read: impl FnOnce(&ResultView) -> T) -> Option<T> {
        self.with_result(|v, _| read(v))
    }

    fn with_result<T>(&self, read: impl FnOnce(&ResultView, u64) -> T) -> Option<T> {
        if *self.session.preflight_stop.borrow() {
            return None;
        }
        let docs = self.session.documents.borrow();
        let tab = docs
            .active()
            .filter(|t| t.id == self.pane.0 && (self.pane.1 == Side::Main || t.split))?;
        let view = tab.pane(self.pane.1).view().filter(|v| {
            v.data.id == self.result && v.choice() == rsql_desktop::results::ViewChoice::Grid
        })?;
        Some(read(view, tab.pane(self.pane.1).generation()))
    }
}

#[derive(Clone)]
struct GridData {
    result: Arc<QueryResult>,
    filtered: Option<Arc<[usize]>>,
    access: Arc<CellAccess>,
}

impl GridData {
    fn rows(&self) -> usize {
        self.filtered
            .as_ref()
            .map_or(self.result.total_rows, |f| f.len())
    }

    fn retained(&self, pos: Position) -> Option<Position> {
        let row = match &self.filtered {
            Some(rows) => *rows.get(pos.row)?,
            None if pos.row < self.rows() => pos.row,
            _ => return None,
        };
        (pos.col < self.result.columns().len()).then_some(Position { row, ..pos })
    }
}

fn clipped(value: &str) -> String {
    match value.char_indices().nth(160) {
        Some((end, _)) => format!("{}…", &value[..end]),
        None => value.to_owned(),
    }
}
