use super::{SelectionType, Side, TerminalModel};
use alacritty_terminal::{
    grid::Dimensions,
    index::{Column, Line, Point},
    selection::{Selection, SelectionRange},
    term::viewport_to_point,
};
use std::sync::Arc;

pub(crate) struct DisplayedRows {
    revision: u64,
    first: Line,
    wraps_from_history: bool,
    user_scroll: i64,
    columns: usize,
    visible: Box<[DisplayedRow]>,
}

struct DisplayedRow {
    id: u64,
    wraps: bool,
}

#[derive(Clone)]
pub struct DisplayedPoint {
    rows: Arc<DisplayedRows>,
    row: usize,
    column: Column,
}

impl DisplayedRows {
    pub(crate) fn at(self: &Arc<Self>, row: usize, column: usize) -> Option<DisplayedPoint> {
        if column >= self.columns || row >= self.visible.len() {
            return None;
        }
        Some(DisplayedPoint {
            rows: Arc::clone(self),
            row,
            column: Column(column),
        })
    }
}

impl DisplayedPoint {
    fn verifies_selection(
        &self,
        model: &TerminalModel,
        point: Point,
        selection: &Selection,
    ) -> bool {
        if self.rows.revision == model.revision {
            return true;
        }
        let range = match selection.ty {
            SelectionType::Semantic if "()[]{}<>".contains(model.term.grid()[point].c) => {
                // Without a mate, native selection falls back to a word whose painted extent is unknown.
                let Some(mate) = model.term.bracket_search(point) else {
                    return false;
                };
                SelectionRange::new(point.min(mate), point.max(mate), false)
            }
            SelectionType::Lines | SelectionType::Semantic => {
                let Some(range) = selection.to_range(&model.term) else {
                    return false;
                };
                range
            }
            _ => return true,
        };
        self.covers(model, point.line, range)
    }

    fn covers(&self, model: &TerminalModel, anchor: Line, range: SelectionRange) -> bool {
        // A changed delimiter may have shortened the live range since paint.
        let visible = &self.rows.visible;
        let mut first = self.row;
        while first > 0 && visible[first - 1].wraps {
            first -= 1;
        }
        if first == 0 && self.rows.wraps_from_history {
            return false;
        }
        let mut last = self.row;
        while visible[last].wraps {
            last += 1;
            if last == visible.len() {
                return false;
            }
        }
        let start = range.start.line.0.min(anchor.0 - (self.row - first) as i32);
        let end = range.end.line.0.max(anchor.0 + (last - self.row) as i32);
        let grid = model.term.grid();
        (start..=end).all(|line| {
            let painted = self.row as i32 + (line - anchor.0);
            (grid.topmost_line().0..=grid.bottommost_line().0).contains(&line)
                && usize::try_from(painted)
                    .ok()
                    .and_then(|row| visible.get(row))
                    .is_some_and(|row| grid[Line(line)].matches_display_id(row.id))
        })
    }
}

impl TerminalModel {
    pub(crate) fn displayed_rows(&self) -> Arc<DisplayedRows> {
        let grid = self.term.grid();
        let first = Line(-(grid.display_offset() as i32));
        let wraps = |line: Line| {
            grid[line][grid.last_column()]
                .flags
                .contains(super::CellFlags::WRAPLINE)
        };
        Arc::new(DisplayedRows {
            first,
            wraps_from_history: first > grid.topmost_line() && wraps(first - 1i32),
            columns: grid.columns(),
            user_scroll: self.user_scroll,
            revision: self.revision,
            visible: (0..grid.screen_lines())
                .map(|row| {
                    let line = first + row;
                    DisplayedRow {
                        id: grid[line].display_id(),
                        wraps: wraps(line),
                    }
                })
                .collect(),
        })
    }

    fn resolve_displayed(&self, target: &DisplayedPoint) -> Option<Point> {
        let grid = self.term.grid();
        let lines = grid.topmost_line()..=grid.bottommost_line();
        let original = target.rows.first + target.row;
        let id = target.rows.visible[target.row].id;
        let line = if lines.contains(&original) && grid[original].matches_display_id(id) {
            Some(original)
        } else if target.rows.revision != self.revision {
            (lines.start().0..=lines.end().0)
                .map(Line)
                .find(|&line| grid[line].matches_display_id(id))
        } else {
            None
        };
        line.map(|line| Point::new(line, target.column))
    }

    pub(crate) fn select_displayed(
        &mut self,
        kind: Option<SelectionType>,
        target: DisplayedPoint,
        side: Side,
    ) {
        let Some(point) = self.resolve_displayed(&target) else {
            self.clear_selection();
            return;
        };
        if let Some(kind) = kind {
            let selection = Selection::new(kind, point, side);
            if !target.verifies_selection(self, point, &selection) {
                self.clear_selection();
                return;
            }
            self.term.selection = Some(selection);
        } else if let Some(selection) = &mut self.term.selection {
            selection.update(point, side);
        }
        self.revision = self.revision.wrapping_add(1);
    }

    pub(crate) fn scroll_displayed(
        &mut self,
        delta: i32,
        row: usize,
        target: DisplayedPoint,
        side: Side,
    ) {
        if self.term.selection.is_none() || self.term.mode().contains(super::TermMode::ALT_SCREEN) {
            return;
        }
        let Some(point) = self.resolve_displayed(&target) else {
            self.clear_selection();
            return;
        };
        // Anchor the view to the surviving row, including wheel gestures since its last paint.
        let limit = self.term.grid().history_size() as i64;
        let travel = self.user_scroll.wrapping_sub(target.rows.user_scroll);
        let base = (row as i64 - i64::from(point.line.0))
            .saturating_add(travel)
            .clamp(0, limit);
        let next = (base + i64::from(delta)).clamp(0, limit);
        let current = self.term.grid().display_offset() as i64;
        if next != current {
            self.term
                .scroll_display(super::Scroll::Delta((next - current) as i32));
        }
        self.user_scroll = self.user_scroll.wrapping_add(next - base);
        self.update_selection(row, point.column.0, side);
    }

    pub(crate) fn select_all(&mut self) {
        let mut selection = Selection::new(
            SelectionType::Simple,
            Point::new(self.term.topmost_line(), Column(0)),
            Side::Left,
        );
        selection.update(
            Point::new(self.term.bottommost_line(), self.term.last_column()),
            Side::Right,
        );
        self.term.selection = Some(selection);
        self.revision = self.revision.wrapping_add(1);
    }

    /// Selection coordinates are viewport-relative, including a scrolled view.
    pub fn start_selection(&mut self, kind: SelectionType, row: usize, column: usize, side: Side) {
        self.term.selection = Some(Selection::new(
            kind,
            self.selection_point(row, column),
            side,
        ));
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn update_selection(&mut self, row: usize, column: usize, side: Side) {
        let point = self.selection_point(row, column);
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, side);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn clear_selection(&mut self) {
        if self.term.selection.take().is_some() {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    fn selection_point(&self, row: usize, column: usize) -> Point {
        let point = Point::new(
            row.min(self.term.screen_lines() - 1),
            Column(column.min(self.term.columns() - 1)),
        );
        viewport_to_point(self.term.grid().display_offset(), point)
    }
}
