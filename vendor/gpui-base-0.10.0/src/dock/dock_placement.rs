//! Pure size arithmetic for resizing one dock (left/right/bottom), and the
//! dock's runtime state (open/collapsible/size/resizing).
//!
//! This module decides *how big a dock is allowed to be*. It draws nothing:
//! the resize-handle chrome and collapsed/expanded presentation live in
//! `crates/component`.

use gpui::{Bounds, Pixels, Point, px};

use crate::PANEL_MIN_SIZE;

use super::state::DockPlacement;

/// Pure arithmetic for resizing one dock. The caller supplies the area
/// bounds and the opposite dock's size; base does not reach across entities
/// to find them — the original `Dock::resize` read sibling dock sizes
/// straight off the (application-owned) `DockArea` entity, which base has no
/// way to do.
#[derive(Clone, Copy, Debug)]
pub struct DockSizing {
    placement: DockPlacement,
    area: Bounds<Pixels>,
    opposite_dock_size: Pixels,
}

impl DockSizing {
    pub fn new(placement: DockPlacement) -> Self {
        Self {
            placement,
            area: Bounds::default(),
            opposite_dock_size: px(0.),
        }
    }

    /// Set the full area bounds (origin and size). Needed whenever the
    /// dock's placement depends on the area's origin, e.g. a bottom dock
    /// measuring from the area's bottom edge.
    pub fn with_area_bounds(mut self, area: Bounds<Pixels>) -> Self {
        self.area = area;
        self
    }

    /// Set only the area's width, leaving its origin and height untouched.
    /// Convenient for left/right dock clamping, which only reads
    /// `area.size.width`.
    pub fn with_area_width(mut self, width: Pixels) -> Self {
        self.area.size.width = width;
        self
    }

    /// Set only the area's height, leaving its origin and width untouched.
    /// Convenient for bottom dock clamping, which only reads
    /// `area.size.height`.
    pub fn with_area_height(mut self, height: Pixels) -> Self {
        self.area.size.height = height;
        self
    }

    /// Set the size of the dock on the opposite side (right, for a left
    /// dock; left, for a right dock). Bottom docks have no opposite side.
    pub fn with_opposite_dock_size(mut self, size: Pixels) -> Self {
        self.opposite_dock_size = size;
        self
    }

    /// The dock size the pointer implies, before clamping.
    ///
    /// `DockPlacement::Center` names the canvas, which has no edge to measure
    /// from and no dock size, so it answers zero. Both this and [`Self::clamp`]
    /// are public and take a `pub` enum, so a caller can name that variant; a
    /// base-layer arithmetic helper must not panic a desktop application over
    /// it. [`DockPlacement::axis`] resolves the same variant the same way,
    /// silently rather than by panicking.
    pub fn size_from_pointer(&self, pointer: Point<Pixels>) -> Pixels {
        match self.placement {
            DockPlacement::Left => pointer.x - self.area.left(),
            DockPlacement::Right => self.area.right() - pointer.x,
            DockPlacement::Bottom => self.area.bottom() - pointer.y,
            DockPlacement::Center => px(0.),
        }
    }

    /// Clamp a size into the range this dock may occupy: never below
    /// `PANEL_MIN_SIZE`, and never so large it would squeeze the opposite
    /// dock (if any) below `PANEL_MIN_SIZE` either. The `.max(PANEL_MIN_SIZE)`
    /// on the computed maximum matters when the area itself is narrower than
    /// both minimums combined — it keeps the clamp range non-empty.
    ///
    /// `DockPlacement::Center` constrains nothing, so it hands the size back
    /// unchanged. See [`Self::size_from_pointer`] for why that variant is
    /// answered rather than rejected.
    pub fn clamp(&self, size: Pixels) -> Pixels {
        let max_size = match self.placement {
            DockPlacement::Left | DockPlacement::Right => {
                (self.area.size.width - PANEL_MIN_SIZE - self.opposite_dock_size)
                    .max(PANEL_MIN_SIZE)
            }
            DockPlacement::Bottom => (self.area.size.height - PANEL_MIN_SIZE).max(PANEL_MIN_SIZE),
            DockPlacement::Center => return size,
        };
        size.clamp(PANEL_MIN_SIZE, max_size)
    }
}

/// Runtime state for one dock: whether it is open, collapsible, its current
/// size, and whether it is mid-resize.
///
/// This does not include the dock's placement or its panel content. `DockArea`
/// owns one of these per dock, paired with that dock's `PaneTree` and keyed
/// by its [`DockPlacement`]; the placement is the key, and the content is the
/// tree.
#[derive(Clone, Copy, Debug)]
pub struct Dock {
    open: bool,
    collapsible: bool,
    size: Pixels,
    live_size: Option<Pixels>,
    resizing: bool,
}

impl Dock {
    pub fn new(size: Pixels) -> Self {
        Self {
            open: true,
            collapsible: true,
            size,
            live_size: None,
            resizing: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    pub fn is_collapsible(&self) -> bool {
        self.collapsible
    }

    pub fn set_collapsible(&mut self, collapsible: bool) {
        self.collapsible = collapsible;
    }

    pub fn size(&self) -> Pixels {
        self.size
    }

    /// Set the dock's size, never below [`PANEL_MIN_SIZE`].
    ///
    /// The floor is here rather than at the call sites because
    /// `DockArea::set_dock_size` is public and unclamped: a smaller value
    /// would collapse the dock to nothing, the skin clips the resize handle
    /// that would drag it back out, and the collapsed size persists.
    pub fn set_size(&mut self, size: Pixels) {
        self.size = size.max(PANEL_MIN_SIZE);
    }

    /// A size below [`PANEL_MIN_SIZE`] that a drag in progress is showing.
    /// It is never persisted: the drag's release settles it into either a
    /// closed dock or one at the minimum.
    pub(crate) fn live_size(&self) -> Option<Pixels> {
        self.live_size
    }

    pub(crate) fn set_live_size(&mut self, size: Option<Pixels>) {
        self.live_size = size;
    }

    pub fn is_resizing(&self) -> bool {
        self.resizing
    }

    pub fn set_resizing(&mut self, resizing: bool) {
        self.resizing = resizing;
    }
}
