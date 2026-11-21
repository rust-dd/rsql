use gpui::{
    AnyElement, App, Axis, Div, ElementId, FocusHandle, InteractiveElement, Interactivity,
    IntoElement, KeyDownEvent, ParentElement, RenderOnce, Role, SharedString, Stateful,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, accesskit, div,
    prelude::FluentBuilder as _,
};
use smallvec::SmallVec;

use crate::{LayoutDirection, StyledExt as _, TestSupportExt as _};

/// An unstyled container that groups a set of controls and owns roving
/// keyboard focus among them.
///
/// This is the behavior primitive behind a styled toolbar. The container
/// exposes `Toolbar` semantics to assistive technology (`Role::Toolbar` with
/// an orientation) and moves focus between its focusable descendants with the
/// arrow keys, so applications do not have to reimplement the roving-focus
/// contract per toolbar. It works with any focusable children — buttons,
/// menu triggers, inputs — because traversal walks the rendered tab stops
/// and constrains them to this subtree, mirroring how `Root` constrains
/// focus-trap cycling.
///
/// Keyboard contract:
///
/// - Left and Right move focus to the previous or next focusable descendant,
///   wrapping around at either end (the same default as Base UI's `loopFocus`).
///   A vertical toolbar ([`Toolbar::axis`]) uses Up and Down instead and
///   reports a vertical orientation.
/// - When `disabled`, the arrow keys do nothing. Hosted controls must be
///   disabled by their owner; the flag only suppresses the toolbar's own
///   navigation.
///
/// The container is not itself a tab stop, so ordinary `Tab` traversal enters
/// and leaves the toolbar through its items, matching the ARIA toolbar
/// pattern.
///
/// An input hosted inside the toolbar keeps its own arrow-key behavior: text
/// inputs consume the arrow keys for caret movement before the toolbar sees
/// them. Place inputs at the trailing end of a horizontal toolbar, as the
/// Base UI Toolbar recommends.
#[derive(IntoElement)]
pub struct Toolbar {
    id: ElementId,
    base: crate::ObservedElement<Stateful<Div>>,
    style: StyleRefinement,
    disabled: bool,
    axis: Axis,
    children: SmallVec<[AnyElement; 4]>,
}

impl Toolbar {
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()).test_support(),
            style: StyleRefinement::default(),
            disabled: false,
            axis: Axis::Horizontal,
            children: SmallVec::new(),
            id,
        }
    }

    /// Disables the toolbar's own keyboard navigation. Hosted controls are
    /// not automatically disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The direction the items run: horizontal (Left/Right, the default) or
    /// vertical (Up/Down). Assistive technology hears the same orientation.
    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }
}

fn step_focus(window: &mut Window, cx: &mut App, forward: bool) {
    if forward {
        window.focus_next(cx);
    } else {
        window.focus_prev(cx);
    }
}

/// Move focus to the next (or previous) focusable element inside `container`.
///
/// Walk the window's tab order in the asked direction, passing over stops
/// outside the container, and land on the first stop inside it. The tab order
/// wraps around the window, so past the container's last stop the walk comes
/// back in at its first however many stops the rest of the window holds, and a
/// stop that a tab index places between two of the container's own is passed
/// over like any other. GPUI's tab order can cycle short of the whole window,
/// so the walk ends at the first repeated stop and focus goes back where it
/// started: a container with no other reachable stop keeps focus where it is.
fn move_focus(container: &FocusHandle, forward: bool, window: &mut Window, cx: &mut App) {
    let Some(start) = window.focused(cx) else {
        return;
    };

    let mut visited = vec![start.clone()];
    loop {
        step_focus(window, cx, forward);
        let Some(focused) = window.focused(cx) else {
            break;
        };
        if visited.contains(&focused) {
            break;
        }
        if container.contains_focused(window, cx) {
            return;
        }
        visited.push(focused);
    }
    window.focus(&start, cx);
}

fn handle_key_down(
    disabled: bool,
    axis: Axis,
    container: &FocusHandle,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut App,
) {
    if disabled {
        return;
    }

    let forward = match (axis, event.keystroke.key.as_str()) {
        // In a right-to-left layout the next item sits to the left.
        (Axis::Horizontal, "left") => Some(LayoutDirection::global(cx).is_rtl()),
        (Axis::Horizontal, "right") => Some(!LayoutDirection::global(cx).is_rtl()),
        (Axis::Vertical, "up") => Some(false),
        (Axis::Vertical, "down") => Some(true),
        _ => None,
    };

    if let Some(forward) = forward {
        move_focus(container, forward, window, cx);
        cx.stop_propagation();
    }
}

impl Styled for Toolbar {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Toolbar {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for Toolbar {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Toolbar {}

impl RenderOnce for Toolbar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            base,
            style,
            disabled,
            axis,
            children,
            ..
        } = self;

        // The handle must survive across frames so containment checks and the
        // key handler refer to the same dispatch-tree node; button.rs uses
        // the same keyed-state pattern for its own focus handle.
        let focus_handle = window
            .use_keyed_state(self.id, cx, |_, cx| cx.focus_handle().tab_stop(false))
            .read(cx)
            .clone();
        let key_handler = {
            let focus_handle = focus_handle.clone();
            move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                handle_key_down(disabled, axis, &focus_handle, event, window, cx);
            }
        };

        base.track_focus(&focus_handle)
            .role(Role::Toolbar)
            .aria_orientation(match axis {
                Axis::Horizontal => accesskit::Orientation::Horizontal,
                Axis::Vertical => accesskit::Orientation::Vertical,
            })
            .on_key_down(key_handler)
            // The handle exists for containment checks only. Without this, a
            // press on blank space or non-focusable content would focus the
            // toolbar itself and take focus away from the rest of the window.
            // Focusable items handle the press first and keep their focus.
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            .children(children)
            .refine_style(&style)
    }
}

/// A semantic subgroup of items within a [`Toolbar`].
///
/// The group carries no behavior or styling of its own: the surrounding
/// toolbar's roving arrow-key focus traverses its items exactly like the
/// toolbar's direct children, because containment follows the element tree.
/// Its value is structure — assistive technology announces the group and its
/// accessible name, so a run of related controls reads as one unit ("Undo",
/// "Redo" inside a "History" group).
///
/// Unlike Base UI's `Toolbar.Group`, the group cannot disable its children.
/// That API propagates through React context into Base UI's own button
/// primitives; GPUI composition offers no equivalent for arbitrary children,
/// and the platform a11y layer exposes no disabled state for a container
/// node. Disabling the hosted controls is the group owner's job.
#[derive(IntoElement)]
pub struct ToolbarGroup {
    base: Stateful<Div>,
    style: StyleRefinement,
    label: Option<SharedString>,
    children: SmallVec<[AnyElement; 4]>,
}

impl ToolbarGroup {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id.into()),
            style: StyleRefinement::default(),
            label: None,
            children: SmallVec::new(),
        }
    }

    /// Sets the accessible name announced for the group, e.g. "History".
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl Styled for ToolbarGroup {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for ToolbarGroup {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for ToolbarGroup {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for ToolbarGroup {}

impl RenderOnce for ToolbarGroup {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // A group is one inline segment of the bar, so its children flow
        // along the row and center on the bar's cross axis; the same neutral
        // geometry `Tab` applies. Spacing between items is the caller's
        // (matching the bar's own gap).
        self.base
            .flex()
            .items_center()
            .role(Role::Group)
            .when_some(self.label, |this, label| this.aria_label(label))
            .children(self.children)
            .refine_style(&self.style)
    }
}
