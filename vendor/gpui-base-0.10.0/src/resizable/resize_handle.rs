use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, App, Axis, Div, Element, ElementId, Entity, GlobalElementId, Hitbox,
    HitboxBehavior, InteractiveElement, Interactivity, IntoElement, MouseButton, MouseDownEvent,
    MouseExitEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point, Render,
    StatefulInteractiveElement, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::{AxisExt as _, InteractiveElementExt, TestSupportExt as _, theme::ActiveTheme as _};

pub(crate) const HANDLE_PADDING: Pixels = px(4.);
pub(crate) const HANDLE_SIZE: Pixels = px(1.);

type DragConstructor<T, E> = Box<dyn Fn(&T, &Point<Pixels>, &mut Window, &mut App) -> Entity<E>>;

/// Create a resize handle: a band that resizes whatever it edges when it is
/// dragged.
///
/// Base owns the band -- where it sits, how wide it is, its cursor, and the
/// Idle → Hovered → Pressed → Dragging progression its appearance is told
/// about. What a drag resizes is the caller's. Give the handle a payload with
/// [`ResizeHandle::on_drag`] and follow that payload with
/// [`on_drag_move`](InteractiveElement::on_drag_move), the way a slider thumb
/// does; any other listener -- a double click that restores a default size, a
/// hover that a surrounding drawer has to count as its own -- goes on the
/// handle through the interactive element traits.
///
/// The band occludes. While the pointer is on it nothing behind it reads as
/// hovered, its own container included, so a container that has to know the
/// pointer is still over it listens on the handle as well.
///
/// The band's element id, its `"handle"` group, its occlusion and its drag
/// are base's: an `id`, `group`, hitbox behavior or `on_drag` set through the
/// interactive element traits is replaced when the band is laid out.
///
/// ```ignore
/// resize_handle("sidebar-edge", Axis::Horizontal)
///     .inside(HandleEdge::Trailing)
///     .on_drag(ResizeSidebar, |drag, _, _, cx| cx.new(|_| drag.clone()))
///     .on_double_click(|_, _, cx| restore_default_width(cx))
/// ```
pub fn resize_handle<T: 'static, E: 'static + Render>(
    id: impl Into<ElementId>,
    axis: Axis,
) -> ResizeHandle<T, E> {
    ResizeHandle::new(id, axis)
}

/// Draws the visible part of a resize handle.
///
/// Returning `None` keeps the built-in line, so a renderer can override some
/// handles and leave the rest alone.
pub type ResizeHandleRenderer =
    Rc<dyn Fn(&ResizeHandleContext, &mut Window, &mut App) -> Option<AnyElement>>;

/// What a [`ResizeHandleRenderer`] is told about the handle it is drawing.
///
/// The hit area, the cursor and the drag itself stay with the handle; a
/// renderer only supplies what is painted inside it.
pub struct ResizeHandleContext {
    axis: Axis,
    edge: Option<HandleEdge>,
    state: ResizeHandleState,
}

impl ResizeHandleContext {
    /// The axis the handle resizes along: `Horizontal` for a vertical divider
    /// between two side-by-side panels.
    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// The edge of its container this handle hugs, or `None` for a handle
    /// straddling the boundary it resizes.
    ///
    /// A hugging handle's line is the container's outermost pixel, so anything
    /// a renderer centres on it crosses the boundary; see [`HandleEdge`] for
    /// what the container does with that.
    pub fn edge(&self) -> Option<HandleEdge> {
        self.edge
    }

    /// Whether the pointer currently owns this handle.
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// How far the pointer has gone with this handle.
    pub fn state(&self) -> ResizeHandleState {
        self.state
    }
}

/// How far the pointer has gone with a resize handle.
///
/// A drag takes the pointer out of the handle's own band within a pixel or
/// two, so GPUI's hover reads false for most of a drag and cannot stand in for
/// `Dragging`. Base tracks the progression instead, and a renderer reads it
/// through [`ResizeHandleContext::state`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResizeHandleState {
    /// The pointer is somewhere else.
    #[default]
    Idle,
    /// The pointer is over the handle's band.
    Hovered,
    /// The pointer went down on the handle and has not moved since.
    Pressed,
    /// The handle is being dragged.
    Dragging,
}

impl ResizeHandleState {
    /// Whether the pointer owns the handle -- pressed on it, or dragging it.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Pressed | Self::Dragging)
    }
}

/// Which edge of its own container a handle hugs.
///
/// A handle named no edge straddles the boundary it resizes, half its band on
/// either side, which is what a divider between two panels of a group wants.
/// A dock's own edge handle cannot: [`dock_frame`] clips to the dock's box, so
/// the half hanging outside is cut away -- what it paints and what it
/// hit-tests alike, which is why the outer half of a dock's grab band has
/// never actually been grabbable. Naming the edge moves the whole band inside.
///
/// The hairline stays on the boundary itself: it is the container's outermost
/// pixel, the one the neighbour's content butts up against. Moving it inward
/// by even a pixel leaves that pixel of the container showing past the line
/// on one side, or a gap before it on the other, along the whole seam. What a
/// renderer paints on top of the line -- an indicator thicker than the line,
/// centred on it -- overhangs the boundary, and the container's clip takes
/// the outer half off it unless the renderer defers that part. Only that part:
/// a deferred element paints after the whole tree, and no priority puts it
/// beneath the application's own deferred content, so a deferred line would
/// cut straight through a popover opened at the default priority from a panel
/// drawn before this container. A renderer learns which edge it is drawing for
/// from [`ResizeHandleContext::edge`].
///
/// [`dock_frame`]: crate::dock::dock_frame
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleEdge {
    /// Where the axis starts: the left edge of a horizontal handle's
    /// container, the top edge of a vertical one's.
    Leading,
    /// Where the axis ends.
    Trailing,
}

/// A band that resizes along one axis when it is dragged, carrying a `T` that
/// the drag renders as an `E`. See [`resize_handle`].
pub struct ResizeHandle<T: 'static, E: 'static + Render> {
    id: ElementId,
    axis: Axis,
    edge: Option<HandleEdge>,
    drag: Option<(T, DragConstructor<T, E>)>,
    appearance: Option<ResizeHandleRenderer>,
    /// The band itself. Listeners a caller attaches through the interactive
    /// element traits land here, on the element the pointer actually hits;
    /// base places and sizes it when it is laid out.
    base: Div,
}

impl<T: 'static, E: 'static + Render> ResizeHandle<T, E> {
    fn new(id: impl Into<ElementId>, axis: Axis) -> Self {
        let id = id.into();
        Self {
            id,
            edge: None,
            drag: None,
            appearance: None,
            axis,
            base: div(),
        }
    }

    /// Hand the painted part of this handle to `appearance`.
    pub fn with_appearance(mut self, appearance: ResizeHandleRenderer) -> Self {
        self.appearance = Some(appearance);
        self
    }

    /// Start a drag carrying `value` when the band is pressed and moved.
    ///
    /// A band takes one drag, and this is it: use this rather than
    /// [`StatefulInteractiveElement::on_drag`], which it shadows.
    ///
    /// GPUI tells drags apart by the type of their payload, so the drag
    /// carries `value` itself, and a caller following it with
    /// [`on_drag_move`](InteractiveElement::on_drag_move) listens for a `T`.
    pub fn on_drag(
        mut self,
        value: T,
        f: impl Fn(&T, &Point<Pixels>, &mut Window, &mut App) -> Entity<E> + 'static,
    ) -> Self {
        self.drag = Some((value, Box::new(f)));
        self
    }

    /// Keep the whole handle inside its container, hugging `edge`, instead of
    /// straddling the boundary it resizes.
    pub fn inside(mut self, edge: HandleEdge) -> Self {
        self.edge = Some(edge);
        self
    }
}

impl<T: 'static, E: 'static + Render> InteractiveElement for ResizeHandle<T, E> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl<T: 'static, E: 'static + Render> StatefulInteractiveElement for ResizeHandle<T, E> {}

impl<T: 'static, E: 'static + Render> InteractiveElementExt for ResizeHandle<T, E> {}

/// One handle's [`ResizeHandleState`], shared between the element and the
/// mouse listeners it registers.
///
/// The `Rc` is load-bearing. `with_element_state` hands back a clone and a
/// bare `Cell` clones by value, so a listener holding one wrote its progress
/// into a copy that died with the event: the stored state never left `Idle`
/// and `is_active` never once read true.
#[derive(Default, Debug, Clone)]
struct SharedHandleState {
    state: Rc<Cell<ResizeHandleState>>,
}

impl SharedHandleState {
    fn get(&self) -> ResizeHandleState {
        self.state.get()
    }

    /// Reports whether the state actually changed, so a listener repaints the
    /// window only when there is something new to paint.
    fn set(&self, state: ResizeHandleState) -> bool {
        let changed = self.state.get() != state;
        self.state.set(state);
        changed
    }
}

#[cfg(not(feature = "gpui-fast"))]
impl<T: 'static, E: 'static + Render> IntoElement for ResizeHandle<T, E> {
    type Element = ResizeHandle<T, E>;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl<T: 'static, E: 'static + Render> Element for ResizeHandle<T, E> {
    type RequestLayoutState = AnyElement;
    /// The band's own hitbox, so the listeners in `paint` can ask whether the
    /// pointer is really on the handle rather than merely within its bounds.
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let neg_offset = -HANDLE_PADDING;
        let axis = self.axis;
        let edge = self.edge;
        // Sizes are border-box: the extent has to name the whole band, padding
        // included, or the content box resolves to zero and the hairline
        // overflows into the padding, landing wherever the padding happens to
        // push it.
        let hug_extent = HANDLE_SIZE + HANDLE_PADDING;
        let straddle_extent = HANDLE_SIZE + HANDLE_PADDING * 2.;
        // The caller's listeners are already on it; what base adds is where the
        // band goes and what it holds.
        let base = std::mem::replace(&mut self.base, div());
        let drag = self.drag.take();

        window.with_element_state(id.unwrap(), |state, window| {
            let state: SharedHandleState = state.unwrap_or_default();

            let bg_color = handle_color(&cx.theme(), state.get().is_active());

            let mut el = base
                .id(self.id.clone())
                .test_support()
                .occlude()
                .absolute()
                .flex_shrink_0()
                .group("handle")
                .when_some(drag, |this, (value, on_drag)| {
                    this.on_drag(value, move |value, position, window, cx| {
                        on_drag(value, &position, window, cx)
                    })
                })
                .map(|this| match (edge, axis) {
                    // Hugging an edge: the whole band is inside the container,
                    // padded on the inner side only, so the hairline is the
                    // container's outermost pixel -- the seam itself.
                    // FIXME: Improve this to let the scroll bar have px(HANDLE_PADDING)
                    (Some(HandleEdge::Trailing), Axis::Horizontal) => this
                        .cursor_col_resize()
                        .top_0()
                        .right_0()
                        .h_full()
                        .w(hug_extent)
                        .pl(HANDLE_PADDING),
                    (Some(HandleEdge::Leading), Axis::Horizontal) => this
                        .cursor_col_resize()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(hug_extent)
                        .pr(HANDLE_PADDING),
                    (Some(HandleEdge::Trailing), Axis::Vertical) => this
                        .cursor_row_resize()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .h(hug_extent)
                        .pt(HANDLE_PADDING),
                    (Some(HandleEdge::Leading), Axis::Vertical) => this
                        .cursor_row_resize()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(hug_extent)
                        .pb(HANDLE_PADDING),
                    // Straddling the boundary: half the band on either side.
                    (None, Axis::Horizontal) => this
                        .cursor_col_resize()
                        .top_0()
                        .left(neg_offset)
                        .h_full()
                        .w(straddle_extent)
                        .px(HANDLE_PADDING),
                    (None, Axis::Vertical) => this
                        .cursor_row_resize()
                        .top(neg_offset)
                        .left_0()
                        .w_full()
                        .h(straddle_extent)
                        .py(HANDLE_PADDING),
                })
                .child(
                    // A renderer that declines — or is absent — leaves the
                    // built-in line, so overriding one handle never obliges a
                    // caller to redraw them all. What it paints stays in tree
                    // order, under the container's own mask: a divider is
                    // part of the panel it edges, and anything the
                    // application floats over that panel has to cover it.
                    self.appearance
                        .as_ref()
                        .and_then(|appearance| {
                            appearance(
                                &ResizeHandleContext {
                                    axis,
                                    edge,
                                    state: state.get(),
                                },
                                window,
                                cx,
                            )
                        })
                        .unwrap_or_else(|| {
                            div()
                                // The line fills the handle's content box
                                // exactly, so it has nothing to give: a
                                // shrinkable child collapses with it.
                                .flex_none()
                                .bg(bg_color)
                                .group_hover("handle", |this| this.bg(bg_color))
                                .when(axis.is_horizontal(), |this| this.h_full().w(HANDLE_SIZE))
                                .when(axis.is_vertical(), |this| this.w_full().h(HANDLE_SIZE))
                                .into_any_element()
                        }),
                )
                .into_any_element();

            let layout_id = el.request_layout(window, cx);

            ((layout_id, el), state)
        })
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        request_layout.prepaint(window, cx);
        // After the child, deliberately. The band's own div occludes, and a
        // hit test stops at the first hitbox that does, so a hitbox inserted
        // before it would sit underneath and never read as hovered. Inserted
        // here it sits directly on top of the band and directly under whatever
        // is painted after this handle -- a sheet's overlay, a toast -- which
        // is exactly the ordering `is_hovered_at` should answer from.
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        request_layout.paint(window, cx);

        // Hovered and pressed are answered by the hitbox, not by the bounds.
        // Bounds containment reads true through anything painted over the
        // handle, so a divider under a sheet lit up as the pointer crossed
        // where it lay; the hitbox is occluded by that sheet and says no.
        let hitbox = hitbox.clone();

        window.with_element_state(id.unwrap(), |state: Option<SharedHandleState>, window| {
            let state = state.unwrap_or_default();

            window.on_mouse_event({
                let state = state.clone();
                let hitbox = hitbox.clone();
                move |ev: &MouseDownEvent, phase, window, _| {
                    if phase.bubble()
                        && ev.button == MouseButton::Left
                        && hitbox.is_hovered_at(ev.position, window)
                        && state.set(ResizeHandleState::Pressed)
                    {
                        window.refresh();
                    }
                }
            });

            window.on_mouse_event({
                let state = state.clone();
                let hitbox = hitbox.clone();
                move |ev: &MouseMoveEvent, phase, window, cx| {
                    if phase.capture()
                        && ev.pressed_button != Some(MouseButton::Left)
                        && state.get().is_active()
                    {
                        state.set(ResizeHandleState::Idle);
                        cx.stop_active_drag(window);
                        window.refresh();
                    }
                    if !phase.bubble() {
                        return;
                    }

                    // A press that moves is a drag, and stays one until the
                    // button comes back up: by the second frame the pointer is
                    // outside this nine-pixel band, so where it is says nothing
                    // about whether the handle is still being dragged.
                    let next = match state.get() {
                        engaged if engaged.is_active() => ResizeHandleState::Dragging,
                        _ if hitbox.is_hovered_at(ev.position, window) => {
                            ResizeHandleState::Hovered
                        }
                        _ => ResizeHandleState::Idle,
                    };
                    if state.set(next) {
                        window.refresh();
                    }
                }
            });

            window.on_mouse_event({
                let state = state.clone();
                move |_: &MouseExitEvent, phase, window, cx| {
                    if phase.capture() && state.get().is_active() {
                        state.set(ResizeHandleState::Idle);
                        cx.stop_active_drag(window);
                        window.refresh();
                    }
                }
            });

            window.on_mouse_event({
                let state = state.clone();
                let hitbox = hitbox.clone();
                move |ev: &MouseUpEvent, _, window, _| {
                    if !state.get().is_active() {
                        return;
                    }

                    // Releasing over the handle leaves it hovered. Going
                    // straight to idle there would drop the indicator for one
                    // frame and bring it back under a pointer that never left.
                    let next = if hitbox.is_hovered_at(ev.position, window) {
                        ResizeHandleState::Hovered
                    } else {
                        ResizeHandleState::Idle
                    };
                    if state.set(next) {
                        window.refresh();
                    }
                }
            });

            ((), state)
        });
    }
}

/// What a resize handle paints, given the active theme.
///
/// Projected colors win; without them the handle resolves from the tokens that
/// already mean these two states everywhere else -- `border` for a divider at
/// rest, `ring` for the thing the pointer currently owns. Before this the
/// unprojected answer was `Hsla::default()`, which is transparent, so a
/// consumer with no styled façade had no divider at all.
pub(crate) fn handle_color(theme: &crate::Theme, active: bool) -> gpui::Hsla {
    if active {
        theme
            .resizable
            .active_handle
            .unwrap_or(theme.tokens.colors.ring)
    } else {
        theme.resizable.handle.unwrap_or(theme.tokens.colors.border)
    }
}
