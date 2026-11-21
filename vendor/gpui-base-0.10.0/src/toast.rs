use crate::TestSupportExt as _;
#[cfg(not(target_family = "wasm"))]
use std::time::Instant;
use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};
#[cfg(target_family = "wasm")]
use web_time::Instant;

use gpui::{
    Anchor, AnyElement, App, Div, ElementId, FocusHandle, InteractiveElement, Interactivity,
    IntoElement, MouseMoveEvent, ParentElement, Pixels, RenderOnce, Role, Stateful,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, canvas, div,
    prelude::FluentBuilder as _, px,
};

use crate::{
    ElementExt as _, StyledExt as _,
    motion::{Spring, spring},
};

/// Motion tokens used by an unstyled toast stack.
#[derive(Clone, Copy, Debug)]
pub struct ToastMotion {
    /// Time scale of the stack's motion.
    ///
    /// This is read as two different quantities, because the stack sequences
    /// one thing and interpolates another. [`ToastManager`] treats it as a
    /// deadline: a toast is present once it has elapsed. The layout treats it
    /// as a spring response, which is the scale the reflow is felt at rather
    /// than the moment it stops — a toast arriving still nudges the stack for
    /// a little longer than this.
    pub duration: Duration,
    /// Duration before an ending toast is unmounted.
    pub exit_duration: Duration,
    /// Visible distance between collapsed toast layers.
    pub collapsed_peek: Pixels,
    /// Distance between expanded toast items.
    pub expanded_gap: Pixels,
    /// Fractional width reduction for each collapsed layer.
    pub collapsed_scale_step: f32,
    /// Number of layers visible while the stack is collapsed.
    pub collapsed_visible: usize,
}

impl ToastMotion {
    /// Create motion matching the shadcn/Sonner toaster.
    pub fn sonner() -> Self {
        Self {
            duration: Duration::from_millis(400),
            exit_duration: Duration::from_millis(200),
            collapsed_peek: px(14.),
            expanded_gap: px(14.),
            collapsed_scale_step: 0.05,
            collapsed_visible: 3,
        }
    }
}

impl Default for ToastMotion {
    fn default() -> Self {
        Self::sonner()
    }
}

/// Persistent private layout state used by [`ToastStack`].
#[derive(Clone, Debug, Default)]
pub struct ToastStackState {
    heights: Rc<RefCell<std::collections::HashMap<ElementId, Pixels>>>,
    width: Rc<std::cell::Cell<Pixels>>,
    hovered: Rc<std::cell::Cell<bool>>,
    focused: Rc<std::cell::Cell<bool>>,
}

impl ToastStackState {
    /// Return whether interaction has expanded the stack.
    pub fn is_expanded(&self) -> bool {
        self.hovered.get() || self.focused.get()
    }
}

/// Options applied when a toast enters a [`ToastManager`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ToastOptions {
    /// Active time before automatic dismissal; `None` disables auto-hide.
    pub timeout: Option<Duration>,
}

#[derive(Debug)]
struct ManagedToast<I, T> {
    id: I,
    value: T,
    status: ToastTransitionStatus,
    timeout_remaining: Option<Duration>,
    transition_elapsed: Duration,
    last_advance: Instant,
}

/// Changes produced when a toast manager advances its lifecycle clock.
#[derive(Debug)]
pub struct ToastAdvance<I, T> {
    /// Whether a mounted toast changed phase or membership.
    pub changed: bool,
    /// Toast ids whose entry transition completed.
    pub presented: Vec<I>,
    /// Toast ids that entered their ending transition.
    pub ending: Vec<I>,
    /// Toast values removed after their ending transition completed.
    pub removed: Vec<(I, T)>,
}

/// Ordered toast storage, lifecycle, auto-hide, limits, and exit coordination.
#[derive(Debug)]
pub struct ToastManager<I, T> {
    entries: VecDeque<ManagedToast<I, T>>,
    transition_duration: Duration,
    exit_duration: Duration,
}

impl<I, T> ToastManager<I, T> {
    /// Create a manager using the supplied motion duration for enter and exit.
    pub fn new(motion: ToastMotion) -> Self {
        Self {
            entries: VecDeque::new(),
            transition_duration: motion.duration,
            exit_duration: motion.exit_duration,
        }
    }

    /// Return the number of mounted toasts, including ending toasts.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether no toast is mounted.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over mounted toast ids, values, and phases in display order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&I, &T, ToastTransitionStatus)> {
        self.entries
            .iter()
            .map(|entry| (&entry.id, &entry.value, entry.status))
    }

    /// Iterate over newest visible active toasts plus ending toasts.
    pub fn visible(&self, limit: usize) -> impl Iterator<Item = (&I, &T, ToastTransitionStatus)> {
        let first = self
            .entries
            .iter()
            .filter(|entry| entry.status != ToastTransitionStatus::Ending)
            .count()
            .saturating_sub(limit);
        let mut active_index = 0usize;
        self.entries.iter().filter_map(move |entry| {
            let visible = if entry.status == ToastTransitionStatus::Ending {
                true
            } else {
                let keep = active_index >= first;
                active_index += 1;
                keep
            };
            visible.then_some((&entry.id, &entry.value, entry.status))
        })
    }

    /// Return a mounted toast value by id.
    pub fn get(&self, id: &I) -> Option<&T>
    where
        I: Eq,
    {
        self.entries
            .iter()
            .find_map(|entry| (&entry.id == id).then_some(&entry.value))
    }
}

impl<I: Clone + Eq, T> ToastManager<I, T> {
    /// Add a newest toast, replacing an existing toast with the same id.
    pub fn push(&mut self, id: I, value: T, options: ToastOptions, now: Instant) -> Option<T> {
        let replaced = self
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .and_then(|index| self.entries.remove(index))
            .map(|entry| entry.value);
        self.entries.push_back(ManagedToast {
            id,
            value,
            status: ToastTransitionStatus::Starting,
            timeout_remaining: options.timeout,
            transition_elapsed: Duration::ZERO,
            last_advance: now,
        });
        replaced
    }

    /// Begin a toast's exit transition, returning whether its state changed.
    pub fn dismiss(&mut self, id: &I, now: Instant) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| &entry.id == id) else {
            return false;
        };
        if entry.status == ToastTransitionStatus::Ending {
            return false;
        }
        entry.status = ToastTransitionStatus::Ending;
        entry.transition_elapsed = Duration::ZERO;
        entry.last_advance = now;
        true
    }

    /// Begin the exit transition for every active toast.
    pub fn dismiss_all(&mut self, now: Instant) -> Vec<I> {
        let mut changed = Vec::new();
        for entry in &mut self.entries {
            if entry.status != ToastTransitionStatus::Ending {
                entry.status = ToastTransitionStatus::Ending;
                entry.transition_elapsed = Duration::ZERO;
                entry.last_advance = now;
                changed.push(entry.id.clone());
            }
        }
        changed
    }

    /// Advance lifecycle time; active timers pause while `paused` is true.
    pub fn advance(&mut self, now: Instant, paused: bool) -> ToastAdvance<I, T> {
        let mut ending = Vec::new();
        let mut presented = Vec::new();
        let mut changed = false;
        for entry in &mut self.entries {
            let delta = now.saturating_duration_since(entry.last_advance);
            entry.last_advance = now;
            match entry.status {
                ToastTransitionStatus::Starting => {
                    entry.transition_elapsed += delta;
                    if entry.transition_elapsed >= self.transition_duration {
                        entry.status = ToastTransitionStatus::Present;
                        entry.transition_elapsed = Duration::ZERO;
                        presented.push(entry.id.clone());
                        changed = true;
                    }
                }
                ToastTransitionStatus::Present if !paused => {
                    if let Some(remaining) = &mut entry.timeout_remaining {
                        *remaining = remaining.saturating_sub(delta);
                        if remaining.is_zero() {
                            entry.status = ToastTransitionStatus::Ending;
                            entry.transition_elapsed = Duration::ZERO;
                            ending.push(entry.id.clone());
                            changed = true;
                        }
                    }
                }
                ToastTransitionStatus::Ending => entry.transition_elapsed += delta,
                ToastTransitionStatus::Present => {}
            }
        }
        let mut removed = Vec::new();
        let mut index = 0;
        while index < self.entries.len() {
            if self.entries[index].status == ToastTransitionStatus::Ending
                && self.entries[index].transition_elapsed >= self.exit_duration
            {
                let entry = self.entries.remove(index).expect("toast index is valid");
                removed.push((entry.id, entry.value));
                changed = true;
            } else {
                index += 1;
            }
        }
        ToastAdvance {
            changed,
            presented,
            ending,
            removed,
        }
    }
}

/// A deep toast-stack element that owns measurement, overlap, and expansion motion.
#[derive(IntoElement)]
pub struct ToastStack {
    id: ElementId,
    base: crate::ObservedElement<Stateful<Div>>,
    style: StyleRefinement,
    state: ToastStackState,
    motion: ToastMotion,
    placement: Anchor,
    focus_handle: Option<FocusHandle>,
    children: Vec<(ElementId, AnyElement)>,
}

impl ToastStack {
    /// Create a toast stack with Base UI-compatible motion.
    pub fn new(id: impl Into<ElementId>, state: ToastStackState) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()).test_support(),
            id,
            style: StyleRefinement::default(),
            state,
            motion: ToastMotion::sonner(),
            placement: Anchor::TopRight,
            focus_handle: None,
            children: Vec::new(),
        }
    }

    /// Add a stably keyed toast item to the stack.
    pub fn item(mut self, id: impl Into<ElementId>, child: impl IntoElement) -> Self {
        self.children.push((id.into(), child.into_any_element()));
        self
    }

    /// Set the stack motion tokens.
    pub fn motion(mut self, motion: ToastMotion) -> Self {
        self.motion = motion;
        self
    }

    /// Set the viewport edge used to anchor stack geometry.
    pub fn placement(mut self, placement: Anchor) -> Self {
        self.placement = placement;
        self
    }

    /// Set the focus scope that expands the stack and pauses auto-hide timers.
    pub fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }
}

impl Styled for ToastStack {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for ToastStack {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.children
            .extend(children.into_iter().enumerate().map(|(index, child)| {
                (
                    ElementId::NamedInteger("toast-stack-child".into(), index as u64),
                    child,
                )
            }));
    }
}

impl InteractiveElement for ToastStack {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(handle);
        self
    }
}

impl StatefulInteractiveElement for ToastStack {}

fn stack_geometry(
    heights: &[Pixels],
    gap: Pixels,
    peek: Pixels,
    anchored_bottom: bool,
) -> (Pixels, Pixels, Vec<(Pixels, Pixels)>) {
    let count = heights.len();
    let expanded_height = heights
        .iter()
        .copied()
        .fold(px(0.), |sum, height| sum + height)
        + gap * count.saturating_sub(1) as f32;
    let front_height = heights.last().copied().unwrap_or(px(0.));
    let collapsed_height = heights
        .iter()
        .enumerate()
        .map(|(index, height)| {
            let rank = count - 1 - index;
            *height + peek * rank as f32
        })
        .fold(
            front_height + peek * count.saturating_sub(1) as f32,
            Pixels::max,
        );
    let offsets = heights
        .iter()
        .enumerate()
        .map(|(index, height)| {
            let rank = count - 1 - index;
            let newer_height = heights[(index + 1)..]
                .iter()
                .copied()
                .fold(px(0.), |sum, height| sum + height);
            let expanded = if anchored_bottom {
                expanded_height - newer_height - gap * rank as f32 - *height
            } else {
                newer_height + gap * rank as f32
            };
            let collapsed = if anchored_bottom {
                collapsed_height - *height - peek * rank as f32
            } else {
                peek * rank as f32
            };
            (collapsed, expanded)
        })
        .collect();
    (collapsed_height, expanded_height, offsets)
}

impl RenderOnce for ToastStack {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focused = self
            .focus_handle
            .as_ref()
            .is_some_and(|handle| handle.contains_focused(window, cx));
        self.state.focused.set(focused);
        let expanded = self.state.is_expanded();
        let keys = self
            .children
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        self.state
            .heights
            .borrow_mut()
            .retain(|id, _| keys.contains(id));
        let measured_by_id = self.state.heights.borrow().clone();
        let heights = keys
            .iter()
            .map(|id| measured_by_id.get(id).copied().unwrap_or(px(0.)))
            .collect::<Vec<_>>();
        let measured = self.state.heights.clone();
        let peek = self.motion.collapsed_peek;
        let gap = self.motion.expanded_gap;
        let scale_step = self.motion.collapsed_scale_step;
        let collapsed_visible = self.motion.collapsed_visible.max(1);
        let stack_width = self.state.width.get();
        let count = self.children.len();
        let anchored_bottom = matches!(
            self.placement,
            Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight
        );
        let (collapsed_height, expanded_height, offsets) = stack_geometry(
            &heights[..count.min(heights.len())],
            gap,
            peek,
            anchored_bottom,
        );
        // The whole stack reflows every time a toast arrives or leaves, so each
        // layer is sprung: one retargeted mid-move carries its velocity into the
        // new layout instead of restarting. Critically damped, because a height
        // or an opacity that overshoots its target reads as a glitch. Geometry
        // settles in pixels, so its tolerance is coarser than the fade's.
        //
        // A bottom-anchored item's position is composed from two of these — the
        // stack height and the item's own offset — and the stack height only
        // acquires its real target once the new toast has been measured in
        // prepaint, a frame after the offsets know theirs. Two springs sharing a
        // config stay proportional to each other only while they also share a
        // start, so that one frame of skew lets the composed position pass its
        // settled value by a fraction of a pixel before arriving. Both springs
        // snap on settling, so it is a transient, not a resting error.
        let geometry = Spring::new(self.motion.duration).with_epsilon(0.1);
        let fade = Spring::new(self.motion.duration);
        let stack_height = spring(
            (self.id.clone(), "height"),
            if expanded {
                expanded_height
            } else {
                collapsed_height
            },
            geometry,
            window,
            cx,
        );
        let items = self
            .children
            .into_iter()
            .enumerate()
            .map(move |(index, (item_id, child))| {
                let (collapsed_offset, expanded_offset) =
                    offsets.get(index).copied().unwrap_or((px(0.), px(0.)));
                let measured = measured.clone();
                let measured_id = item_id.clone();
                let rank = count.saturating_sub(1 + index);
                let target_offset = if expanded {
                    expanded_offset
                } else {
                    collapsed_offset
                };
                let offset = spring(
                    (item_id.clone(), "offset"),
                    target_offset,
                    geometry,
                    window,
                    cx,
                );
                let target_inset = if expanded {
                    px(0.)
                } else {
                    stack_width * (scale_step * rank.min(collapsed_visible - 1) as f32 / 2.)
                };
                let inset = spring(
                    (item_id.clone(), "inset"),
                    target_inset,
                    geometry,
                    window,
                    cx,
                );
                let opacity = spring(
                    (item_id.clone(), "visibility"),
                    if expanded || rank < collapsed_visible {
                        1.
                    } else {
                        0.
                    },
                    fade,
                    window,
                    cx,
                );
                div()
                    .id(item_id)
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .top(offset)
                    .left(inset)
                    .right(inset)
                    .opacity(opacity)
                    .when(!expanded && rank >= collapsed_visible, |this| {
                        this.invisible()
                    })
                    .on_prepaint(move |bounds, _, cx| {
                        let mut heights = measured.borrow_mut();
                        if heights.get(&measured_id).copied() != Some(bounds.size.height) {
                            heights.insert(measured_id.clone(), bounds.size.height);
                            cx.refresh_windows();
                        }
                    })
                    .child(child)
            });

        let hovered_state = self.state.hovered.clone();
        let measured_width = self.state.width.clone();
        self.base
            .relative()
            .h(stack_height)
            .when_some(self.focus_handle, |this, handle| this.track_focus(&handle))
            .child(
                canvas(
                    |_, _, _| {},
                    move |mut bounds, _, window, cx| {
                        if measured_width.replace(bounds.size.width) != bounds.size.width {
                            cx.refresh_windows();
                        }
                        if !expanded {
                            if anchored_bottom {
                                bounds.origin.y += stack_height - collapsed_height;
                            }
                            bounds.size.height = collapsed_height;
                        }
                        window.on_mouse_event(move |event: &MouseMoveEvent, _, _, cx| {
                            let hovered = bounds.contains(&event.position);
                            if hovered_state.replace(hovered) != hovered {
                                cx.refresh_windows();
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(items)
            .refine_style(&self.style)
    }
}

/// The lifecycle phase exposed by a toast root to application-owned presentation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ToastTransitionStatus {
    /// The toast has just been added and may run its enter transition.
    #[default]
    Starting,
    /// The toast is fully present.
    Present,
    /// The toast is closing and remains mounted until its exit transition completes.
    Ending,
}

/// An unstyled semantic toast root. Applications own all presentation and motion.
#[derive(IntoElement)]
pub struct Toast {
    base: crate::ObservedElement<Stateful<Div>>,
    style: StyleRefinement,
    transition_status: ToastTransitionStatus,
    children: Vec<AnyElement>,
}

impl Toast {
    /// Create an unstyled semantic toast in the starting transition phase.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id).test_support(),
            style: StyleRefinement::default(),
            transition_status: ToastTransitionStatus::Starting,
            children: Vec::new(),
        }
    }

    /// Set the lifecycle phase used by application-owned toast presentation.
    pub fn transition_status(mut self, status: ToastTransitionStatus) -> Self {
        self.transition_status = status;
        self
    }

    /// Return the lifecycle phase used by application-owned toast presentation.
    pub fn status(&self) -> ToastTransitionStatus {
        self.transition_status
    }
}

impl Styled for Toast {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Toast {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(children);
    }
}

impl InteractiveElement for Toast {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(handle);
        self
    }
}

impl StatefulInteractiveElement for Toast {}

impl RenderOnce for Toast {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base
            .role(Role::Alert)
            .children(self.children)
            .refine_style(&self.style)
    }
}
