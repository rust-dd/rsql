use super::Term;
use crate::grid::{Dimensions, Row};
use crate::index::{Column, Line};
use crate::term::cell::{Cell, Flags};
use crate::vte::ansi::Color;

impl<T> Term<T> {
    pub(super) fn detach_wrapped_glyph(&mut self, line: Line, start: usize) {
        if start <= 1
            && line > self.topmost_line()
            && self.grid[line][Column(0)].flags.contains(Flags::WIDE_CHAR)
        {
            let last = self.last_column();
            let previous = line - 1i32;
            if self.grid[previous][last]
                .flags
                .contains(Flags::LEADING_WIDE_CHAR_SPACER)
            {
                self.grid[previous][last]
                    .flags
                    .remove(Flags::LEADING_WIDE_CHAR_SPACER);
                if previous.0 >= 0 {
                    self.damage.damage_line(previous.0 as usize, last.0, last.0);
                } else if self.grid.display_offset() > 0 {
                    // The per-line tracker only addresses the live screen.
                    self.mark_fully_damaged();
                }
            }
        }
    }

    pub(super) fn erase_line_columns(&mut self, line: Line, start: Column, end: Column, bg: Color) {
        if start == end {
            return;
        }
        self.detach_wrapped_glyph(line, start.0);
        let span = self.grid[line].erase_columns(start, end, bg);
        self.damage
            .damage_line(line.0 as usize, span.start.0, span.end.0 - 1);
    }
}

impl Row<Cell> {
    pub(super) fn take_margin_wrap(&mut self) -> bool {
        let last = Column(self.len() - 1);
        let flags = &mut self[last].flags;
        let wrapped = flags.contains(Flags::WRAPLINE);
        flags.remove(Flags::WRAPLINE | Flags::LEADING_WIDE_CHAR_SPACER);
        wrapped
    }

    pub(super) fn has_wide_lead(&self, col: Column) -> bool {
        col.0 > 0
            && self[col].flags.contains(Flags::WIDE_CHAR_SPACER)
            && self[col - 1].flags.contains(Flags::WIDE_CHAR)
    }

    pub(super) fn clear_split_wide(&mut self, at: usize, bg: Color) {
        if at < self.len() && self.has_wide_lead(Column(at)) {
            self[Column(at - 1)..Column(at + 1)].fill(bg.into());
        }
    }

    pub(super) fn erase_columns(
        &mut self,
        start: Column,
        end: Column,
        bg: Color,
    ) -> std::ops::Range<Column> {
        let mut span = start..end;
        if start < end {
            if self.has_wide_lead(start) {
                span.start -= 1;
            }
            if end.0 < self.len() && self.has_wide_lead(end) {
                span.end += 1;
            }
            self[span.clone()].fill(bg.into());
        }
        span
    }
}
