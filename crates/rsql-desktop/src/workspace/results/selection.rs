use std::ops::Range;

pub use rsql_desktop::results::cell_input::Position;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    pub anchor: Position,
    pub focus: Position,
    pub rows: bool,
}

impl Selection {
    pub fn select(&mut self, pos: Position, extend: bool, rows: bool) {
        if !extend || self.rows != rows {
            self.anchor = pos;
        }
        self.focus = pos;
        self.rows = rows;
    }

    pub fn ranges(&self, columns: usize) -> (Range<usize>, Range<usize>) {
        let interval = |a: usize, b: usize| a.min(b)..a.max(b).saturating_add(1);
        (
            interval(self.anchor.row, self.focus.row),
            if self.rows {
                0..columns
            } else {
                interval(self.anchor.col, self.focus.col)
            },
        )
    }

    pub fn table_label(&self, columns: usize) -> String {
        let (rows, cols) = self.ranges(columns);
        let active = if self.rows {
            format!("Active row {}.", self.focus.row + 1)
        } else {
            format!(
                "Active row {}, column {}.",
                self.focus.row + 1,
                self.focus.col + 1
            )
        };
        let count = rows.len().saturating_mul(cols.len());
        if count == 1 {
            format!(
                "Query results. Copy range row {}, column {} (1 cell). {active}",
                rows.start + 1,
                cols.start + 1
            )
        } else {
            format!(
                "Query results. Copy range rows {}–{}, columns {}–{} ({count} cells). {active}",
                rows.start + 1,
                rows.end,
                cols.start + 1,
                cols.end
            )
        }
    }

    pub fn contains(&self, pos: Position) -> bool {
        let (rows, cols) = self.ranges(usize::MAX);
        rows.contains(&pos.row) && cols.contains(&pos.col)
    }

    pub fn navigate(
        &mut self,
        key: &str,
        shift: bool,
        rows: usize,
        cols: usize,
        page: usize,
    ) -> bool {
        if rows == 0 || cols == 0 {
            return false;
        }
        let mut pos = self.focus;
        let last = cols - 1;
        match key {
            "right" => pos.col = pos.col.saturating_add(1),
            "left" => pos.col = pos.col.saturating_sub(1),
            "up" => pos.row = pos.row.saturating_sub(1),
            "down" => pos.row = pos.row.saturating_add(1),
            "home" => pos.col = 0,
            "end" => pos.col = last,
            "pageup" => pos.row = pos.row.saturating_sub(page.max(1)),
            "pagedown" => pos.row = pos.row.saturating_add(page.max(1)),
            "tab" if shift && pos.col == 0 && pos.row > 0 => {
                pos.row -= 1;
                pos.col = last;
            }
            "tab" if !shift && pos.col == last && pos.row + 1 < rows => {
                pos.row += 1;
                pos.col = 0;
            }
            "tab" if shift => pos.col = pos.col.saturating_sub(1),
            "tab" => pos.col = pos.col.saturating_add(1),
            "space" if shift => {
                self.select(pos, false, true);
                return true;
            }
            _ => return false,
        }
        pos.row = pos.row.min(rows - 1);
        pos.col = pos.col.min(last);
        self.select(
            pos,
            shift && key != "tab",
            self.rows && shift && matches!(key, "up" | "down" | "pageup" | "pagedown"),
        );
        true
    }
}
