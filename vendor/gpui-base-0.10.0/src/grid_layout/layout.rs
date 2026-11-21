//! The arithmetic of a grid layout: where items may sit, what pushes what, and
//! how a layout settles. Everything here is pure, in grid cells, and knows
//! nothing about pixels, entities or rendering.

use gpui::{Axis, SharedString};
use serde::{Deserialize, Serialize};

/// Where one item sits in a grid layout, in zero-based grid cells.
///
/// `col` and `row` are the item's leading column and top row, counted from
/// zero; `col_span` and `row_span` are how many columns and rows it covers.
/// Unlike GPUI's `col_start`, which names a one-based CSS grid line, `col` is
/// a cell index, so a layout reads the same in code and in its saved form.
///
/// A layout is a list of areas keyed by `id`, which is what an application
/// persists to restore a user's arrangement. Every field after the place has a
/// serde default, so a layout saved with only the place still loads.
///
/// ```ignore
/// GridArea::new("revenue").col_span(6).row_span(4).min_span(3, 2)
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GridArea {
    /// The stable identity that pairs this area with its [`GridItem`](super::GridItem).
    pub id: SharedString,
    /// The leading column, from zero.
    pub col: usize,
    /// The top row, from zero.
    pub row: usize,
    /// The number of columns covered; at least 1.
    pub col_span: usize,
    /// The number of rows covered; at least 1.
    pub row_span: usize,
    /// The fewest columns a resize may leave the item.
    #[serde(default = "one")]
    pub min_col_span: usize,
    /// The fewest rows a resize may leave the item.
    #[serde(default = "one")]
    pub min_row_span: usize,
    /// The most columns a resize may give the item; `None` allows the full width.
    #[serde(default)]
    pub max_col_span: Option<usize>,
    /// The most rows a resize may give the item; `None` is unbounded.
    #[serde(default)]
    pub max_row_span: Option<usize>,
    /// A locked item is neither dragged, resized nor pushed; other items flow
    /// around it.
    #[serde(default)]
    pub locked: bool,
}

fn one() -> usize {
    1
}

impl GridArea {
    /// A one-cell area at the top-leading corner; place and size it with the
    /// builders.
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            col: 0,
            row: 0,
            col_span: 1,
            row_span: 1,
            min_col_span: 1,
            min_row_span: 1,
            max_col_span: None,
            max_row_span: None,
            locked: false,
        }
    }

    /// The leading column, from zero.
    pub fn col(mut self, col: usize) -> Self {
        self.col = col;
        self
    }

    /// The top row, from zero.
    pub fn row(mut self, row: usize) -> Self {
        self.row = row;
        self
    }

    /// How many columns the item covers.
    pub fn col_span(mut self, col_span: usize) -> Self {
        self.col_span = col_span.max(1);
        self
    }

    /// How many rows the item covers.
    pub fn row_span(mut self, row_span: usize) -> Self {
        self.row_span = row_span.max(1);
        self
    }

    /// The fewest columns and rows a resize may leave the item.
    pub fn min_span(mut self, col_span: usize, row_span: usize) -> Self {
        self.min_col_span = col_span.max(1);
        self.min_row_span = row_span.max(1);
        self
    }

    /// The most columns and rows a resize may give the item.
    pub fn max_span(mut self, col_span: usize, row_span: usize) -> Self {
        self.max_col_span = Some(col_span.max(1));
        self.max_row_span = Some(row_span.max(1));
        self
    }

    /// Lock the item in place; see [`GridArea::locked`].
    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// The row just below the item.
    pub(crate) fn row_end(&self) -> usize {
        self.row + self.row_span
    }

    /// The column just past the item's trailing edge.
    pub(crate) fn col_end(&self) -> usize {
        self.col + self.col_span
    }

    fn rect(&self) -> Rect {
        Rect {
            col: self.col,
            row: self.row,
            col_span: self.col_span,
            row_span: self.row_span,
        }
    }

    /// Clamp a requested span to the item's limits and the grid's width.
    pub(crate) fn clamp_size(&self, w: usize, h: usize, columns: usize) -> (usize, usize) {
        let max_col_span = self
            .max_col_span
            .unwrap_or(usize::MAX)
            .min(columns.saturating_sub(self.col))
            .max(1);
        let max_row_span = self.max_row_span.unwrap_or(usize::MAX).max(1);
        (
            w.clamp(self.min_col_span.min(max_col_span), max_col_span),
            h.clamp(self.min_row_span.min(max_row_span), max_row_span),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    col: usize,
    row: usize,
    col_span: usize,
    row_span: usize,
}

impl Rect {
    fn overlaps(&self, other: &Rect) -> bool {
        self.col < other.col + other.col_span
            && other.col < self.col + self.col_span
            && self.row < other.row + other.row_span
            && other.row < self.row + self.row_span
    }
}

/// How a layout responds to edits, shared by every operation in this module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayoutRules {
    pub columns: usize,
    /// The direction items float toward; `None` leaves them where they are put.
    pub compaction: Option<Axis>,
    /// Reject a move or resize that would overlap another item instead of
    /// pushing that item away.
    pub prevent_collision: bool,
}

/// The number of rows a layout occupies.
pub(crate) fn rows(items: &[GridArea]) -> usize {
    items.iter().map(GridArea::row_end).max().unwrap_or(0)
}

/// Bring a layout into a valid state: every item fits the columns, no locked
/// items overlap, and the remainder are compacted around them.
pub(crate) fn normalize(items: &mut [GridArea], rules: LayoutRules) {
    let columns = rules.columns.max(1);
    for item in items.iter_mut() {
        // An item's own limits first, which may have changed since it was
        // placed, then the grid's width.
        let max_col_span = item.max_col_span.unwrap_or(usize::MAX).max(1);
        let max_row_span = item.max_row_span.unwrap_or(usize::MAX).max(1);
        item.col_span = item
            .col_span
            .clamp(item.min_col_span.min(max_col_span), max_col_span);
        item.row_span = item
            .row_span
            .clamp(item.min_row_span.min(max_row_span), max_row_span);
        item.col_span = item.col_span.clamp(1, columns);
        item.row_span = item.row_span.max(1);
        if item.col_end() > columns {
            item.col = columns - item.col_span;
        }
    }

    // Locked items cannot be pushed by others, so settle them among
    // themselves first, in reading order.
    let locked = sorted_indices(items, rules.compaction)
        .into_iter()
        .filter(|&ix| items[ix].locked)
        .collect::<Vec<_>>();
    for (n, &ix) in locked.iter().enumerate() {
        while let Some(other) = first_collision(items, &locked[..n], items[ix].rect(), ix) {
            items[ix].row = items[other].row_end();
        }
    }

    settle(items, rules, None);
}

/// Compact the layout, or without compaction just push overlapping items
/// down. `priority` names an item that keeps its place when it overlaps
/// another — the one the user is moving.
fn settle(items: &mut [GridArea], rules: LayoutRules, priority: Option<usize>) {
    if rules.compaction.is_some() {
        compact(items, rules);
        return;
    }
    let mut placed = (0..items.len())
        .filter(|&ix| items[ix].locked)
        .collect::<Vec<_>>();
    if let Some(priority) = priority.filter(|&ix| !items[ix].locked) {
        placed.push(priority);
    }
    for ix in sorted_indices(items, None) {
        if placed.contains(&ix) {
            continue;
        }
        while let Some(other) = first_collision(items, &placed, items[ix].rect(), ix) {
            items[ix].row = items[other].row_end();
        }
        placed.push(ix);
    }
}

/// The layout `areas` would take over `columns` columns, for display only:
/// every area fitted to the width and the whole compacted upward in reading
/// order. Locks are dropped from the copy, since a display reflow pins nothing
/// — a locked area pinned to a row of the wide layout would only open gaps in
/// the narrow one. `areas` itself is left as it is.
pub(crate) fn reflowed(areas: &[GridArea], columns: usize) -> Vec<GridArea> {
    let mut reflowed = areas.to_vec();
    // Fitting to fewer columns moves areas to the leading edge, and areas
    // that shared a row would then tie in the compaction order. Order the copy
    // as the wide layout reads first; compaction sorts stably, so ties keep it.
    reflowed.sort_by_key(|area| (area.row, area.col));
    for area in &mut reflowed {
        area.locked = false;
    }
    normalize(
        &mut reflowed,
        LayoutRules {
            columns,
            compaction: Some(Axis::Vertical),
            prevent_collision: false,
        },
    );
    reflowed
}

/// Float every unlocked item toward `axis` as far as the others allow.
pub(crate) fn compact(items: &mut [GridArea], rules: LayoutRules) {
    let Some(axis) = rules.compaction else {
        return;
    };
    let columns = rules.columns.max(1);
    let mut placed = (0..items.len())
        .filter(|&ix| items[ix].locked)
        .collect::<Vec<_>>();
    for ix in sorted_indices(items, Some(axis)) {
        if items[ix].locked {
            continue;
        }
        compact_item(items, &placed, ix, axis, columns);
        placed.push(ix);
    }
}

fn compact_item(items: &mut [GridArea], placed: &[usize], ix: usize, axis: Axis, columns: usize) {
    match axis {
        Axis::Vertical => {
            // Start no lower than the bottom of what is already placed, so a
            // tall gap closes in one step instead of row by row.
            let floor = placed
                .iter()
                .map(|&p| items[p].row_end())
                .max()
                .unwrap_or(0);
            items[ix].row = items[ix].row.min(floor);
            loop {
                while items[ix].row > 0 {
                    let mut probe = items[ix].rect();
                    probe.row -= 1;
                    if first_collision(items, placed, probe, ix).is_some() {
                        break;
                    }
                    items[ix].row -= 1;
                }
                match first_collision(items, placed, items[ix].rect(), ix) {
                    Some(other) => items[ix].row = items[other].row_end(),
                    None => break,
                }
            }
        }
        Axis::Horizontal => loop {
            while items[ix].col > 0 {
                let mut probe = items[ix].rect();
                probe.col -= 1;
                if first_collision(items, placed, probe, ix).is_some() {
                    break;
                }
                items[ix].col -= 1;
            }
            // Past the trailing edge, wrap to the next row and try again from
            // the leading edge.
            if items[ix].col_end() > columns {
                items[ix].col = 0;
                items[ix].row += 1;
                continue;
            }
            match first_collision(items, placed, items[ix].rect(), ix) {
                Some(other) => {
                    items[ix].col = items[other].col_end();
                    if items[ix].col_end() > columns {
                        items[ix].col = 0;
                        items[ix].row += 1;
                    }
                }
                None => break,
            }
        },
    }
}

/// Move the item `id` to `(x, y)`, pushing whatever it lands on out of the
/// way, then compact. Returns `None` when the rules refuse the move.
///
/// The move is computed from `items` as they were when the gesture began, not
/// from the previous step of the gesture, so items the pointer merely passed
/// over return to where they were.
pub(crate) fn move_item(
    items: &[GridArea],
    id: &str,
    x: usize,
    y: usize,
    rules: LayoutRules,
) -> Option<Vec<GridArea>> {
    let ix = items.iter().position(|item| item.id.as_ref() == id)?;
    if items[ix].locked {
        return None;
    }
    let mut next = items.to_vec();
    let x = x.min(rules.columns.saturating_sub(next[ix].col_span));
    if rules.prevent_collision {
        let mut probe = next[ix].rect();
        probe.col = x;
        probe.row = y;
        if any_collision(&next, probe, ix) {
            return None;
        }
    }
    Push::new(rules, next.len()).move_element(&mut next, ix, Some(x), Some(y), true);
    settle(&mut next, rules, Some(ix));
    Some(next)
}

/// Resize the item `id` to span `w` by `h`, clamped to its limits, pushing
/// whatever it grows over, then compact. Returns `None` when the rules refuse
/// the resize.
pub(crate) fn resize_item(
    items: &[GridArea],
    id: &str,
    w: usize,
    h: usize,
    rules: LayoutRules,
) -> Option<Vec<GridArea>> {
    let ix = items.iter().position(|item| item.id.as_ref() == id)?;
    if items[ix].locked {
        return None;
    }
    let mut next = items.to_vec();
    let (w, h) = next[ix].clamp_size(w, h, rules.columns);

    // A locked neighbour is a wall. Rather than refuse the whole gesture, try
    // keeping the other dimension where it started.
    let start = next[ix].clone();
    let fits = |w: usize, h: usize, next: &[GridArea]| {
        let probe = Rect {
            col_span: w,
            row_span: h,
            ..start.rect()
        };
        !next
            .iter()
            .enumerate()
            .any(|(other, item)| other != ix && item.locked && probe.overlaps(&item.rect()))
    };
    let (w, h) = [(w, h), (w, start.row_span), (start.col_span, h)]
        .into_iter()
        .find(|&(w, h)| fits(w, h, &next))?;

    next[ix].col_span = w;
    next[ix].row_span = h;
    if rules.prevent_collision && any_collision(&next, next[ix].rect(), ix) {
        return None;
    }

    let mut push = Push::new(rules, next.len());
    push.moved[ix] = true;
    for other in collisions(&next, sorted_indices(&next, rules.compaction), ix) {
        let below = next[ix].row_end();
        push.move_element(&mut next, other, None, Some(below), false);
    }
    settle(&mut next, rules, Some(ix));
    Some(next)
}

/// The cascade of a move: an item that lands on others pushes them, and they
/// push the items they land on in turn.
struct Push {
    rules: LayoutRules,
    /// Items already settled by this move, which later collisions leave alone.
    moved: Vec<bool>,
}

impl Push {
    fn new(rules: LayoutRules, len: usize) -> Self {
        Self {
            rules,
            moved: vec![false; len],
        }
    }

    fn move_element(
        &mut self,
        items: &mut [GridArea],
        ix: usize,
        x: Option<usize>,
        y: Option<usize>,
        user_action: bool,
    ) {
        if items[ix].locked {
            return;
        }
        let (old_x, old_y) = (items[ix].col, items[ix].row);
        let x = x.unwrap_or(old_x);
        let y = y.unwrap_or(old_y);
        if (x, y) == (old_x, old_y) {
            return;
        }
        items[ix].col = x;
        items[ix].row = y;
        self.moved[ix] = true;

        // Moving toward the compaction direction, resolve the nearest
        // collisions last so they win the space the item vacated.
        let mut order = sorted_indices(items, self.rules.compaction);
        let moving_back = match self.rules.compaction {
            Some(Axis::Horizontal) => old_x >= x,
            _ => old_y >= y,
        };
        if moving_back {
            order.reverse();
        }

        for other in collisions(items, order, ix) {
            if self.moved[other] {
                continue;
            }
            if items[other].locked {
                // A locked item cannot give way, so the mover does.
                self.away(items, other, ix, user_action);
            } else {
                self.away(items, ix, other, user_action);
            }
        }
    }

    /// Move `mover` clear of `obstacle`.
    fn away(&mut self, items: &mut [GridArea], obstacle: usize, mover: usize, user_action: bool) {
        let horizontal = self.rules.compaction == Some(Axis::Horizontal);
        if user_action {
            // A dragged item that lands on another first tries to trade places
            // with it: if the space on the near side of the obstacle is free,
            // the pushed item goes there instead of being shoved further away.
            let (o, m) = (&items[obstacle], &items[mover]);
            let probe = Rect {
                col: if horizontal {
                    o.col.saturating_sub(m.col_span)
                } else {
                    m.col
                },
                row: if horizontal {
                    m.row
                } else {
                    o.row.saturating_sub(m.row_span)
                },
                col_span: m.col_span,
                row_span: m.row_span,
            };
            let blocker = (0..items.len()).find(|&other| {
                other != mover && other != obstacle && probe.overlaps(&items[other].rect())
            });
            let obstacle_rect = items[obstacle].rect();
            match blocker {
                None if !probe.overlaps(&obstacle_rect) => {
                    let (x, y) = if horizontal {
                        (Some(probe.col), None)
                    } else {
                        (None, Some(probe.row))
                    };
                    return self.move_element(items, mover, x, y, false);
                }
                _ => {}
            }
        }

        // Otherwise push it past the obstacle along the compaction axis.
        let (x, y) = if horizontal {
            (Some(items[obstacle].col_end()), None)
        } else {
            (None, Some(items[obstacle].row_end()))
        };
        self.move_element(items, mover, x, y, false);
    }
}

/// Item indices in the order compaction visits them: row-major for vertical
/// layouts and column-major for horizontal ones.
fn sorted_indices(items: &[GridArea], compaction: Option<Axis>) -> Vec<usize> {
    let mut order = (0..items.len()).collect::<Vec<_>>();
    match compaction {
        Some(Axis::Horizontal) => order.sort_by_key(|&ix| (items[ix].col, items[ix].row)),
        _ => order.sort_by_key(|&ix| (items[ix].row, items[ix].col)),
    }
    order
}

fn first_collision(items: &[GridArea], among: &[usize], rect: Rect, skip: usize) -> Option<usize> {
    among
        .iter()
        .copied()
        .find(|&other| other != skip && rect.overlaps(&items[other].rect()))
}

fn any_collision(items: &[GridArea], rect: Rect, skip: usize) -> bool {
    items
        .iter()
        .enumerate()
        .any(|(other, item)| other != skip && rect.overlaps(&item.rect()))
}

fn collisions(items: &[GridArea], order: Vec<usize>, ix: usize) -> Vec<usize> {
    let rect = items[ix].rect();
    order
        .into_iter()
        .filter(|&other| other != ix && rect.overlaps(&items[other].rect()))
        .collect()
}
