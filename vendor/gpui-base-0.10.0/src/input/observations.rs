// RSQL local source changes. See NOTICE for upstream provenance and modifications.
//! Read-only measurements of native editor shaping, layout and paint.

use gpui::{Hsla, Pixels, Point};
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, Default)]
pub struct ProjectionWork {
    pub measured_bytes: usize,
    pub shaped_rows: usize,
    pub projections: usize,
    pub intersecting_styles: usize,
    pub indexed_styles: usize,
    pub comparisons: usize,
}

thread_local! {
    pub(crate) static GUIDE_VISITS: Cell<usize> = const { Cell::new(0) };
    pub(crate) static LAYOUTS: Cell<usize> = const { Cell::new(0) };
    static WORK: Cell<ProjectionWork> = Cell::default();
    static GUTTER: RefCell<Option<Vec<(Point<Pixels>, Hsla)>>> = const { RefCell::new(None) };
}

pub fn guide_visits() -> usize {
    GUIDE_VISITS.get()
}

pub fn take_projection_work() -> ProjectionWork {
    WORK.take()
}

pub(crate) fn measure(update: impl FnOnce(&mut ProjectionWork)) {
    let mut work = WORK.get();
    update(&mut work);
    WORK.set(work);
}

pub fn take_layout_count() -> usize {
    LAYOUTS.take()
}

pub fn observe_gutter(frame: impl FnOnce()) -> Vec<(Point<Pixels>, Hsla)> {
    GUTTER.set(Some(Vec::new()));
    frame();
    GUTTER.take().unwrap()
}

pub(crate) fn painted_line_number(position: Point<Pixels>, color: Hsla) {
    GUTTER.with_borrow_mut(|records| {
        if let Some(records) = records {
            records.push((position, color));
        }
    });
}
