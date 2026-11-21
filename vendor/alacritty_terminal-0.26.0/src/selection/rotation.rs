use super::{Anchor, Selection, SelectionRange, SelectionType};
use crate::grid::Dimensions;
use crate::index::{Column, Line, Point, Side};
use crate::term::Term;
use std::ops::Range;

impl Selection {
    pub(crate) fn native_scroll_range<T>(&self, term: &Term<T>) -> Option<SelectionRange> {
        if self.current_extent().is_some()
            || !matches!(self.ty, SelectionType::Semantic | SelectionType::Lines)
        {
            return None;
        }

        // A surviving hard row can gain a soft-wrapped neighbor when intervening rows move.
        self.to_range(term)
    }

    pub(crate) fn rotate_scrolled<T>(
        mut self,
        term: &Term<T>,
        region: &Range<Line>,
        delta: i32,
        native: Option<SelectionRange>,
    ) -> Option<Self> {
        if self.current_extent().is_some()
            || !matches!(self.ty, SelectionType::Semantic | SelectionType::Lines)
        {
            return self.rotate(term, region, delta);
        }
        self.extent = None;
        let (mut selection, retained, intact) = self.rotate_extent(term, region, delta, native?)?;
        if !intact || selection.to_range(term) != Some(retained) {
            selection.extent = Some((selection.ty, retained));
        }
        Some(selection)
    }

    // Dimensions describe the grid after scrolling, including the actual retained history.
    pub(super) fn rotate_extent<D: Dimensions>(
        mut self,
        grid: &D,
        region: &Range<Line>,
        delta: i32,
        old: SelectionRange,
    ) -> Option<(Self, SelectionRange, bool)> {
        let top = grid.topmost_line();
        let scrolls_history = region.start == 0 && delta > 0;
        // Intersect the inclusive extent with the source rows that survive: the stationary
        // rows above the region, the shifted rows, and the stationary rows below it.
        let fixed_end = if scrolls_history { top } else { region.start };
        let moved_start = if scrolls_history {
            top + delta
        } else {
            region.start + delta.max(0)
        };
        let bands = [
            (top..fixed_end, 0),
            (moved_start..region.end + delta.min(0), -delta),
            (region.end..grid.bottommost_line() + 1, 0),
        ];
        let mut intact = false;
        let mut retained = None::<SelectionRange>;
        for (rows, shift) in &bands {
            if rows.is_empty() {
                continue;
            }
            let mut start = old.start.max(Point::new(rows.start, Column(0)));
            let mut end = old.end.min(Point::new(rows.end - 1, grid.last_column()));
            if start <= end {
                intact |= start == old.start && end == old.end;
                start.line += *shift;
                end.line += *shift;
                match &mut retained {
                    Some(extent) => extent.end = end,
                    None => retained = Some(SelectionRange::new(start, end, false)),
                }
            }
        }
        let retained = retained?;
        for anchor in [&mut self.region.start, &mut self.region.end] {
            if let Some((_, shift)) = bands
                .iter()
                .find(|(rows, _)| rows.contains(&anchor.point.line))
            {
                anchor.point.line += *shift;
            } else {
                // This gesture cell was discarded. Use the surviving edge on that side.
                *anchor = if delta > 0 {
                    Anchor::new(retained.start, Side::Left)
                } else {
                    Anchor::new(retained.end, Side::Right)
                };
            }
        }
        Some((self, retained, intact))
    }
}
