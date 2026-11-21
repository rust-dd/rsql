use super::{Anchor, Selection, SelectionRange, SelectionType};
use crate::grid::Dimensions;
use crate::grid::reflow::ReflowPoint;
use crate::index::{Boundary, Column, Point, Side};
use crate::term::Term;
use crate::term::cell::Flags;

impl Selection {
    pub(crate) fn resize<T>(
        mut self,
        term: &mut Term<T>,
        reflow: bool,
        lines: usize,
        columns: usize,
        line_delta: i32,
    ) -> Option<Self> {
        let reverse = self.region.start.point > self.region.end.point
            || (self.region.start.point == self.region.end.point
                && self.region.start.side == Side::Right);
        let [first, last] = if reverse {
            [self.region.end, self.region.start]
        } else {
            [self.region.start, self.region.end]
        };
        let grid = term.grid();
        let expanded = matches!(self.ty, SelectionType::Semantic | SelectionType::Lines);
        let range = match self.ty {
            SelectionType::Simple => self.range_simple(first, last, grid.columns()),
            SelectionType::Semantic | SelectionType::Lines => self.to_range(term),
            SelectionType::Block => None,
        }
        .filter(|r| r.start <= r.end);
        let had_extent = self.current_extent().is_some();
        self.extent = None;
        let empty = self.is_empty() || (self.ty == SelectionType::Simple && range.is_none());
        let track = |mut point: Point| {
            // A wrap-margin placeholder moves with the following wide glyph.
            if reflow
                && point == point.grid_clamp(grid, Boundary::Grid)
                && grid[point].flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
                && point.line < grid.bottommost_line()
                && grid[point.line + 1i32][Column(0)]
                    .flags
                    .contains(Flags::WIDE_CHAR)
            {
                point = Point::new(point.line + 1, Column(0));
            }
            point.line += line_delta;
            ReflowPoint::new(point)
        };
        let (start, end) = range.map_or((first.point, last.point), |r| (r.start, r.end));
        // Both ends can survive while alternate-screen narrowing removes cells between them.
        let cropped_interior = !reflow && columns < grid.columns() && start.line < end.line;
        let extent = [track(start), track(end)];
        // Reflow expanded display bounds and the directed gesture through the same row moves.
        // A future update still starts from the user's fixed cell, even if a wrap was cropped.
        let (points, gesture) = if expanded {
            let mut points = [
                extent[0],
                extent[1],
                track(self.region.start.point),
                track(self.region.end.point),
            ];
            term.grid_mut()
                .resize_with_points(reflow, lines, columns, &mut points);
            let [a, b, fixed, moving] = points;
            ([a, b], Some([fixed, moving]))
        } else {
            let mut points = extent;
            term.grid_mut()
                .resize_with_points(reflow, lines, columns, &mut points);
            (points, None)
        };
        let grid = term.grid();
        let [first_point, last_point] = points;
        let (mut start, end) = (first_point.mapped()?, last_point.mapped()?);
        if start.line.max(end.line) < grid.topmost_line()
            || start.line.min(end.line) > grid.bottommost_line()
        {
            return None;
        }

        // For an inclusive span a removed starting cell begins *after* the surviving prefix;
        // a removed ending cell ends at the prefix. An empty intersection has no selection.
        if range.is_some() && first_point.after_cell && start.line >= grid.topmost_line() {
            if start.line >= end.line || start.line >= grid.bottommost_line() {
                return None;
            }
            start = Point::new(start.line + 1, Column(0));
        }
        if let Some([fixed, moving]) = gesture {
            range?;
            let clamp = |point: Point| point.grid_clamp(grid, Boundary::Grid);
            let (start, end) = (clamp(start), clamp(end));
            if start > end {
                return None;
            }
            for (anchor, point) in [
                (&mut self.region.start, fixed),
                (&mut self.region.end, moving),
            ] {
                let mapped = point.mapped()?;
                anchor.point = clamp(mapped);
                if mapped.line < grid.topmost_line() {
                    anchor.side = Side::Left;
                } else if point.after_cell || mapped.line > grid.bottommost_line() {
                    anchor.side = Side::Right;
                }
            }
            let retained = SelectionRange::new(start, end, false);
            let clipped = [first_point, last_point, fixed, moving]
                .into_iter()
                .any(|tracked| {
                    tracked.after_cell
                        || tracked.mapped().is_some_and(|point| point != clamp(point))
                });
            // Only an intact gesture can follow subsequent edits using its native search.
            if had_extent || cropped_interior || clipped || self.to_range(term) != Some(retained) {
                self.extent = Some((self.ty, retained));
            }
            return Some(self);
        }
        let simple = self.ty == SelectionType::Simple && !empty;
        let restore = |mut anchor: Anchor, point: Point, first: bool, after_cell: bool| {
            anchor.point = point.grid_clamp(grid, Boundary::Grid);
            if self.ty == SelectionType::Block {
                anchor.point.column = point.column.min(grid.last_column());
                if after_cell {
                    anchor.side = Side::Right;
                }
            } else if point.line < grid.topmost_line() {
                anchor.side = Side::Left;
            } else if point.line > grid.bottommost_line() {
                anchor.side = Side::Right;
            } else if simple && first && anchor.side == Side::Right {
                let previous = anchor.point.sub(grid, Boundary::Grid, 1);
                if previous == anchor.point {
                    anchor.side = Side::Left;
                }
                anchor.point = previous;
            } else if simple && !first && anchor.side == Side::Left {
                let next = anchor.point.add(grid, Boundary::Grid, 1);
                if next == anchor.point {
                    anchor.side = Side::Right;
                }
                anchor.point = next;
            }
            anchor
        };
        let start = restore(first, start, true, first_point.after_cell);
        let end = restore(last, end, false, last_point.after_cell);
        self.region = if reverse { end..start } else { start..end };
        if empty && !self.is_empty() {
            self.region.end = self.region.start;
        }
        Some(self)
    }
}
