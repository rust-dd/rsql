use std::{cell::Cell, ops::Deref, panic::Location, rc::Rc};

use instant::{Duration, Instant};

use crate::{
    AxisExt,
    animation::{ease_in_cubic, ease_out_cubic},
    theme::ActiveTheme as _,
};
use gpui::{
    Anchor, App, Axis, Background, BorderStyle, Bounds, ContentMask, CursorStyle, Edges, Element,
    ElementId, EntityId, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IsZero,
    LayoutId, ListState, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    Position, ScrollEvent, ScrollHandle, Size, Style, TouchDragEvent, TouchPhase,
    UniformListScrollHandle, Window, fill, point, prelude::FluentBuilder, px, relative, size,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The width of the scrollbar (THUMB_ACTIVE_INSET * 2 + THUMB_ACTIVE_WIDTH)
const WIDTH: Pixels = px(4. * 2. + 8.);
const MIN_THUMB_SIZE: Pixels = px(48.);

const THUMB_WIDTH: Pixels = px(6.);
const THUMB_RADIUS: Pixels = Pixels::ZERO;
const THUMB_INSET: Pixels = px(4.);

const THUMB_ACTIVE_WIDTH: Pixels = px(8.);
const THUMB_ACTIVE_RADIUS: Pixels = Pixels::ZERO;
const THUMB_ACTIVE_INSET: Pixels = px(4.);

/// How long visibility is held after the last activity, when the styled layer
/// projects no [`ScrollbarMotion`] of its own.
///
/// This is a visibility hold rather than motion: without it [`ScrollbarMode::Scrolling`]
/// could never reveal the scrollbar.
const DEFAULT_IDLE: Duration = Duration::from_secs(2);

fn clamp_thumb_radius(radius: Pixels, bounds: Bounds<Pixels>) -> Pixels {
    radius
        .min(bounds.size.width / 2.)
        .min(bounds.size.height / 2.)
}

/// Scrollbar show mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash, Default, JsonSchema)]
pub enum ScrollbarMode {
    /// Show scrollbar when scrolling, will fade out after idle.
    #[default]
    Scrolling,
    /// Show scrollbar on hover.
    Hover,
    /// Always show scrollbar.
    Always,
}

impl ScrollbarMode {
    fn is_hover(&self) -> bool {
        matches!(self, Self::Hover)
    }

    fn is_always(&self) -> bool {
        matches!(self, Self::Always)
    }
}

/// A trait for scroll handles that can get and set offset.
pub trait ScrollbarHandle: 'static {
    /// Bounds of the viewport the scrollbar overlays.
    fn viewport_bounds(&self) -> Bounds<Pixels>;
    /// Get the current offset of the scroll handle.
    fn offset(&self) -> Point<Pixels>;
    /// Set the offset of the scroll handle.
    fn set_offset(&self, offset: Point<Pixels>);
    /// The full size of the content, including padding.
    fn content_size(&self) -> Size<Pixels>;
    /// Called when start dragging the scrollbar thumb.
    fn start_drag(&self) {}
    /// Called when end dragging the scrollbar thumb.
    fn end_drag(&self) {}
}

impl ScrollbarHandle for ScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        self.offset()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.set_offset(offset);
    }

    fn content_size(&self) -> Size<Pixels> {
        (self.max_offset() + self.bounds().size.into()).into()
    }
}

impl ScrollbarHandle for UniformListScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().base_handle.bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        self.0.borrow().base_handle.offset()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.0.borrow_mut().base_handle.set_offset(offset)
    }

    fn content_size(&self) -> Size<Pixels> {
        let base_handle = &self.0.borrow().base_handle;
        (base_handle.max_offset() + base_handle.bounds().size.into()).into()
    }
}

impl ScrollbarHandle for ListState {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        ListState::viewport_bounds(self)
    }

    fn offset(&self) -> Point<Pixels> {
        self.scroll_px_offset_for_scrollbar()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.set_offset_from_scrollbar(offset);
    }

    fn content_size(&self) -> Size<Pixels> {
        self.viewport_bounds().size + self.max_offset_for_scrollbar().into()
    }

    fn start_drag(&self) {
        self.scrollbar_drag_started();
    }

    fn end_drag(&self) {
        self.scrollbar_drag_ended();
    }
}

#[doc(hidden)]
#[derive(Debug, Clone)]
struct ScrollbarState(Rc<Cell<ScrollbarStateInner>>);

#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
struct ScrollbarStateInner {
    hovered_axis: Option<Axis>,
    hovered_on_thumb: Option<Axis>,
    dragged_axis: Option<Axis>,
    drag_pos: Point<Pixels>,
    last_scroll_offset: Point<Pixels>,
    last_scroll_time: Option<Instant>,
    // Last update offset
    last_update: Instant,
    drag_update_scheduled: bool,
    drag_update_generation: u64,
    idle_timer_scheduled: bool,
    visibility: VisibilityAnimation,
    vertical_width: WidthAnimation,
    horizontal_width: WidthAnimation,
}

impl Default for ScrollbarState {
    fn default() -> Self {
        let now = Instant::now();
        Self(Rc::new(Cell::new(ScrollbarStateInner {
            hovered_axis: None,
            hovered_on_thumb: None,
            dragged_axis: None,
            drag_pos: point(px(0.), px(0.)),
            last_scroll_offset: point(px(0.), px(0.)),
            last_scroll_time: None,
            last_update: now,
            drag_update_scheduled: false,
            drag_update_generation: 0,
            idle_timer_scheduled: false,
            visibility: VisibilityAnimation::hidden(now),
            vertical_width: WidthAnimation::new(now),
            horizontal_width: WidthAnimation::new(now),
        })))
    }
}

#[derive(Debug, Clone, Copy)]
struct ScalarTransition<T> {
    from: T,
    target: T,
    started_at: Instant,
    duration: Duration,
}

impl<T: Copy + PartialEq> ScalarTransition<T> {
    fn settled(value: T, now: Instant) -> Self {
        Self {
            from: value,
            target: value,
            started_at: now,
            duration: Duration::ZERO,
        }
    }

    fn sample(&self, now: Instant, interpolate: impl FnOnce(T, T, f32) -> T) -> (T, bool) {
        if self.from == self.target || self.duration.is_zero() {
            return (self.target, false);
        }
        let linear = now.saturating_duration_since(self.started_at).as_secs_f32()
            / self.duration.as_secs_f32();
        if linear >= 1.0 {
            (self.target, false)
        } else {
            (
                interpolate(self.from, self.target, linear.clamp(0.0, 1.0)),
                true,
            )
        }
    }

    fn start(&mut self, from: T, target: T, duration: Duration, now: Instant) {
        self.from = from;
        self.target = target;
        self.started_at = now;
        self.duration = duration;
    }

    fn settle(&mut self, target: T, now: Instant) {
        self.start(target, target, Duration::ZERO, now);
    }
}

#[derive(Debug, Clone, Copy)]
struct WidthAnimation {
    transition: ScalarTransition<Pixels>,
    initialized: bool,
}

impl WidthAnimation {
    fn new(now: Instant) -> Self {
        Self {
            transition: ScalarTransition::settled(Pixels::ZERO, now),
            initialized: false,
        }
    }

    fn sample(&self, now: Instant) -> (Pixels, bool) {
        self.transition.sample(now, |from, target, linear| {
            from + (target - from) * ease_out_cubic(linear)
        })
    }

    /// Move toward `target` over `duration`. A zero duration adopts the target
    /// immediately, which is how reduced motion and a motionless theme arrive here.
    fn set_target(&mut self, target: Pixels, duration: Duration, now: Instant) -> (Pixels, bool) {
        if duration.is_zero() || !self.initialized {
            self.transition.settle(target, now);
            self.initialized = true;
        } else if self.transition.target != target {
            let from = self.sample(now).0;
            self.transition.start(from, target, duration, now);
        }
        self.sample(now)
    }
}

/// How a scrollbar becomes visible.
///
/// The styled layer chooses the choreography; Base only plays it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollbarEntrance {
    /// Fade in without moving.
    #[default]
    Fade,
    /// Slide in from the nearest edge while fading.
    SlideAndFade,
}

/// Motion tokens used by [`Scrollbar`].
///
/// Base installs no motion of its own: every transition duration defaults to
/// zero, so visibility and thumb width snap. Product timing belongs to the
/// styled layer, which projects it through [`crate::ScrollbarTheme`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarMotion {
    idle: Duration,
    enter: Duration,
    exit: Duration,
    expand: Duration,
    entrance: ScrollbarEntrance,
    thumb_hover_entrance: ScrollbarEntrance,
}

impl Default for ScrollbarMotion {
    fn default() -> Self {
        Self {
            idle: DEFAULT_IDLE,
            enter: Duration::ZERO,
            exit: Duration::ZERO,
            expand: Duration::ZERO,
            entrance: ScrollbarEntrance::Fade,
            thumb_hover_entrance: ScrollbarEntrance::Fade,
        }
    }
}

impl ScrollbarMotion {
    /// How long visibility is held after the last scroll, drag, or hover.
    pub fn with_idle(mut self, idle: Duration) -> Self {
        self.idle = idle;
        self
    }

    /// How long the scrollbar takes to become fully visible.
    pub fn with_enter(mut self, enter: Duration) -> Self {
        self.enter = enter;
        self
    }

    /// How long the scrollbar takes to fade away once the idle hold expires.
    pub fn with_exit(mut self, exit: Duration) -> Self {
        self.exit = exit;
        self
    }

    /// How long the thumb takes to reach a new width.
    pub fn with_expand(mut self, expand: Duration) -> Self {
        self.expand = expand;
        self
    }

    /// Which entrance choreography to play.
    pub fn with_entrance(mut self, entrance: ScrollbarEntrance) -> Self {
        self.entrance = entrance;
        self
    }

    /// Which entrance choreography to play when hover reveals the thumb.
    pub fn with_thumb_hover_entrance(mut self, entrance: ScrollbarEntrance) -> Self {
        self.thumb_hover_entrance = entrance;
        self
    }

    pub fn idle(&self) -> Duration {
        self.idle
    }

    pub fn enter(&self) -> Duration {
        self.enter
    }

    pub fn exit(&self) -> Duration {
        self.exit
    }

    pub fn expand(&self) -> Duration {
        self.expand
    }

    pub fn entrance(&self) -> ScrollbarEntrance {
        self.entrance
    }

    pub fn thumb_hover_entrance(&self) -> ScrollbarEntrance {
        self.thumb_hover_entrance
    }

    fn entrance_for(&self, mode: ScrollbarMode, thumb_hovered: bool) -> ScrollbarEntrance {
        if mode.is_hover() && thumb_hovered {
            self.thumb_hover_entrance
        } else {
            self.entrance
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct VisibilityAnimation {
    opacity: ScalarTransition<f32>,
    position: ScalarTransition<f32>,
    entrance: ScrollbarEntrance,
}

#[derive(Debug, Clone, Copy)]
struct VisibilitySample {
    opacity: f32,
    position: f32,
    running: bool,
}

impl VisibilityAnimation {
    fn hidden(now: Instant) -> Self {
        Self {
            opacity: ScalarTransition::settled(0.0, now),
            position: ScalarTransition::settled(0.0, now),
            entrance: ScrollbarEntrance::Fade,
        }
    }

    fn sample(&self, now: Instant) -> VisibilitySample {
        let entering =
            self.opacity.target > self.opacity.from || self.position.target > self.position.from;
        let (opacity, opacity_running) = self.opacity.sample(now, |from, target, linear| {
            let factor = if entering {
                linear
            } else {
                ease_in_cubic(linear)
            };
            from + (target - from) * factor
        });
        let (position, position_running) = self.position.sample(now, |from, target, linear| {
            let factor = if entering {
                ease_out_cubic(linear)
            } else {
                ease_in_cubic(linear)
            };
            from + (target - from) * factor
        });

        VisibilitySample {
            opacity,
            position,
            running: opacity_running || position_running,
        }
    }

    /// Reverse or start a transition toward `visible`.
    ///
    /// The leg runs for `enter` or `exit` scaled by the distance still to cover,
    /// so an interrupted transition keeps its speed instead of restarting.
    fn set_visible(
        &mut self,
        visible: bool,
        entrance: ScrollbarEntrance,
        enter: Duration,
        exit: Duration,
        now: Instant,
    ) {
        let target = if visible { 1.0 } else { 0.0 };
        let full_duration = if visible { enter } else { exit };
        if full_duration.is_zero() {
            // A motionless policy — reduced motion, an always-visible scrollbar,
            // or a theme that projects no motion — adopts the target outright,
            // even if a transition was in flight when the policy changed.
            self.opacity.settle(target, now);
            self.position.settle(target, now);
            self.entrance = entrance;
            return;
        }
        if self.opacity.target == target
            && self.position.target == target
            && self.entrance == entrance
        {
            return;
        }

        let sample = self.sample(now);
        let from_position = if visible && entrance == ScrollbarEntrance::Fade {
            1.0
        } else {
            sample.position
        };
        let distance = (target - sample.opacity)
            .abs()
            .max((target - from_position).abs());
        let duration = full_duration.mul_f32(distance);
        self.opacity.start(sample.opacity, target, duration, now);
        self.position.start(from_position, target, duration, now);
        self.entrance = entrance;
    }
}

fn visibility_translation(axis: Axis, track_width: Pixels, progress: f32) -> Point<Pixels> {
    let offset = track_width * (1.0 - progress.clamp(0.0, 1.0));
    if axis.is_vertical() {
        point(offset, px(0.))
    } else {
        point(px(0.), offset)
    }
}

fn wants_visible(
    mode: ScrollbarMode,
    is_hovered: bool,
    is_dragging: bool,
    last_scroll_time: Option<Instant>,
    idle: Duration,
    now: Instant,
) -> bool {
    mode.is_always()
        || is_dragging
        || (mode.is_hover() && is_hovered)
        || last_scroll_time.is_some_and(|last| now.saturating_duration_since(last) < idle)
}

fn tracks_thumb_hover(mode: ScrollbarMode, is_visible: bool) -> bool {
    mode.is_hover() || is_visible
}

fn hover_keeps_visible(mode: ScrollbarMode, is_hovered: bool, is_currently_visible: bool) -> bool {
    is_hovered && (mode.is_hover() || (mode == ScrollbarMode::Scrolling && is_currently_visible))
}

impl Deref for ScrollbarState {
    type Target = Rc<Cell<ScrollbarStateInner>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ScrollbarState {
    fn notify_drag(
        &self,
        now: Instant,
        interval: Duration,
        view: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut inner = self.get();
        if inner.drag_update_scheduled {
            return;
        }
        let elapsed = now.saturating_duration_since(inner.last_update);
        if elapsed >= interval {
            self.set(inner.with_last_update(now));
            cx.notify(view);
            return;
        }
        inner.drag_update_scheduled = true;
        self.set(inner);
        let generation = inner.drag_update_generation;
        let delay = interval - elapsed;
        let state = self.clone();
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(delay).await;
                let mut inner = state.get();
                // A release or new drag supersedes the pending trailing update.
                if !inner.drag_update_scheduled || inner.drag_update_generation != generation {
                    return;
                }
                inner.drag_update_scheduled = false;
                inner.last_update = Instant::now();
                state.set(inner);
                cx.update(|_, cx| cx.notify(view)).ok();
            })
            .detach();
    }
}

impl ScrollbarStateInner {
    fn with_drag_pos(&self, axis: Axis, pos: Point<Pixels>) -> Self {
        let mut state = *self;
        if axis.is_vertical() {
            state.drag_pos.y = pos.y;
        } else {
            state.drag_pos.x = pos.x;
        }

        state.dragged_axis = Some(axis);
        state.last_update = Instant::now();
        state.drag_update_scheduled = false;
        state.drag_update_generation = state.drag_update_generation.wrapping_add(1);
        state
    }

    fn with_unset_drag_pos(&self, now: Instant) -> Self {
        let mut state = *self;
        state.dragged_axis = None;
        state.drag_update_scheduled = false;
        state.drag_update_generation = state.drag_update_generation.wrapping_add(1);
        state.last_scroll_time = Some(now);
        state
    }

    fn with_hovered(&self, axis: Option<Axis>, now: Instant) -> Self {
        let mut state = *self;
        state.hovered_axis = axis;
        state.last_scroll_time = Some(now);
        state
    }

    fn with_hovered_on_thumb(&self, axis: Option<Axis>) -> Self {
        let mut state = *self;
        state.hovered_on_thumb = axis;
        if self.is_scrollbar_visible() {
            if axis.is_some() {
                state.last_scroll_time = Some(Instant::now());
            }
        }
        state
    }

    fn with_last_scroll(
        &self,
        last_scroll_offset: Point<Pixels>,
        last_scroll_time: Option<Instant>,
    ) -> Self {
        let mut state = *self;
        state.last_scroll_offset = last_scroll_offset;
        state.last_scroll_time = last_scroll_time;
        state
    }

    fn with_last_update(&self, t: Instant) -> Self {
        let mut state = *self;
        state.last_update = t;
        state
    }

    fn with_idle_timer_scheduled(&self, scheduled: bool) -> Self {
        let mut state = *self;
        state.idle_timer_scheduled = scheduled;
        state
    }

    fn is_scrollbar_visible(&self) -> bool {
        self.dragged_axis.is_some() || self.visibility.sample(Instant::now()).opacity > 0.0
    }
}

/// Scrollbar axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollbarAxis {
    /// Vertical scrollbar.
    Vertical,
    /// Horizontal scrollbar.
    Horizontal,
    /// Show both vertical and horizontal scrollbars.
    Both,
}

/// Paint-only styles for a scrollbar track.
#[derive(Clone, Default)]
pub struct ScrollbarTrackStyle {
    background: Option<Hsla>,
    border: Option<Hsla>,
    width: Option<Pixels>,
}

impl ScrollbarTrackStyle {
    pub fn bg(mut self, background: Hsla) -> Self {
        self.background = Some(background);
        self
    }

    pub fn border_color(mut self, border: Hsla) -> Self {
        self.border = Some(border);
        self
    }

    pub fn width(mut self, width: impl Into<Pixels>) -> Self {
        self.width = Some(width.into());
        self
    }
}

impl FluentBuilder for ScrollbarTrackStyle {}

/// Paint-only styles for a scrollbar thumb.
#[derive(Clone, Default)]
pub struct ScrollbarThumbStyle {
    background: Option<Background>,
    width: Option<Pixels>,
    inset: Option<Pixels>,
    radius: Option<Pixels>,
    min_length: Option<Pixels>,
}

impl ScrollbarThumbStyle {
    pub fn bg(mut self, background: impl Into<Background>) -> Self {
        self.background = Some(background.into());
        self
    }

    pub fn width(mut self, width: impl Into<Pixels>) -> Self {
        self.width = Some(width.into());
        self
    }

    pub fn inset(mut self, inset: impl Into<Pixels>) -> Self {
        self.inset = Some(inset.into());
        self
    }

    pub fn radius(mut self, radius: impl Into<Pixels>) -> Self {
        self.radius = Some(radius.into());
        self
    }

    pub fn min_length(mut self, min_length: impl Into<Pixels>) -> Self {
        self.min_length = Some(min_length.into());
        self
    }
}

impl FluentBuilder for ScrollbarThumbStyle {}

/// Typed paint styles supported by [`Scrollbar`].
#[derive(Clone, Default)]
pub struct ScrollbarStyles {
    track: ScrollbarTrackStyle,
    track_hover: ScrollbarTrackStyle,
    track_active: ScrollbarTrackStyle,
    thumb: ScrollbarThumbStyle,
    thumb_hover: ScrollbarThumbStyle,
    thumb_active: ScrollbarThumbStyle,
}

impl ScrollbarStyles {
    pub fn track(mut self, build: impl FnOnce(ScrollbarTrackStyle) -> ScrollbarTrackStyle) -> Self {
        self.track = build(self.track);
        self
    }

    pub fn track_hover(
        mut self,
        build: impl FnOnce(ScrollbarTrackStyle) -> ScrollbarTrackStyle,
    ) -> Self {
        self.track_hover = build(self.track_hover);
        self
    }

    pub fn track_active(
        mut self,
        build: impl FnOnce(ScrollbarTrackStyle) -> ScrollbarTrackStyle,
    ) -> Self {
        self.track_active = build(self.track_active);
        self
    }

    pub fn thumb(mut self, build: impl FnOnce(ScrollbarThumbStyle) -> ScrollbarThumbStyle) -> Self {
        self.thumb = build(self.thumb);
        self
    }

    pub fn thumb_hover(
        mut self,
        build: impl FnOnce(ScrollbarThumbStyle) -> ScrollbarThumbStyle,
    ) -> Self {
        self.thumb_hover = build(self.thumb_hover);
        self
    }

    pub fn thumb_active(
        mut self,
        build: impl FnOnce(ScrollbarThumbStyle) -> ScrollbarThumbStyle,
    ) -> Self {
        self.thumb_active = build(self.thumb_active);
        self
    }
}

impl FluentBuilder for ScrollbarStyles {}

impl From<Axis> for ScrollbarAxis {
    fn from(axis: Axis) -> Self {
        match axis {
            Axis::Vertical => Self::Vertical,
            Axis::Horizontal => Self::Horizontal,
        }
    }
}

impl ScrollbarAxis {
    /// Return true if the scrollbar axis is vertical.
    #[inline]
    pub fn is_vertical(&self) -> bool {
        matches!(self, Self::Vertical)
    }

    /// Return true if the scrollbar axis is horizontal.
    #[inline]
    pub fn is_horizontal(&self) -> bool {
        matches!(self, Self::Horizontal)
    }

    /// Return true if the scrollbar axis is both vertical and horizontal.
    #[inline]
    pub fn is_both(&self) -> bool {
        matches!(self, Self::Both)
    }

    /// Return true if the scrollbar has vertical axis.
    #[inline]
    pub fn has_vertical(&self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    /// Return true if the scrollbar has horizontal axis.
    #[inline]
    pub fn has_horizontal(&self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    #[inline]
    fn all(&self) -> Vec<Axis> {
        match self {
            Self::Vertical => vec![Axis::Vertical],
            Self::Horizontal => vec![Axis::Horizontal],
            // This should keep Horizontal first, Vertical is the primary axis
            // if Vertical not need display, then Horizontal will not keep right margin.
            Self::Both => vec![Axis::Horizontal, Axis::Vertical],
        }
    }
}

/// Scrollbar control for scroll-area or a uniform-list.
pub struct Scrollbar {
    pub(crate) id: ElementId,
    axis: ScrollbarAxis,
    mode: Option<ScrollbarMode>,
    scroll_handle: Rc<dyn ScrollbarHandle>,
    scroll_size: Option<Size<Pixels>>,
    viewport_bounds: Option<Bounds<Pixels>>,
    // Scroll range may use a smaller viewport than the padded track bounds.
    scroll_viewport_size: Option<Size<Pixels>>,
    use_layout_bounds: bool,
    /// Maximum frames per second for scrolling by drag. Default is 120 FPS.
    ///
    /// This is used to limit the update rate of the scrollbar when it is
    /// being dragged for some complex interactions for reducing CPU usage.
    max_fps: usize,
    styles: ScrollbarStyles,
}

impl Scrollbar {
    /// Create a new scrollbar.
    ///
    /// This will have both vertical and horizontal scrollbars.
    #[track_caller]
    pub fn new<H: ScrollbarHandle + Clone>(scroll_handle: &H) -> Self {
        let caller = Location::caller();
        Self {
            id: ElementId::CodeLocation(*caller),
            axis: ScrollbarAxis::Both,
            mode: None,
            scroll_handle: Rc::new(scroll_handle.clone()),
            max_fps: 120,
            scroll_size: None,
            viewport_bounds: None,
            scroll_viewport_size: None,
            use_layout_bounds: false,
            styles: ScrollbarStyles::default(),
        }
    }

    /// Create with horizontal scrollbar.
    #[track_caller]
    pub fn horizontal<H: ScrollbarHandle + Clone>(scroll_handle: &H) -> Self {
        Self::new(scroll_handle).axis(ScrollbarAxis::Horizontal)
    }

    /// Create with vertical scrollbar.
    #[track_caller]
    pub fn vertical<H: ScrollbarHandle + Clone>(scroll_handle: &H) -> Self {
        Self::new(scroll_handle).axis(ScrollbarAxis::Vertical)
    }

    /// Set a specific element id, default is the [`Location::caller`].
    ///
    /// NOTE: In most cases, you don't need to set a specific id for scrollbar.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    /// Set the scrollbar show mode [`ScrollbarMode`].
    ///
    /// If unset, the current application theme projection is used.
    pub fn mode(mut self, mode: ScrollbarMode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Set a special scroll size of the content area, default is None.
    ///
    /// Default will sync the `content_size` from `scroll_handle`.
    pub fn scroll_size(mut self, scroll_size: Size<Pixels>) -> Self {
        self.scroll_size = Some(scroll_size);
        self
    }

    /// Override the viewport bounds that this scrollbar overlays.
    ///
    /// Most scroll containers should rely on the bounds reported by their
    /// scroll handle. Custom-painted viewports, such as the text editor, can
    /// use this when their visible bounds differ from the handle's layout
    /// bounds.
    pub fn viewport_bounds(mut self, bounds: Bounds<Pixels>) -> Self {
        self.viewport_bounds = Some(bounds);
        self
    }

    pub(crate) fn scroll_viewport_size(mut self, size: Size<Pixels>) -> Self {
        self.scroll_viewport_size = Some(size);
        self
    }

    /// Use the scrollbar element's layout bounds as its viewport.
    ///
    /// This is useful for composite widgets whose scrollbar viewport excludes
    /// fixed headers or columns and is therefore defined by a positioned
    /// overlay container rather than by the underlying scroll handle.
    pub fn viewport_from_layout(mut self) -> Self {
        self.use_layout_bounds = true;
        self
    }

    fn resolved_viewport_bounds(&self, layout_bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        self.viewport_bounds.unwrap_or_else(|| {
            if self.use_layout_bounds {
                layout_bounds
            } else {
                self.scroll_handle.viewport_bounds()
            }
        })
    }

    /// Set scrollbar axis.
    pub fn axis(mut self, axis: impl Into<ScrollbarAxis>) -> Self {
        self.axis = axis.into();
        self
    }

    pub fn styles(mut self, build: impl FnOnce(ScrollbarStyles) -> ScrollbarStyles) -> Self {
        self.styles = build(self.styles);
        self
    }

    /// Set maximum frames per second for scrolling by drag. Default is 120 FPS.
    ///
    /// If you have very high CPU usage, consider reducing this value to improve performance.
    ///
    /// Available values: 30..120
    #[doc(hidden)]
    pub fn max_fps(mut self, max_fps: usize) -> Self {
        self.max_fps = max_fps.clamp(30, 120);
        self
    }

    // Get the width of the scrollbar.
    #[doc(hidden)]
    pub const fn width() -> Pixels {
        WIDTH
    }

    /// The width of a track this theme gives scrollbars without styles of their own.
    pub(crate) fn themed_track_width(cx: &App) -> Pixels {
        cx.theme().scrollbar.styles().track.width.unwrap_or(WIDTH)
    }

    fn resolve_track(
        &self,
        cx: &App,
        state: &ScrollbarTrackStyle,
        global_state: &ScrollbarTrackStyle,
        default_border: Hsla,
    ) -> (Hsla, Hsla) {
        let theme = cx.theme();
        let global = theme.scrollbar.styles();
        (
            state
                .background
                .or(self.styles.track.background)
                .or(global_state.background)
                .or(global.track.background)
                .unwrap_or_else(gpui::transparent_black),
            state
                .border
                .or(self.styles.track.border)
                .or(global_state.border)
                .or(global.track.border)
                .unwrap_or(default_border),
        )
    }

    fn resolve_thumb(
        &self,
        cx: &App,
        state: &ScrollbarThumbStyle,
        global_state: &ScrollbarThumbStyle,
        defaults: ScrollbarThumbStyle,
    ) -> (Background, Pixels, Pixels, Pixels, Pixels) {
        let theme = cx.theme();
        let global = theme.scrollbar.styles();
        (
            state
                .background
                .or(self.styles.thumb.background)
                .or(global_state.background)
                .or(global.thumb.background)
                .unwrap_or_else(|| defaults.background.unwrap()),
            state
                .width
                .or(self.styles.thumb.width)
                .or(global_state.width)
                .or(global.thumb.width)
                .unwrap_or_else(|| defaults.width.unwrap()),
            state
                .inset
                .or(self.styles.thumb.inset)
                .or(global_state.inset)
                .or(global.thumb.inset)
                .unwrap_or_else(|| defaults.inset.unwrap()),
            state
                .radius
                .or(self.styles.thumb.radius)
                .or(global_state.radius)
                .or(global.thumb.radius)
                .unwrap_or_else(|| defaults.radius.unwrap()),
            state
                .min_length
                .or(self.styles.thumb.min_length)
                .or(global_state.min_length)
                .or(global.thumb.min_length)
                .or(defaults.min_length)
                .unwrap_or(MIN_THUMB_SIZE),
        )
    }

    /// The thumb colour a scrollbar falls back to when nothing has overridden
    /// it, taken from the active theme rather than fixed.
    ///
    /// It used to be literal black at these alphas. That reads as an ordinary
    /// grey thumb on a light surface and as very nearly nothing at all on a
    /// dark one, and no palette an application installed could change it:
    /// `Theme` carries `scrollbar` beside `tokens` rather than derived from
    /// them, so a theme swap moved every token except the ones the scrollbar
    /// actually paints with.
    ///
    /// `foreground` is the token that already means "ink on this surface" and
    /// already flips with the appearance, so on a light theme this stays within
    /// a hair of the old constant and on a dark one it becomes visible. An
    /// explicit `ScrollbarStyles` still wins: this is the bottom of the
    /// cascade, not a new top of it.
    fn thumb_default_background(cx: &App, alpha: f32) -> Background {
        cx.theme().tokens.colors.foreground.alpha(alpha).into()
    }

    fn thumb_defaults(
        background: Background,
        width: Pixels,
        inset: Pixels,
        radius: Pixels,
    ) -> ScrollbarThumbStyle {
        ScrollbarThumbStyle {
            background: Some(background),
            width: Some(width),
            inset: Some(inset),
            radius: Some(radius),
            min_length: Some(MIN_THUMB_SIZE),
        }
    }

    fn style_for_active(
        &self,
        cx: &App,
    ) -> (Background, Hsla, Hsla, Pixels, Pixels, Pixels, Pixels) {
        let theme = cx.theme();
        let global = theme.scrollbar.styles();
        let (track, border) = self.resolve_track(
            cx,
            &self.styles.track_active,
            &global.track_active,
            gpui::transparent_black(),
        );
        let (thumb, width, inset, radius, min_length) = self.resolve_thumb(
            cx,
            &self.styles.thumb_active,
            &global.thumb_active,
            Self::thumb_defaults(
                Self::thumb_default_background(cx, 0.55),
                THUMB_ACTIVE_WIDTH,
                THUMB_ACTIVE_INSET,
                THUMB_ACTIVE_RADIUS,
            ),
        );
        (thumb, track, border, width, inset, radius, min_length)
    }

    fn style_for_hovered_thumb(
        &self,
        cx: &App,
    ) -> (Background, Hsla, Hsla, Pixels, Pixels, Pixels, Pixels) {
        let theme = cx.theme();
        let global = theme.scrollbar.styles();
        let (track, border) = self.resolve_track(
            cx,
            &self.styles.track_active,
            &global.track_active,
            gpui::transparent_black(),
        );
        let (thumb, width, inset, radius, min_length) = self.resolve_thumb(
            cx,
            &self.styles.thumb_hover,
            &global.thumb_hover,
            Self::thumb_defaults(
                Self::thumb_default_background(cx, 0.55),
                THUMB_ACTIVE_WIDTH,
                THUMB_ACTIVE_INSET,
                THUMB_ACTIVE_RADIUS,
            ),
        );
        (thumb, track, border, width, inset, radius, min_length)
    }

    fn style_for_hovered_bar(
        &self,
        cx: &App,
    ) -> (Background, Hsla, Hsla, Pixels, Pixels, Pixels, Pixels) {
        let theme = cx.theme();
        let global = theme.scrollbar.styles();
        let (track, border) = self.resolve_track(
            cx,
            &self.styles.track_hover,
            &global.track_hover,
            gpui::transparent_black(),
        );
        let (thumb, width, inset, radius, min_length) = self.resolve_thumb(
            cx,
            &self.styles.thumb,
            &global.thumb,
            Self::thumb_defaults(
                Self::thumb_default_background(cx, 0.35),
                THUMB_WIDTH,
                THUMB_INSET,
                THUMB_RADIUS,
            ),
        );
        (thumb, track, border, width, inset, radius, min_length)
    }

    fn style_for_normal(
        &self,
        cx: &App,
    ) -> (Background, Hsla, Hsla, Pixels, Pixels, Pixels, Pixels) {
        let theme = cx.theme();
        let global = theme.scrollbar.styles();

        let (track, border) = self.resolve_track(
            cx,
            &self.styles.track,
            &global.track,
            gpui::transparent_black(),
        );
        let (thumb, width, inset, radius, min_length) = self.resolve_thumb(
            cx,
            &self.styles.thumb,
            &global.thumb,
            Self::thumb_defaults(
                Self::thumb_default_background(cx, 0.35),
                THUMB_WIDTH,
                THUMB_INSET,
                THUMB_RADIUS,
            ),
        );
        (thumb, track, border, width, inset, radius, min_length)
    }
}

#[cfg(not(feature = "gpui-fast"))]
impl gpui::IntoElement for Scrollbar {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[doc(hidden)]
pub struct PrepaintState {
    hitbox: Hitbox,
    scrollbar_state: ScrollbarState,
    states: Vec<AxisPrepaintState>,
}

#[doc(hidden)]
pub struct AxisPrepaintState {
    axis: Axis,
    bar_hitbox: Hitbox,
    bounds: Bounds<Pixels>,
    radius: Pixels,
    bg: Hsla,
    border: Hsla,
    thumb_bounds: Bounds<Pixels>,
    // Bounds of thumb to be rendered.
    thumb_fill_bounds: Bounds<Pixels>,
    thumb_bg: Background,
    geometry: ThumbGeometry,
    active_geometry: ThumbGeometry,
    track_width: Pixels,
    visibility_opacity: f32,
    visibility_position: f32,
    visibility_requested: bool,
}

/// The same longitudinal geometry drives painting, track clicks and dragging.
/// `length` excludes the inset, while `travel` excludes the full logical thumb.
#[derive(Clone, Copy)]
struct ThumbGeometry {
    origin: Pixels,
    inset: Pixels,
    length: Pixels,
    travel: Pixels,
    extent: Pixels,
}

impl ThumbGeometry {
    fn new(
        origin: Pixels,
        track_size: Pixels,
        viewport_size: Pixels,
        content: Pixels,
        margin_end: Pixels,
        inset: Pixels,
        min_length: Pixels,
    ) -> Self {
        let track = (track_size - margin_end).max(px(0.));
        let logical_length = (viewport_size / content * track).max(min_length).min(track);
        let inset = inset.clamp(px(0.), logical_length / 2.);
        Self {
            origin,
            inset,
            length: logical_length - inset * 2.,
            travel: track - logical_length,
            extent: content - viewport_size,
        }
    }

    fn start(self, offset: Pixels) -> Pixels {
        self.origin + self.inset + (-offset / self.extent).clamp(0., 1.) * self.travel
    }

    fn offset(self, position: Pixels, grab: Pixels) -> Pixels {
        if self.travel <= px(0.) {
            return px(0.);
        }
        -self.extent * ((position - self.origin - self.inset - grab) / self.travel).clamp(0., 1.)
    }

    fn drag_offset(
        self,
        axis: Axis,
        position: Point<Pixels>,
        grab: Point<Pixels>,
        mut offset: Point<Pixels>,
    ) -> Point<Pixels> {
        if self.travel <= px(0.) {
            return offset;
        }
        if axis.is_vertical() {
            offset.y = self.offset(position.y, grab.y);
        } else {
            offset.x = self.offset(position.x, grab.x);
        }
        offset
    }
}

impl Element for Scrollbar {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<gpui::ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.flex_grow = 1.0;
        style.flex_shrink = 1.0;
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();

        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let bounds = self.resolved_viewport_bounds(bounds);
        let hitbox = window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.insert_hitbox(bounds, HitboxBehavior::Normal)
        });

        let state = window
            .use_state(cx, |_, _| ScrollbarState::default())
            .read(cx)
            .clone();

        let now = Instant::now();
        let base_theme = cx.theme();
        let mode = self.mode.unwrap_or(base_theme.scrollbar.mode());
        let motion = base_theme.scrollbar.motion();
        // Always-visible scrollbars skip visibility motion but still animate
        // their activity width. Reduced motion snaps every channel.
        let reduce_motion = cx.reduce_motion();
        let (enter, exit) = if !mode.is_always() && !reduce_motion {
            (motion.enter(), motion.exit())
        } else {
            (Duration::ZERO, Duration::ZERO)
        };
        let expand = if reduce_motion {
            Duration::ZERO
        } else {
            motion.expand()
        };

        let mut inner = state.get();
        let current_offset = self.scroll_handle.offset();
        if current_offset != inner.last_scroll_offset {
            inner = inner.with_last_scroll(current_offset, Some(now));
        }

        let is_hovered = inner.hovered_axis.is_some() || inner.hovered_on_thumb.is_some();
        let is_dragging = inner.dragged_axis.is_some();
        let is_currently_visible = inner.visibility.sample(now).opacity > 0.0;
        let visible = hover_keeps_visible(mode, is_hovered, is_currently_visible)
            || wants_visible(
                mode,
                is_hovered,
                is_dragging,
                inner.last_scroll_time,
                motion.idle(),
                now,
            );
        inner.visibility.set_visible(
            visible,
            motion.entrance_for(mode, inner.hovered_on_thumb.is_some()),
            enter,
            exit,
            now,
        );
        let visibility = inner.visibility.sample(now);
        if visibility.running {
            window.request_animation_frame();
        }

        if !is_hovered && !is_dragging {
            if let Some(last_time) = inner.last_scroll_time {
                let elapsed = now.saturating_duration_since(last_time);
                if elapsed < motion.idle() && !inner.idle_timer_scheduled {
                    inner.idle_timer_scheduled = true;
                    let state = state.clone();
                    let current_view = window.current_view();
                    let next_delay = motion.idle() - elapsed;
                    window
                        .spawn(cx, async move |cx| {
                            cx.background_executor().timer(next_delay).await;
                            state.set(state.get().with_idle_timer_scheduled(false));
                            cx.update(|_, cx| cx.notify(current_view)).ok();
                        })
                        .detach();
                }
            }
        }
        state.set(inner);

        let mut states = vec![];
        let mut has_both = self.axis.is_both();
        let scroll_size = self
            .scroll_size
            .unwrap_or(self.scroll_handle.content_size());
        let scroll_viewport_size = self.scroll_viewport_size.unwrap_or(hitbox.size);

        for axis in self.axis.all().into_iter() {
            let is_vertical = axis.is_vertical();
            let track_width = self
                .styles
                .track
                .width
                .or(cx.theme().scrollbar.styles().track.width)
                .unwrap_or(WIDTH);
            let (scroll_area_size, container_size, scroll_viewport_size, scroll_position) =
                if is_vertical {
                    (
                        scroll_size.height,
                        hitbox.size.height,
                        scroll_viewport_size.height,
                        self.scroll_handle.offset().y,
                    )
                } else {
                    (
                        scroll_size.width,
                        hitbox.size.width,
                        scroll_viewport_size.width,
                        self.scroll_handle.offset().x,
                    )
                };

            // The horizontal scrollbar is set avoid overlapping with the vertical scrollbar, if the vertical scrollbar is visible.
            let margin_end = if has_both && !is_vertical {
                track_width
            } else {
                px(0.)
            };

            // Hide scrollbar, if the scroll area is smaller than the container.
            if scroll_area_size <= scroll_viewport_size {
                has_both = false;
                continue;
            }

            let bounds = Bounds {
                origin: if is_vertical {
                    point(
                        hitbox.origin.x + hitbox.size.width - track_width,
                        hitbox.origin.y,
                    )
                } else {
                    point(
                        hitbox.origin.x,
                        hitbox.origin.y + hitbox.size.height - track_width,
                    )
                },
                size: gpui::Size {
                    width: if is_vertical {
                        track_width
                    } else {
                        hitbox.size.width
                    },
                    height: if is_vertical {
                        hitbox.size.height
                    } else {
                        track_width
                    },
                },
            };

            let is_always_to_show = mode.is_always();
            let is_hover_to_show = mode.is_hover();
            let is_hovered_on_bar = state.get().hovered_axis == Some(axis);
            let is_hovered_on_thumb = state.get().hovered_on_thumb == Some(axis);

            let (thumb_bg, bar_bg, bar_border, mut thumb_width, inset, radius, min_length) =
                if state.get().dragged_axis == Some(axis) {
                    self.style_for_active(cx)
                } else if (is_hover_to_show || mode == ScrollbarMode::Scrolling)
                    && (is_hovered_on_bar || is_hovered_on_thumb)
                {
                    if is_hovered_on_thumb {
                        self.style_for_hovered_thumb(cx)
                    } else {
                        self.style_for_hovered_bar(cx)
                    }
                } else if is_always_to_show && (is_hovered_on_bar || is_hovered_on_thumb) {
                    if is_hovered_on_thumb {
                        self.style_for_hovered_thumb(cx)
                    } else {
                        self.style_for_hovered_bar(cx)
                    }
                } else {
                    self.style_for_normal(cx)
                };

            let mut width_animation = if is_vertical {
                state.get().vertical_width
            } else {
                state.get().horizontal_width
            };
            let (animated_width, running) = width_animation.set_target(thumb_width, expand, now);
            let mut updated = state.get();
            if is_vertical {
                updated.vertical_width = width_animation;
            } else {
                updated.horizontal_width = width_animation;
            }
            state.set(updated);
            thumb_width = animated_width;
            if running {
                window.request_animation_frame();
            }

            let origin = if is_vertical {
                bounds.origin.y
            } else {
                bounds.origin.x
            };
            let geometry = ThumbGeometry::new(
                origin,
                container_size,
                scroll_viewport_size,
                scroll_area_size,
                margin_end,
                inset,
                min_length,
            );
            let (_, _, _, _, active_inset, _, active_min_length) = self.style_for_active(cx);
            let active_geometry = ThumbGeometry::new(
                origin,
                container_size,
                scroll_viewport_size,
                scroll_area_size,
                margin_end,
                active_inset,
                active_min_length,
            );
            let thumb_start = geometry.start(scroll_position) - origin;

            // The clickable area of the thumb
            let thumb_length = geometry.length;
            let thumb_bounds = if is_vertical {
                Bounds::from_anchor_and_size(
                    Anchor::TopRight,
                    bounds.top_right() + point(-inset, thumb_start),
                    size(track_width, thumb_length),
                )
            } else {
                Bounds::from_anchor_and_size(
                    Anchor::BottomLeft,
                    bounds.bottom_left() + point(thumb_start, -inset),
                    size(thumb_length, track_width),
                )
            };

            // The actual render area of the thumb
            let thumb_fill_bounds = if is_vertical {
                Bounds::from_anchor_and_size(
                    Anchor::TopRight,
                    bounds.top_right() + point(-inset, thumb_start),
                    size(thumb_width, thumb_length),
                )
            } else {
                Bounds::from_anchor_and_size(
                    Anchor::BottomLeft,
                    bounds.bottom_left() + point(thumb_start, -inset),
                    size(thumb_length, thumb_width),
                )
            };

            let bar_hitbox = window.with_content_mask(Some(ContentMask { bounds }), |window| {
                window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)
            });

            states.push(AxisPrepaintState {
                axis,
                bar_hitbox,
                bounds,
                radius,
                bg: bar_bg,
                border: bar_border,
                thumb_bounds,
                thumb_fill_bounds,
                thumb_bg,
                geometry,
                active_geometry,
                track_width,
                visibility_opacity: visibility.opacity,
                visibility_position: visibility.position,
                visibility_requested: visible,
            })
        }

        PrepaintState {
            hitbox,
            states,
            scrollbar_state: state,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let scrollbar_state = &prepaint.scrollbar_state;
        let theme = cx.theme();
        let mode = self.mode.unwrap_or(theme.scrollbar.mode());
        let view_id = window.current_view();
        let hitbox_bounds = prepaint.hitbox.bounds;
        let is_hover_to_show = mode.is_hover();

        window.with_content_mask(
            Some(ContentMask {
                bounds: hitbox_bounds,
            }),
            |window| {
                for state in prepaint.states.iter() {
                    let axis = state.axis;
                    let mut radius = state.radius;
                    if theme.tokens.radius.md.is_zero() {
                        radius = px(0.);
                    }
                    radius = clamp_thumb_radius(radius, state.thumb_fill_bounds);
                    let thumb_bounds = state.thumb_bounds;
                    let geometry = state.geometry;
                    let active_geometry = state.active_geometry;
                    let is_vertical = axis.is_vertical();
                    let visibility_opacity = state.visibility_opacity;
                    let is_visible = state.visibility_requested || visibility_opacity > 0.0;
                    let translation =
                        visibility_translation(axis, state.track_width, state.visibility_position);
                    let painted_bounds = state.bounds + translation;
                    let painted_thumb_bounds = state.thumb_fill_bounds + translation;
                    let painted_track_bg = state.bg.opacity(visibility_opacity);
                    let painted_border = state.border.opacity(visibility_opacity);
                    let painted_thumb_bg = state.thumb_bg.clone().opacity(visibility_opacity);

                    window.set_cursor_style(CursorStyle::default(), &state.bar_hitbox);

                    window.paint_layer(hitbox_bounds, |cx| {
                        cx.paint_quad(fill(painted_bounds, painted_track_bg));

                        cx.paint_quad(PaintQuad {
                            bounds: painted_bounds,
                            corner_radii: (0.).into(),
                            background: gpui::transparent_black().into(),
                            border_widths: if is_vertical {
                                Edges {
                                    top: px(0.),
                                    right: px(0.),
                                    bottom: px(0.),
                                    left: px(0.),
                                }
                            } else {
                                Edges {
                                    top: px(0.),
                                    right: px(0.),
                                    bottom: px(0.),
                                    left: px(0.),
                                }
                            },
                            border_color: painted_border,
                            border_style: BorderStyle::default(),
                        });

                        cx.paint_quad(
                            fill(painted_thumb_bounds, painted_thumb_bg).corner_radii(radius),
                        );
                    });

                    window.on_mouse_event({
                        let state = scrollbar_state.clone();
                        let scroll_handle = self.scroll_handle.clone();

                        move |event: &ScrollEvent, phase, _, cx| {
                            if phase.bubble() && hitbox_bounds.contains(&event.position) {
                                if scroll_handle.offset() != state.get().last_scroll_offset {
                                    state.set(state.get().with_last_scroll(
                                        scroll_handle.offset(),
                                        Some(Instant::now()),
                                    ));
                                    cx.notify(view_id);
                                }
                            }
                        }
                    });

                    // A thumb follows the finger, unlike content panning. Claim
                    // this touch before the window turns it into wheel deltas.
                    window.on_mouse_event({
                        let state = scrollbar_state.clone();
                        let scroll_handle = self.scroll_handle.clone();
                        let max_fps_duration = Duration::from_secs_f64(1. / self.max_fps as f64);
                        move |event: &TouchDragEvent, phase, window, cx| {
                            if !phase.bubble() {
                                return;
                            }
                            if event.phase == TouchPhase::Started {
                                if !is_visible
                                    || window.default_prevented()
                                    || !thumb_bounds.contains(&event.start_position)
                                {
                                    return;
                                }
                                scroll_handle.start_drag();
                                state.set(state.get().with_drag_pos(
                                    axis,
                                    event.start_position - thumb_bounds.origin,
                                ));
                            } else if state.get().dragged_axis != Some(axis) {
                                return;
                            }
                            if event.phase != TouchPhase::Cancelled {
                                scroll_handle.set_offset(active_geometry.drag_offset(
                                    axis,
                                    event.position,
                                    state.get().drag_pos,
                                    scroll_handle.offset(),
                                ));
                            }
                            if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                                scroll_handle.end_drag();
                                state.set(state.get().with_unset_drag_pos(Instant::now()));
                            }
                            window.prevent_default();
                            cx.stop_propagation();
                            if event.phase == TouchPhase::Moved {
                                state.notify_drag(
                                    Instant::now(),
                                    max_fps_duration,
                                    view_id,
                                    window,
                                    cx,
                                );
                            } else {
                                cx.notify(view_id);
                            }
                        }
                    });

                    if is_visible {
                        window.on_mouse_event({
                            // The hitbox rather than the bounds, so that an
                            // element above the bar (a menu, a dialog) keeps
                            // the press.
                            let bar_hitbox = state.bar_hitbox.clone();
                            let state = scrollbar_state.clone();
                            let scroll_handle = self.scroll_handle.clone();

                            move |event: &MouseDownEvent, phase, window, cx| {
                                if phase.bubble() && bar_hitbox.is_hovered(window) {
                                    cx.stop_propagation();

                                    if thumb_bounds.contains(&event.position) {
                                        // click on the thumb bar, set the drag position
                                        let pos = event.position - thumb_bounds.origin;

                                        scroll_handle.start_drag();
                                        state.set(state.get().with_drag_pos(axis, pos));
                                        // Active styling can change the inset or minimum length.
                                        // Keep the original grab point under the pointer on that frame.
                                        scroll_handle.set_offset(active_geometry.drag_offset(
                                            axis,
                                            event.position,
                                            pos,
                                            scroll_handle.offset(),
                                        ));
                                    } else {
                                        // click on the scrollbar, jump to the position
                                        // Set the thumb bar center to the click position
                                        let center =
                                            point(geometry.length / 2., geometry.length / 2.);
                                        scroll_handle.set_offset(geometry.drag_offset(
                                            axis,
                                            event.position,
                                            center,
                                            scroll_handle.offset(),
                                        ));
                                    }

                                    cx.notify(view_id);
                                }
                            }
                        });
                    }

                    window.on_mouse_event({
                        let bar_hitbox = state.bar_hitbox.clone();
                        let scroll_handle = self.scroll_handle.clone();
                        let state = scrollbar_state.clone();
                        let max_fps_duration = Duration::from_secs_f64(1. / self.max_fps as f64);

                        move |event: &MouseMoveEvent, _, window, cx| {
                            let mut notify = false;
                            // When is hover to show mode or it was visible,
                            // we need to update the hovered state and increase the last_scroll_time.
                            let need_hover_to_update = is_hover_to_show || is_visible;
                            // Update hovered state for scrollbar
                            let bar_hovered = bar_hitbox.is_hovered(window);
                            if bar_hovered && need_hover_to_update {
                                let hover_changed = state.get().hovered_axis != Some(axis);
                                state.set(state.get().with_hovered(Some(axis), Instant::now()));
                                notify |= hover_changed;
                            } else if state.get().hovered_axis == Some(axis) {
                                state.set(state.get().with_hovered(None, Instant::now()));
                                notify = true;
                            }

                            // Update hovered state for scrollbar thumb
                            if tracks_thumb_hover(mode, is_visible)
                                && bar_hovered
                                && thumb_bounds.contains(&event.position)
                            {
                                if state.get().hovered_on_thumb != Some(axis) {
                                    state.set(state.get().with_hovered_on_thumb(Some(axis)));
                                    notify = true;
                                }
                            } else {
                                if state.get().hovered_on_thumb == Some(axis) {
                                    state.set(state.get().with_hovered_on_thumb(None));
                                    notify = true;
                                }
                            }

                            // Move thumb position on dragging
                            if state.get().dragged_axis == Some(axis) && event.dragging() {
                                // Stop the event propagation to avoid selecting text or other side effects.
                                cx.stop_propagation();

                                let offset = active_geometry.drag_offset(
                                    axis,
                                    event.position,
                                    state.get().drag_pos,
                                    scroll_handle.offset(),
                                );
                                if scroll_handle.offset() != offset {
                                    // Coalesce rendering, never discard the newest pointer position.
                                    scroll_handle.set_offset(offset);
                                    notify = true;
                                }
                                if notify {
                                    state.notify_drag(
                                        Instant::now(),
                                        max_fps_duration,
                                        view_id,
                                        window,
                                        cx,
                                    );
                                }
                                return;
                            }

                            if notify {
                                cx.notify(view_id);
                            }
                        }
                    });

                    window.on_mouse_event({
                        let state = scrollbar_state.clone();
                        let scroll_handle = self.scroll_handle.clone();

                        move |event: &MouseUpEvent, phase, _, cx| {
                            if phase.bubble() && state.get().dragged_axis == Some(axis) {
                                scroll_handle.set_offset(active_geometry.drag_offset(
                                    axis,
                                    event.position,
                                    state.get().drag_pos,
                                    scroll_handle.offset(),
                                ));
                                scroll_handle.end_drag();
                                state.set(state.get().with_unset_drag_pos(Instant::now()));
                                cx.notify(view_id);
                            }
                        }
                    });
                }
            },
        );
    }
}
