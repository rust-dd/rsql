use crate::TestSupportExt as _;
use std::ops::Range;

use gpui::{
    AccessibleAction, Along, AnyElement, App, AppContext as _, Axis, Bounds, Context, Div,
    DragMoveEvent, Empty, Entity, EntityId, EventEmitter, FocusHandle, HitboxBehavior,
    InteractiveElement, IntoElement, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent,
    Orientation, ParentElement, Pixels, Point, Render, RenderOnce, Role, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, TouchDragEvent, TouchPhase, Window,
    accesskit::ActionData, div, prelude::FluentBuilder as _, px,
};

use crate::{
    LayoutDirection, a11y_disabled::A11yDisabledExt as _, element_ext::ElementExt,
    geometry::AxisExt,
};

/// Events emitted by the [`SliderState`].
pub enum SliderEvent {
    /// Emitted continuously while the slider value is being changed by the user.
    Change(SliderValue),
    /// Emitted once when the user releases the slider after a drag or click, and after
    /// each step from the keyboard or an assistive technology.
    Release(SliderValue),
}

/// The value of the slider, can be a single value or a range of values.
///
/// - Can from a f32 value, which will be treated as a single value.
/// - Or from a (f32, f32) tuple, which will be treated as a range of values.
///
/// The default value is `SliderValue::Single(0.0)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderValue {
    Single(f32),
    Range(f32, f32),
}

impl std::fmt::Display for SliderValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SliderValue::Single(value) => write!(f, "{}", value),
            SliderValue::Range(start, end) => write!(f, "{}..{}", start, end),
        }
    }
}

impl From<f32> for SliderValue {
    fn from(value: f32) -> Self {
        SliderValue::Single(value)
    }
}

impl From<(f32, f32)> for SliderValue {
    fn from(value: (f32, f32)) -> Self {
        SliderValue::Range(value.0, value.1)
    }
}

impl From<Range<f32>> for SliderValue {
    fn from(value: Range<f32>) -> Self {
        SliderValue::Range(value.start, value.end)
    }
}

impl Default for SliderValue {
    fn default() -> Self {
        SliderValue::Single(0.)
    }
}

impl SliderValue {
    /// Clamp the value to the given range.
    pub fn clamp(self, min: f32, max: f32) -> Self {
        match self {
            SliderValue::Single(value) => SliderValue::Single(value.clamp(min, max)),
            SliderValue::Range(start, end) => {
                SliderValue::Range(start.clamp(min, max), end.clamp(min, max))
            }
        }
    }

    /// Check if the value is a single value.
    #[inline]
    pub fn is_single(&self) -> bool {
        matches!(self, SliderValue::Single(_))
    }

    /// Check if the value is a range of values.
    #[inline]
    pub fn is_range(&self) -> bool {
        matches!(self, SliderValue::Range(_, _))
    }

    /// Get the start value.
    pub fn start(&self) -> f32 {
        match self {
            SliderValue::Single(value) => *value,
            SliderValue::Range(start, _) => *start,
        }
    }

    /// Get the end value.
    pub fn end(&self) -> f32 {
        match self {
            SliderValue::Single(value) => *value,
            SliderValue::Range(_, end) => *end,
        }
    }

    fn set_start(&mut self, value: f32) {
        if let SliderValue::Range(_, end) = self {
            *self = SliderValue::Range(value.min(*end), *end);
        } else {
            *self = SliderValue::Single(value);
        }
    }

    fn set_end(&mut self, value: f32) {
        if let SliderValue::Range(start, _) = self {
            *self = SliderValue::Range(*start, value.max(*start));
        } else {
            *self = SliderValue::Single(value);
        }
    }
}

/// The scale mode of the slider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SliderScale {
    /// Linear scale where values change uniformly across the slider range.
    /// This is the default mode.
    #[default]
    Linear,
    /// Logarithmic scale where the distance between values increases exponentially.
    ///
    /// This is useful for parameters that have a large range of values where smaller
    /// changes are more significant at lower values. Common examples include:
    ///
    /// - Volume controls (human hearing perception is logarithmic)
    /// - Frequency controls (musical notes follow a logarithmic scale)
    /// - Zoom levels
    /// - Any parameter where you want finer control at lower values
    ///
    /// # For example
    ///
    /// ```
    /// use gpui_base::slider::{SliderScale, SliderState};
    ///
    /// let slider = SliderState::new()
    ///     .min(1.0)    // Must be > 0 for logarithmic scale
    ///     .max(1000.0)
    ///     .scale(SliderScale::Logarithmic);
    /// ```
    ///
    /// - Moving the slider 1/3 of the way will yield ~10
    /// - Moving it 2/3 of the way will yield ~100
    /// - The full range covers 3 orders of magnitude evenly
    Logarithmic,
}

impl SliderScale {
    #[inline]
    pub fn is_linear(&self) -> bool {
        matches!(self, SliderScale::Linear)
    }

    #[inline]
    pub fn is_logarithmic(&self) -> bool {
        matches!(self, SliderScale::Logarithmic)
    }
}

/// State of the [`Slider`].
pub struct SliderState {
    min: f32,
    max: f32,
    step: f32,
    value: SliderValue,
    /// When is single value mode, only `end` is used, the start is always 0.0.
    percentage: Range<f32>,
    /// The bounds of the slider after rendered.
    bounds: Bounds<Pixels>,
    scale: SliderScale,
    /// Tracks whether the user is currently interacting with the slider so we
    /// only emit [`SliderEvent::Release`] after a real press/drag.
    dragging: bool,
    /// The touch drag this slider claimed, while one is live.
    touch_drag: Option<TouchDrag>,
}

/// A touch drag claimed by a slider.
#[derive(Clone, Copy)]
struct TouchDrag {
    /// Identifies the gesture: every event of one drag carries the same start.
    start_position: Point<Pixels>,
    /// Whether it moves the range start thumb.
    is_start: bool,
}

impl SliderState {
    /// Create a new [`SliderState`].
    pub fn new() -> Self {
        Self {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            value: SliderValue::default(),
            percentage: (0.0..0.0),
            bounds: Bounds::default(),
            scale: SliderScale::default(),
            dragging: false,
            touch_drag: None,
        }
    }

    /// Set the minimum value of the slider, default: 0.0
    pub fn min(mut self, min: f32) -> Self {
        if self.scale.is_logarithmic() {
            assert!(
                min > 0.0,
                "`min` must be greater than 0 for SliderScale::Logarithmic"
            );
            assert!(
                min < self.max,
                "`min` must be less than `max` for Logarithmic scale"
            );
        }
        self.min = min;
        self.update_thumb_pos();
        self
    }

    /// Set the maximum value of the slider, default: 100.0
    pub fn max(mut self, max: f32) -> Self {
        if self.scale.is_logarithmic() {
            assert!(
                max > self.min,
                "`max` must be greater than `min` for Logarithmic scale"
            );
        }
        self.max = max;
        self.update_thumb_pos();
        self
    }

    /// Set the step value of the slider, default: 1.0
    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// Set the scale of the slider, default: [`SliderScale::Linear`].
    pub fn scale(mut self, scale: SliderScale) -> Self {
        if scale.is_logarithmic() {
            assert!(
                self.min > 0.0,
                "`min` must be greater than 0 for Logarithmic scale"
            );
            assert!(
                self.max > self.min,
                "`max` must be greater than `min` for Logarithmic scale"
            );
        }
        self.scale = scale;
        self.update_thumb_pos();
        self
    }

    /// Set the default value of the slider, default: 0.0
    pub fn default_value(mut self, value: impl Into<SliderValue>) -> Self {
        self.value = value.into();
        self.update_thumb_pos();
        self
    }

    /// Set the value of the slider.
    pub fn set_value(
        &mut self,
        value: impl Into<SliderValue>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.value = value.into();
        self.update_thumb_pos();
        cx.notify();
    }

    /// Get the value of the slider.
    pub fn value(&self) -> SliderValue {
        self.value
    }

    /// Get the minimum value.
    pub fn min_value(&self) -> f32 {
        self.min
    }

    /// Get the maximum value.
    pub fn max_value(&self) -> f32 {
        self.max
    }

    /// Get the step value.
    pub fn step_value(&self) -> f32 {
        self.step
    }

    /// The amount an assistive technology's Increment and Decrement move a thumb, and the
    /// step reported to it: the step, or a hundredth of the range for a step that is not a
    /// positive finite number (a continuous slider).
    pub fn accessibility_step(&self) -> f32 {
        if is_step(self.step) {
            self.step
        } else {
            (self.max - self.min) / 100.
        }
    }

    /// Converts a value between 0.0 and 1.0 to a value between the minimum and maximum value,
    /// depending on the chosen scale.
    fn percentage_to_value(&self, percentage: f32) -> f32 {
        match self.scale {
            SliderScale::Linear => self.min + (self.max - self.min) * percentage,
            SliderScale::Logarithmic => {
                // when percentage is 0, this simplifies to (max/min)^0 * min = 1 * min = min
                // when percentage is 1, this simplifies to (max/min)^1 * min = (max*min)/min = max
                // we clamp just to make sure we don't have issue with floating point precision
                let base = self.max / self.min;
                (base.powf(percentage) * self.min).clamp(self.min, self.max)
            }
        }
    }

    /// Converts a value between the minimum and maximum value to a value between 0.0 and 1.0,
    /// depending on the chosen scale.
    fn value_to_percentage(&self, value: f32) -> f32 {
        match self.scale {
            SliderScale::Linear => {
                let range = self.max - self.min;
                if range <= 0.0 {
                    0.0
                } else {
                    (value - self.min) / range
                }
            }
            SliderScale::Logarithmic => {
                let base = self.max / self.min;
                (value / self.min).log(base).clamp(0.0, 1.0)
            }
        }
    }

    fn update_thumb_pos(&mut self) {
        match self.value {
            SliderValue::Single(value) => {
                let percentage = self.value_to_percentage(value.min(self.max).max(self.min));
                self.percentage = 0.0..percentage;
            }
            SliderValue::Range(start, end) => {
                // Not `clamp`: while a builder is between its `min` and `max`, `min`
                // can stand above `max`.
                let clamped_start = start.min(self.max).max(self.min);
                let clamped_end = end.min(self.max).max(self.min);
                self.percentage =
                    self.value_to_percentage(clamped_start)..self.value_to_percentage(clamped_end);
            }
        }
    }

    /// Update value by mouse position
    #[doc(hidden)]
    pub fn update_value_by_position(
        &mut self,
        axis: Axis,
        position: Point<Pixels>,
        is_start: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dragging = true;
        let bounds = self.bounds;
        let step = self.step;

        let inner_pos = self.offset_along(axis, position, cx);
        let total_size = bounds.size.along(axis);
        let percentage = inner_pos.clamp(px(0.), total_size) / total_size;

        let percentage = if is_start {
            percentage.clamp(0.0, self.percentage.end)
        } else {
            percentage.clamp(self.percentage.start, 1.0)
        };

        let value = self.percentage_to_value(percentage);
        let value = snap_to_step(value, self.min, self.max, step);

        if is_start {
            self.value.set_start(value);
        } else {
            self.value.set_end(value);
        }
        // The thumb stands at the value, on its step, not at the pointer.
        self.update_thumb_pos();
        cx.emit(SliderEvent::Change(self.value));
        cx.notify();
    }

    /// Moves the end thumb or, with `start`, a range's start thumb by `delta`, as a key and
    /// an assistive technology's Increment and Decrement do, and emits
    /// [`SliderEvent::Change`] then [`SliderEvent::Release`] if it moved. The thumb lands
    /// on the step grid counted from `min` and stops at the other thumb and at the
    /// slider's ends.
    #[doc(hidden)]
    pub fn step_thumb(
        &mut self,
        start: bool,
        delta: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A NaN never equals itself, so it would read as a change below.
        if !steppable(self.value, self.min, self.max) {
            return;
        }
        let value = step_thumb(self.value, start, delta, self.min, self.max, self.step);
        self.commit_thumb_value(value, window, cx);
    }

    fn handle_accessibility_set_value(
        &mut self,
        data: Option<&ActionData>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ActionData::NumericValue(target)) = data else {
            return;
        };
        if !target.is_finite() || !steppable(self.value, self.min, self.max) {
            return;
        }
        let value = thumb_value_at(
            self.value,
            false,
            *target as f32,
            self.min,
            self.max,
            self.step,
        );
        self.commit_thumb_value(value, window, cx);
    }

    fn commit_thumb_value(
        &mut self,
        value: SliderValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if value != self.value {
            self.set_value(value, window, cx);
            cx.emit(SliderEvent::Change(value));
            // A keyboard or accessibility adjustment is a complete gesture.
            cx.emit(SliderEvent::Release(value));
        }
    }

    /// Whether `position` is closer to the range start thumb than to the end thumb.
    fn is_nearer_start(&self, axis: Axis, position: Point<Pixels>, cx: &App) -> bool {
        let size = self.bounds.size.along(axis);
        let along = self.offset_along(axis, position, cx);
        let center =
            ((self.percentage.end - self.percentage.start) / 2. + self.percentage.start) * size;
        along < center
    }

    /// How far `position` is from the minimum end: the start edge of a
    /// horizontal slider (the right in RTL) or the bottom of a vertical one.
    fn offset_along(&self, axis: Axis, position: Point<Pixels>, cx: &App) -> Pixels {
        if axis.is_vertical() {
            self.bounds.bottom() - position.y
        } else if LayoutDirection::global(cx).is_rtl() {
            self.bounds.right() - position.x
        } else {
            position.x - self.bounds.left()
        }
    }

    /// Emit [`SliderEvent::Release`] if the user was actively interacting
    /// with the slider. Called on mouse-up both inside and outside the slider.
    #[doc(hidden)]
    pub fn handle_release(&mut self, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        self.dragging = false;
        cx.emit(SliderEvent::Release(self.value));
    }
}

/// Steps Page Up and Page Down move the thumb.
const PAGE_STEPS: f32 = 10.;

/// How far a key moves the thumb: an arrow `step`, Page Up and Page Down ten of them, Home
/// and End the whole `range`. `None` for any other key and for a key held with a modifier.
fn key_delta(keystroke: &Keystroke, axis: Axis, rtl: bool, step: f32, range: f32) -> Option<f32> {
    if keystroke.modifiers.modified() {
        return None;
    }
    // A horizontal slider grows to the left in a right-to-left layout.
    let toward_end = if axis.is_horizontal() && rtl {
        -step
    } else {
        step
    };
    match keystroke.key.as_str() {
        "right" => Some(toward_end),
        "left" => Some(-toward_end),
        "up" => Some(step),
        "down" => Some(-step),
        "pageup" => Some(step * PAGE_STEPS),
        "pagedown" => Some(-step * PAGE_STEPS),
        "home" => Some(-range),
        "end" => Some(range),
        _ => None,
    }
}

/// Whether `step` divides the range into steps: a positive finite number.
fn is_step(step: f32) -> bool {
    step > 0. && step.is_finite()
}

/// `value` on the nearest step counted from `min`, kept inside the range. A step that is
/// not a positive finite number leaves the value continuous.
fn snap_to_step(value: f32, min: f32, max: f32, step: f32) -> f32 {
    if !is_step(step) {
        return value;
    }
    (min + ((value - min) / step).round() * step)
        .min(max)
        .max(min)
}

/// Whether `value` can be stepped inside `min..=max`: not for a NaN value or bound, a
/// `min` above `max`, or a range whose start is past its end.
fn steppable(value: SliderValue, min: f32, max: f32) -> bool {
    let (start, end) = (value.start(), value.end());
    min <= max && !start.is_nan() && !end.is_nan() && (!value.is_range() || start <= end)
}

/// `value` with its end, or with `start` a range's start, moved by `delta` and snapped to
/// `step` counted from `min`, inside `min..=max` and without passing the other thumb. A
/// value that is not [`steppable`] comes back unchanged.
fn step_thumb(
    value: SliderValue,
    start: bool,
    delta: f32,
    min: f32,
    max: f32,
    step: f32,
) -> SliderValue {
    if !steppable(value, min, max) {
        return value;
    }
    let current = if start && value.is_range() {
        value.start()
    } else {
        value.end()
    };
    thumb_value_at(value, start, current + delta, min, max, step)
}

fn thumb_value_at(
    mut value: SliderValue,
    start: bool,
    target: f32,
    min: f32,
    max: f32,
    step: f32,
) -> SliderValue {
    let next = snap_to_step(target, min, max, step).min(max).max(min);
    if start && value.is_range() {
        value.set_start(next);
    } else {
        value.set_end(next);
    }
    value
}

#[derive(Clone)]
struct DragThumb((EntityId, bool));

impl Render for DragThumb {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
struct DragSlider(EntityId);

impl Render for DragSlider {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// An unstyled slider behavior root.
///
/// Applications provide the track, range, and thumb presentation as children.
#[derive(IntoElement)]
pub struct Slider {
    state: Entity<SliderState>,
    axis: Axis,
    disabled: bool,
    accessibility_label: Option<SharedString>,
    focus: Option<FocusHandle>,
    base: Div,
    children: Vec<AnyElement>,
}

impl Slider {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            axis: Axis::Horizontal,
            disabled: false,
            accessibility_label: None,
            focus: None,
            base: div(),
            children: Vec::new(),
        }
    }

    pub fn horizontal(mut self) -> Self {
        self.axis = Axis::Horizontal;
        self
    }

    pub fn vertical(mut self) -> Self {
        self.axis = Axis::Vertical;
        self
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Sets the label exposed to accessibility clients.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    /// Makes the slider's accessibility node itself focusable with `focus`,
    /// so keyboard focus and assistive technology land on the slider. While
    /// it has focus, the arrow keys move the end thumb a step, Page Up and
    /// Page Down ten steps, and Home and End to the slider's ends.
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
}

impl ParentElement for Slider {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Slider {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Slider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let axis = self.axis;
        let entity_id = self.state.entity_id();
        let state = self.state.read(cx);
        let slider_state = self.state.clone();

        self.base
            .id(("slider", entity_id))
            .test_support()
            .when_some(self.focus, |this, focus| this.track_focus(&focus))
            .role(Role::Slider)
            .when_some(self.accessibility_label, |this, label| {
                this.aria_label(label)
            })
            .aria_numeric_value(state.value().end() as f64)
            .aria_min_numeric_value(state.min_value() as f64)
            .aria_max_numeric_value(state.max_value() as f64)
            .aria_numeric_value_step(state.accessibility_step() as f64)
            .aria_orientation(if axis.is_vertical() {
                Orientation::Vertical
            } else {
                Orientation::Horizontal
            })
            // A disabled slider takes no value, from the pointer or from
            // assistive technology.
            .when(!self.disabled, |this| {
                this.on_a11y_action(AccessibleAction::Increment, {
                    let state = slider_state.clone();
                    move |_, window, cx| {
                        state.update(cx, |state, cx| {
                            state.step_thumb(false, state.accessibility_step(), window, cx);
                        });
                    }
                })
                .on_a11y_action(AccessibleAction::Decrement, {
                    let state = slider_state.clone();
                    move |_, window, cx| {
                        state.update(cx, |state, cx| {
                            state.step_thumb(false, -state.accessibility_step(), window, cx);
                        });
                    }
                })
                .on_a11y_action(AccessibleAction::SetValue, {
                    let state = slider_state.clone();
                    move |data, window, cx| {
                        state.update(cx, |state, cx| {
                            state.handle_accessibility_set_value(data, window, cx);
                        });
                    }
                })
                // Keys reach the slider only while its `track_focus` handle has focus.
                .on_key_down({
                    let state = slider_state.clone();
                    move |event: &KeyDownEvent, window, cx| {
                        let rtl = LayoutDirection::global(cx).is_rtl();
                        state.update(cx, |state, cx| {
                            let step = state.accessibility_step();
                            let range = state.max - state.min;
                            let Some(delta) = key_delta(&event.keystroke, axis, rtl, step, range)
                            else {
                                return;
                            };
                            state.step_thumb(false, delta, window, cx);
                            cx.stop_propagation();
                        });
                    }
                })
                .on_mouse_up(
                    MouseButton::Left,
                    window.listener_for(&self.state, |state, _, _, cx| state.handle_release(cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    window.listener_for(&self.state, |state, _, _, cx| state.handle_release(cx)),
                )
            })
            .children(self.children)
            .a11y_disabled(self.disabled)
    }
}

/// An unstyled track that records the geometry used to map pointer positions.
#[derive(IntoElement)]
pub struct SliderTrack {
    state: Entity<SliderState>,
    axis: Axis,
    disabled: bool,
    base: Div,
    children: Vec<AnyElement>,
}

impl SliderTrack {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            axis: Axis::Horizontal,
            disabled: false,
            base: div(),
            children: Vec::new(),
        }
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl ParentElement for SliderTrack {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for SliderTrack {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for SliderTrack {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for SliderTrack {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let axis = self.axis;
        let entity_id = self.state.entity_id();
        let state = self.state.read(cx);
        let is_range = state.value().is_range();
        // Touch: GPUI routes a finger drag to `TouchDragEvent` (claimed with
        // `prevent_default`) or to scrolling; `on_drag` below is mouse-only. Claim
        // drags that start on the track, as the scrollbar thumb does.
        let touch_layer = (!self.disabled).then(|| {
            let slider_state = self.state.clone();
            gpui::canvas(
                |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |_, hitbox, window, _| {
                    let slider_state = slider_state.clone();
                    window.on_mouse_event(move |event: &TouchDragEvent, phase, window, cx| {
                        if !phase.bubble() {
                            return;
                        }
                        slider_state.update(cx, |state, cx| {
                            if event.phase == TouchPhase::Started {
                                // A drag whose end never arrived (the slider was
                                // disabled or not painted) ends when another begins.
                                if state.touch_drag.take().is_some() {
                                    state.handle_release(cx);
                                }
                                // The hitbox, not the bounds, so a layer covering
                                // the track keeps its own drags.
                                if window.default_prevented() || !hitbox.is_hovered(window) {
                                    return;
                                }
                                window.prevent_default();
                                let is_start = is_range
                                    && state.is_nearer_start(axis, event.start_position, cx);
                                state.touch_drag = Some(TouchDrag {
                                    start_position: event.start_position,
                                    is_start,
                                });
                            }
                            let Some(drag) = state.touch_drag else {
                                return;
                            };
                            if drag.start_position != event.start_position {
                                return;
                            }
                            cx.stop_propagation();
                            match event.phase {
                                TouchPhase::Started | TouchPhase::Moved => {
                                    state.update_value_by_position(
                                        axis,
                                        event.position,
                                        drag.is_start,
                                        window,
                                        cx,
                                    );
                                }
                                TouchPhase::Ended | TouchPhase::Cancelled => {
                                    state.touch_drag = None;
                                    state.handle_release(cx);
                                }
                            }
                        });
                    });
                },
            )
            .absolute()
            .size_full()
        });
        self.base
            .id("slider-bar-container")
            .test_support()
            .children(self.children)
            .children(touch_layer)
            .when(!self.disabled, |this| {
                this.on_mouse_down(
                    MouseButton::Left,
                    window.listener_for(
                        &self.state,
                        move |state, event: &MouseDownEvent, window, cx| {
                            let is_start =
                                is_range && state.is_nearer_start(axis, event.position, cx);
                            state.update_value_by_position(
                                axis,
                                event.position,
                                is_start,
                                window,
                                cx,
                            );
                        },
                    ),
                )
                .when(!is_range, |this| {
                    this.on_drag(DragSlider(entity_id), |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    })
                    .on_drag_move(window.listener_for(
                        &self.state,
                        move |state, event: &DragMoveEvent<DragSlider>, window, cx| {
                            let DragSlider(id) = event.drag(cx);
                            if *id == entity_id {
                                state.update_value_by_position(
                                    axis,
                                    event.event.position,
                                    false,
                                    window,
                                    cx,
                                );
                            }
                        },
                    ))
                })
            })
    }
}

/// An unstyled slider indicator that records the value-mapping bounds.
#[derive(IntoElement)]
pub struct SliderIndicator {
    state: Entity<SliderState>,
    base: Div,
    children: Vec<AnyElement>,
}

impl SliderIndicator {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            base: div(),
            children: Vec::new(),
        }
    }
}

impl ParentElement for SliderIndicator {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for SliderIndicator {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for SliderIndicator {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for SliderIndicator {}

impl RenderOnce for SliderIndicator {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base
            .id("slider-bar")
            .children(self.children)
            .on_prepaint({
                let state = self.state;
                // Only when they moved: with retained views, an update while
                // the window draws renders every view that reads the state
                // again.
                move |bounds, _, cx| {
                    if state.read(cx).bounds != bounds {
                        state.update(cx, |state, _| state.set_bounds(bounds));
                    }
                }
            })
    }
}

/// An unstyled draggable slider thumb.
#[derive(IntoElement)]
pub struct SliderThumb {
    state: Entity<SliderState>,
    axis: Axis,
    start: bool,
    disabled: bool,
    base: Div,
    children: Vec<AnyElement>,
}

impl SliderThumb {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            axis: Axis::Horizontal,
            start: false,
            disabled: false,
            base: div(),
            children: Vec::new(),
        }
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }
    pub fn start(mut self, start: bool) -> Self {
        self.start = start;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl ParentElement for SliderThumb {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for SliderThumb {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for SliderThumb {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for SliderThumb {}

impl RenderOnce for SliderThumb {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let entity_id = self.state.entity_id();
        let axis = self.axis;
        let start = self.start;
        self.base
            .id(("slider-thumb", start as u32))
            .test_support()
            .children(self.children)
            .when(!self.disabled, |this| {
                this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_drag(DragThumb((entity_id, start)), |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    })
                    .on_drag_move(window.listener_for(
                        &self.state,
                        move |state, event: &DragMoveEvent<DragThumb>, window, cx| {
                            let DragThumb((id, start)) = event.drag(cx);
                            if *id == entity_id {
                                state.update_value_by_position(
                                    axis,
                                    event.event.position,
                                    *start,
                                    window,
                                    cx,
                                );
                            }
                        },
                    ))
            })
            // A caller can give a thumb a role of its own, as for a range's thumbs.
            .a11y_disabled(self.disabled)
    }
}

impl EventEmitter<SliderEvent> for SliderState {}

impl SliderState {
    #[doc(hidden)]
    pub fn percentage(&self) -> Range<f32> {
        self.percentage.clone()
    }

    #[doc(hidden)]
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    #[doc(hidden)]
    pub fn set_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.bounds = bounds;
    }
}
