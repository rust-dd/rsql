use rsql_core::wire::Cell;
use std::collections::HashSet;
use sum_tree::{TreeMap, TreeSet};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub struct Position {
    pub row: usize,
    pub col: usize,
}

/// Immutable projection supplied by the pane's edit consumer.
#[derive(Default, Clone)]
pub struct CellAccess {
    pub session: Option<uuid::Uuid>,
    pub eligibility: Option<std::sync::Arc<EditEligibility>>,
    pub deleted: TreeSet<usize>,
    pub edited: TreeMap<Position, std::sync::Arc<Cell>>,
    pub foreign_keys: HashSet<usize>,
}

#[derive(Default)]
pub struct EditEligibility {
    pub rows: HashSet<usize>,
    pub columns: HashSet<usize>,
}

impl CellAccess {
    pub fn allows(&self, pos: Position) -> bool {
        self.eligibility
            .as_ref()
            .is_some_and(|e| e.rows.contains(&pos.row) && e.columns.contains(&pos.col))
            && !self.deleted.contains(&pos.row)
    }
}

/// Positions in an edit intent refer to retained rows and positional result columns.
pub struct EditIntent {
    pub prepared: Option<super::edits::PreparedEdit>,
    pub session: Option<uuid::Uuid>,
    pub result: uuid::Uuid,
    pub changes: Vec<(Position, Cell)>,
}

// ECMAScript whitespace includes BOM and excludes NEXT LINE.
fn null_quotes(input: &str) -> Option<usize> {
    let inner = input.trim_start_matches('"');
    let quotes = input.len() - inner.len();
    let token = inner.trim_end_matches('"');
    (inner.len() - token.len() == quotes
        && token
            .trim_matches(|c: char| c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}'))
            .eq_ignore_ascii_case("null"))
    .then_some(quotes)
}

pub fn parse_cell_input(input: &str) -> Cell {
    match null_quotes(input) {
        Some(0) => None,
        Some(_) => Some(input[1..input.len() - 1].into()),
        None => Some(input.into()),
    }
}

pub fn format_cell_input(value: &Cell) -> String {
    match value {
        None => "NULL".into(),
        Some(text) if null_quotes(text).is_some() => format!("\"{text}\""),
        Some(text) => text.clone(),
    }
}
