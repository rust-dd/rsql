//! State management for a selection in the grid.
//!
//! A selection should start when the mouse is clicked, and it should be
//! finalized when the button is released. The selection should be cleared
//! when text is added/removed/scrolled on the screen. The selection should
//! also be cleared if the user clicks off of the selection.

mod reflow;
mod rotation;

use std::cmp::min;
use std::mem;
use std::ops::{Bound, Range, RangeBounds};

use crate::grid::{Dimensions, GridCell, Indexed};
use crate::index::{Boundary, Column, Line, Point, Side};
use crate::term::Term;
use crate::term::cell::{Cell, Flags};
use crate::vte::ansi::CursorShape;

/// A Point and side within that point.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct Anchor {
    point: Point,
    side: Side,
}

impl Anchor {
    fn new(point: Point, side: Side) -> Anchor {
        Anchor { point, side }
    }

    fn column_edge(self) -> usize {
        self.point.column.0 + usize::from(self.side == Side::Right)
    }
}

/// Represents a range of selected cells.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SelectionRange {
    /// Start point, top left of the selection.
    pub start: Point,
    /// End point, bottom right of the selection.
    pub end: Point,
    /// Whether this selection is a block selection.
    pub is_block: bool,
}

impl SelectionRange {
    pub fn new(start: Point, end: Point, is_block: bool) -> Self {
        assert!(start <= end);
        Self { start, end, is_block }
    }
}

impl SelectionRange {
    /// Check if a point lies within the selection.
    pub fn contains(&self, point: Point) -> bool {
        self.start.line <= point.line
            && self.end.line >= point.line
            && (self.start.column <= point.column
                || (self.start.line != point.line && !self.is_block))
            && (self.end.column >= point.column || (self.end.line != point.line && !self.is_block))
    }

    /// Check if the cell at a point is part of the selection.
    pub fn contains_cell(
        &self,
        indexed: &Indexed<&Cell>,
        point: Point,
        shape: CursorShape,
    ) -> bool {
        // Do not invert block cursor at selection boundaries.
        if shape == CursorShape::Block
            && point == indexed.point
            && (self.start == indexed.point
                || self.end == indexed.point
                || (self.is_block
                    && ((self.start.line == indexed.point.line
                        && self.end.column == indexed.point.column)
                        || (self.end.line == indexed.point.line
                            && self.start.column == indexed.point.column))))
        {
            return false;
        }

        // Point itself is selected.
        if self.contains(indexed.point) {
            return true;
        }

        // Check if a wide char's trailing spacer is selected.
        indexed.cell.flags().contains(Flags::WIDE_CHAR)
            && self.contains(Point::new(indexed.point.line, indexed.point.column + 1))
    }
}

/// Different kinds of selection.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SelectionType {
    Simple,
    Block,
    Semantic,
    Lines,
}

/// Describes a region of a 2-dimensional area.
///
/// Used to track a text selection. There are four supported modes, each with its own constructor:
/// [`simple`], [`block`], [`semantic`], and [`lines`]. The [`simple`] mode precisely tracks which
/// cells are selected without any expansion. [`block`] will select rectangular regions.
/// [`semantic`] mode expands the initial selection to the nearest semantic escape char in either
/// direction. [`lines`] will always select entire lines.
///
/// Calls to [`update`] operate different based on the selection kind. The [`simple`] and [`block`]
/// mode do nothing special, simply track points and sides. [`semantic`] will continue to expand
/// out to semantic boundaries as the selection point changes. Similarly, [`lines`] will always
/// expand the new point to encompass entire lines.
///
/// [`simple`]: enum.Selection.html#method.simple
/// [`block`]: enum.Selection.html#method.block
/// [`semantic`]: enum.Selection.html#method.semantic
/// [`lines`]: enum.Selection.html#method.lines
/// [`update`]: enum.Selection.html#method.update
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub ty: SelectionType,
    region: Range<Anchor>,
    extent: Option<(SelectionType, SelectionRange)>,
}

impl Selection {
    pub fn new(ty: SelectionType, location: Point, side: Side) -> Selection {
        Self {
            region: Range { start: Anchor::new(location, side), end: Anchor::new(location, side) },
            ty,
            extent: None,
        }
    }

    fn current_extent(&self) -> Option<SelectionRange> {
        let (kind, range) = self.extent?;
        (self.ty == kind).then_some(range)
    }

    /// Update the end of the selection.
    pub fn update(&mut self, point: Point, side: Side) {
        self.extent = None;
        self.region.end = Anchor::new(point, side);
    }

    pub fn rotate<D: Dimensions>(
        mut self,
        dimensions: &D,
        range: &Range<Line>,
        delta: i32,
    ) -> Option<Selection> {
        if let Some(extent) = self.current_extent() {
            let (mut selection, retained, _) = self.rotate_extent(dimensions, range, delta, extent)?;
            selection.extent = Some((selection.ty, retained));
            return Some(selection);
        }
        self.extent = None;

        let bottommost_line = dimensions.bottommost_line();
        let range_bottom = range.end;
        let range_top = range.start;

        let (mut start, mut end) = (&mut self.region.start, &mut self.region.end);
        if start.point > end.point {
            mem::swap(&mut start, &mut end);
        }

        // Rotate start of selection.
        if (start.point.line >= range_top || range_top == 0) && start.point.line < range_bottom {
            start.point.line = min(start.point.line - delta, bottommost_line);

            // If end is within the same region, delete selection once start rotates out.
            if start.point.line >= range_bottom && end.point.line < range_bottom {
                return None;
            }

            // Clamp selection to start of region.
            if start.point.line < range_top && range_top != 0 {
                if self.ty != SelectionType::Block {
                    start.point.column = Column(0);
                    start.side = Side::Left;
                }
                start.point.line = range_top;
            }
        }

        // Rotate end of selection.
        if (end.point.line >= range_top || range_top == 0) && end.point.line < range_bottom {
            end.point.line = min(end.point.line - delta, bottommost_line);

            // Delete selection if end has overtaken the start.
            if end.point.line < start.point.line {
                return None;
            }

            // Clamp selection to end of region.
            if end.point.line >= range_bottom {
                if self.ty != SelectionType::Block {
                    end.point.column = dimensions.last_column();
                    end.side = Side::Right;
                }
                end.point.line = range_bottom - 1;
            }
        }

        Some(self)
    }

    pub fn is_empty(&self) -> bool {
        match self.ty {
            SelectionType::Simple => {
                let (mut start, mut end) = (self.region.start, self.region.end);
                if start.point > end.point {
                    mem::swap(&mut start, &mut end);
                }

                // Simple selection is empty when the points are identical
                // or two adjacent cells have the sides right -> left.
                start == end
                    || (start.side == Side::Right
                        && end.side == Side::Left
                        && (start.point.line == end.point.line)
                        && start.point.column + 1 == end.point.column)
            },
            SelectionType::Block => self.region.start.column_edge() == self.region.end.column_edge(),
            SelectionType::Semantic | SelectionType::Lines => false,
        }
    }

    /// Check whether selection contains any point in a given range.
    pub fn intersects_range<R: RangeBounds<Line>>(&self, range: R) -> bool {
        let (start, end) = match self.current_extent() {
            Some(extent) => (extent.start.line, extent.end.line),
            None => {
                let (a, b) = (self.region.start.point.line, self.region.end.point.line);
                (a.min(b), a.max(b))
            },
        };

        let range_top = match range.start_bound() {
            Bound::Included(&range_start) => range_start,
            Bound::Excluded(&range_start) => range_start + 1,
            Bound::Unbounded => Line(i32::MIN),
        };

        let range_bottom = match range.end_bound() {
            Bound::Included(&range_end) => range_end,
            Bound::Excluded(&range_end) => range_end - 1,
            Bound::Unbounded => Line(i32::MAX),
        };

        range_bottom >= start && range_top <= end
    }

    /// Expand selection sides to include all cells.
    pub fn include_all(&mut self) {
        let (start, end) = (self.region.start.point, self.region.end.point);
        let (start_side, end_side) = match self.ty {
            SelectionType::Block
                if start.column > end.column
                    || (start.column == end.column && start.line > end.line) =>
            {
                (Side::Right, Side::Left)
            },
            SelectionType::Block => (Side::Left, Side::Right),
            _ if start > end => (Side::Right, Side::Left),
            _ => (Side::Left, Side::Right),
        };

        self.region.start.side = start_side;
        self.region.end.side = end_side;
    }

    /// Convert selection to grid coordinates.
    pub fn to_range<T>(&self, term: &Term<T>) -> Option<SelectionRange> {
        if let Some(extent) = self.current_extent() {
            return (extent.end.line >= term.topmost_line() && extent.start.line <= term.bottommost_line())
                .then(|| SelectionRange::new(
                    extent.start.grid_clamp(term, Boundary::Grid),
                    extent.end.grid_clamp(term, Boundary::Grid),
                    false,
                ));
        }

        let grid = term.grid();
        let columns = grid.columns();

        // Order start above the end.
        let mut start = self.region.start;
        let mut end = self.region.end;

        if start.point > end.point {
            mem::swap(&mut start, &mut end);
        }

        // Clamp selection to within grid boundaries.
        if end.point.line < term.topmost_line() {
            return None;
        }
        start.point = start.point.grid_clamp(term, Boundary::Grid);
        end.point = end.point.grid_clamp(term, Boundary::Grid);

        match self.ty {
            SelectionType::Simple => self.range_simple(start, end, columns),
            SelectionType::Block => Self::range_block(start, end),
            SelectionType::Semantic => Some(Self::range_semantic(term, start.point, end.point)),
            SelectionType::Lines => Some(Self::range_lines(term, start.point, end.point)),
        }
    }

    fn range_semantic<T>(term: &Term<T>, mut start: Point, mut end: Point) -> SelectionRange {
        if start == end {
            if let Some(matching) = term.bracket_search(start) {
                if (matching.line == start.line && matching.column < start.column)
                    || (matching.line < start.line)
                {
                    start = matching;
                } else {
                    end = matching;
                }

                return SelectionRange { start, end, is_block: false };
            }
        }

        let start = term.semantic_search_left(start);
        let end = term.semantic_search_right(end);

        SelectionRange { start, end, is_block: false }
    }

    fn range_lines<T>(term: &Term<T>, start: Point, end: Point) -> SelectionRange {
        let start = term.line_search_left(start);
        let end = term.line_search_right(end);

        SelectionRange { start, end, is_block: false }
    }

    fn range_simple(
        &self,
        mut start: Anchor,
        mut end: Anchor,
        columns: usize,
    ) -> Option<SelectionRange> {
        if self.is_empty() {
            return None;
        }

        // Remove last cell if selection ends to the left of a cell.
        if end.side == Side::Left && start.point != end.point {
            // Special case when selection ends to left of first cell.
            if end.point.column == 0 {
                end.point.column = Column(columns - 1);
                end.point.line -= 1;
            } else {
                end.point.column -= 1;
            }
        }

        // Remove first cell if selection starts at the right of a cell.
        if start.side == Side::Right && start.point != end.point {
            start.point.column += 1;

            // Wrap to next line when selection starts to the right of last column.
            if start.point.column == columns {
                start.point.column = Column(0);
                start.point.line += 1;
            }
        }

        Some(SelectionRange { start: start.point, end: end.point, is_block: false })
    }

    fn range_block(start: Anchor, end: Anchor) -> Option<SelectionRange> {
        // Each anchor lies on a cell edge. Opposite edges of the same column include that
        // column; coincident edges delimit an empty rectangle in either drag direction.
        let (a, b) = (start.column_edge(), end.column_edge());
        (a != b).then(|| SelectionRange {
            start: Point::new(start.point.line, Column(a.min(b))),
            end: Point::new(end.point.line, Column(a.max(b) - 1)),
            is_block: true,
        })
    }
}
