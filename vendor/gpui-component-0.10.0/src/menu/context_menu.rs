use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui::{
    Anchor, AnyElement, App, Bounds, Context, DismissEvent, Element, ElementId, Entity,
    FocusHandle, Focusable, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId,
    InteractiveElement, IntoElement, LayoutId, LongPressEvent, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, Position, Style, StyleRefinement, Styled, Subscription,
    TouchPhase, Window, anchored, deferred, div, px,
};

use crate::{input::AnyInputState, menu::PopupMenu, root::WindowState};

/// A extension trait for adding a context menu to an element.
pub trait ContextMenuExt: InteractiveElement + ParentElement + Styled {
    /// Add a context menu to the element.
    ///
    /// This will changed the element to be `relative` positioned, and add a child `ContextMenu` element.
    /// Because the `ContextMenu` element is positioned `absolute`, it will not affect the layout of the parent element.
    #[track_caller]
    fn context_menu(
        mut self,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> ContextMenu<Self>
    where
        Self: Sized,
    {
        // The ID must be stable across renders, otherwise the element state
        // (open menu) is lost on every re-render.
        let caller = std::panic::Location::caller();
        let id = self
            .interactivity()
            .element_id
            .clone()
            .map(|id| ElementId::Name(format!("context-menu-{:?}", id).into()))
            .unwrap_or_else(|| ElementId::CodeLocation(*caller));
        ContextMenu::new(id, self).menu(f)
    }
}

impl<E: InteractiveElement + ParentElement + Styled> ContextMenuExt for E {}

/// A context menu that can be shown on right-click.
pub struct ContextMenu<E: ParentElement + Styled + Sized> {
    id: ElementId,
    element: Option<E>,
    menu: Option<Rc<MenuBuilder>>,
    // This is not in use, just for style refinement forwarding.
    _ignore_style: StyleRefinement,
    anchor: Anchor,
}

impl<E: ParentElement + Styled> ContextMenu<E> {
    /// Create a new context menu with the given ID.
    pub fn new(id: impl Into<ElementId>, element: E) -> Self {
        Self {
            id: id.into(),
            element: Some(element),
            menu: None,
            anchor: Anchor::TopLeft,
            _ignore_style: StyleRefinement::default(),
        }
    }

    /// Build the context menu using the given builder function.
    #[must_use]
    fn menu<F>(mut self, builder: F) -> Self
    where
        F: Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    {
        self.menu = Some(Rc::new(builder));
        self
    }

    fn with_element_state<R>(
        &mut self,
        id: &GlobalElementId,
        window: &mut Window,
        cx: &mut App,
        f: impl FnOnce(&mut Self, &mut ContextMenuState, &mut Window, &mut App) -> R,
    ) -> R {
        window.with_optional_element_state::<ContextMenuState, _>(
            Some(id),
            |element_state, window| {
                let mut element_state = element_state.unwrap().unwrap_or_default();
                let result = f(self, &mut element_state, window, cx);
                (result, Some(element_state))
            },
        )
    }
}

impl<E: ParentElement + Styled> ParentElement for ContextMenu<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        if let Some(element) = &mut self.element {
            element.extend(elements);
        }
    }
}

impl<E: ParentElement + Styled> Styled for ContextMenu<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        if let Some(element) = &mut self.element {
            element.style()
        } else {
            &mut self._ignore_style
        }
    }
}

#[cfg(not(feature = "gpui-fast"))]
impl<E: ParentElement + Styled + IntoElement + 'static> IntoElement for ContextMenu<E> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct ContextMenuSharedState {
    menu_view: Option<Entity<PopupMenu>>,
    /// Kept while open so replacing the menu preserves its input's selection
    /// after the input has left the focused-input registry.
    selection_input: Option<AnyInputState>,
    open: bool,
    position: Point<Pixels>,
    /// Registered on this element's dispatch node every frame and never
    /// focused, so the menu can resolve its shortcut hints against the
    /// trigger's key contexts on the frame it opens: GPUI looks a handle up in
    /// the previously rendered frame, where the menu's own element is not yet.
    trigger_focus_handle: Option<FocusHandle>,
    _subscription: Option<Subscription>,
}

pub struct ContextMenuState {
    element: Option<AnyElement>,
    /// Whether this trigger draws the open menu this frame.
    ///
    /// Triggers without an `ElementId` fall back to their code location, so
    /// rows rendered from one call site share the element state and all see
    /// the menu as open. Only the trigger that was pressed draws it: stacked
    /// copies of one `PopupMenu` share an item's pending-click state, and the
    /// covered copies clear it on mouse up before the visible one fires.
    draws_menu: Rc<Cell<bool>>,
    shared_state: Rc<RefCell<ContextMenuSharedState>>,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            element: None,
            draws_menu: Rc::default(),
            shared_state: Rc::new(RefCell::new(ContextMenuSharedState {
                menu_view: None,
                selection_input: None,
                open: false,
                position: Default::default(),
                trigger_focus_handle: None,
                _subscription: None,
            })),
        }
    }
}

/// The deferred menu layer of one trigger.
///
/// Every trigger that shares the open state carries one, but only the trigger
/// whose bounds contain the press builds and lays out the menu, so an open
/// menu is rendered once per frame rather than once per row.
struct DeferredMenu {
    draws: Rc<Cell<bool>>,
    menu_view: Entity<PopupMenu>,
    position: Point<Pixels>,
    anchor: Anchor,
    menu: Option<AnyElement>,
}

impl DeferredMenu {
    fn build_menu(&self, window: &mut Window) -> AnyElement {
        deferred(
            anchored().child(
                div()
                    .w(window.bounds().size.width)
                    .h(window.bounds().size.height)
                    .on_scroll(|_, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        anchored()
                            .position(self.position)
                            .snap_to_window_with_margin(px(8.))
                            .anchor(self.anchor)
                            .child(self.menu_view.clone()),
                    ),
            ),
        )
        .with_priority(gpui_base::POPUP_PRIORITY)
        .into_any()
    }
}

#[cfg(not(feature = "gpui-fast"))]
impl IntoElement for DeferredMenu {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for DeferredMenu {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
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
    ) -> (LayoutId, ()) {
        // An empty absolute placeholder, where the menu's `anchored` root used
        // to sit, so the trigger's own layout is unchanged.
        let style = Style {
            position: Position::Absolute,
            ..Style::default()
        };
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.draws.get() {
            return;
        }
        let mut menu = self.build_menu(window);
        menu.prepaint_as_root(bounds.origin, window.viewport_size().into(), window, cx);
        self.menu = Some(menu);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(menu) = &mut self.menu {
            menu.paint(window, cx);
        }
    }
}

impl<E: ParentElement + Styled + IntoElement + 'static> Element for ContextMenu<E> {
    type RequestLayoutState = ContextMenuState;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        // At the pointer, the menu opens toward the end edge.
        let anchor = gpui_base::logical_anchor(self.anchor, gpui_base::LayoutDirection::global(cx));

        self.with_element_state(
            id.unwrap(),
            window,
            cx,
            |this, state: &mut ContextMenuState, window, cx| {
                let (position, open) = {
                    let shared_state = state.shared_state.borrow();
                    (shared_state.position, shared_state.open)
                };
                state
                    .shared_state
                    .borrow_mut()
                    .trigger_focus_handle
                    .get_or_insert_with(|| cx.focus_handle());
                let menu_view = state.shared_state.borrow().menu_view.clone();
                let draws_menu = Rc::new(Cell::new(false));
                let menu_element = menu_view
                    .filter(|menu| open && !menu.read(cx).is_empty())
                    .map(|menu_view| DeferredMenu {
                        draws: draws_menu.clone(),
                        menu_view,
                        position,
                        anchor,
                        menu: None,
                    });

                let mut element = this
                    .element
                    .take()
                    .expect("Element should exists.")
                    .children(menu_element)
                    .into_any_element();

                let layout_id = element.request_layout(window, cx);

                (
                    layout_id,
                    ContextMenuState {
                        element: Some(element),
                        draws_menu,
                        shared_state: state.shared_state.clone(),
                    },
                )
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(trigger_focus) = request_layout
            .shared_state
            .borrow()
            .trigger_focus_handle
            .as_ref()
        {
            window.set_focus_handle(trigger_focus, cx);
        }
        let position = request_layout.shared_state.borrow().position;
        request_layout.draws_menu.set(bounds.contains(&position));
        if let Some(element) = &mut request_layout.element {
            element.prepaint(window, cx);
        }
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let builder = self.menu.clone();
        let shared_state = request_layout.shared_state.clone();

        // A finger has no right button, so a long press opens the menu. This
        // listener goes in before the children paint, so an input inside the
        // trigger claims its long press first. Rich-text selection is handled
        // by the window layer, which runs later; yield over its text geometry.
        window.on_mouse_event({
            let shared_state = shared_state.clone();
            let builder = builder.clone();
            let hitbox = hitbox.clone();
            move |event: &LongPressEvent, phase, window, cx| {
                if phase.bubble()
                    && event.phase == TouchPhase::Started
                    && !window.default_prevented()
                    && hitbox.is_hovered(window)
                    && !gpui_base::TextSelection::is_selectable_at(event.start_position, window, cx)
                {
                    window.prevent_default();
                    open_menu(&shared_state, &builder, event.start_position, window, cx);
                }
            }
        });

        // Register before child paint so bubbling reaches the innermost
        // trigger first, after its input has had a chance to take focus.
        let hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase.bubble() && event.button == MouseButton::Right && hitbox.is_hovered(window) {
                cx.stop_propagation();
                open_menu(&shared_state, &builder, event.position, window, cx);
            }
        });

        if let Some(element) = &mut request_layout.element {
            element.paint(window, cx);
        }
    }
}

type MenuBuilder = dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu;

/// Builds the menu and shows it at `position`.
fn open_menu(
    shared_state: &Rc<RefCell<ContextMenuSharedState>>,
    builder: &Option<Rc<MenuBuilder>>,
    position: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    // Capture the focused element to restore focus to on dismiss.
    // If focus is still on the previous menu, keep its captured focus.
    let previous_focus_handle = window.focused(cx).and_then(|focused| {
        let shared_state = shared_state.borrow();
        match shared_state.menu_view.as_ref() {
            Some(menu) if menu.read(cx).focus_handle == focused => {
                menu.read(cx).previous_focus_handle.clone()
            }
            _ => Some(focused),
        }
    });

    // Retain the old menu's owner before clearing replacement state.
    let previous_selection_input = {
        let state = shared_state.borrow();
        let menu_focused = state.menu_view.as_ref().is_some_and(|menu| {
            let focus = menu.focus_handle(cx);
            focus.is_focused(window) || focus.contains_focused(window, cx)
        });
        state
            .selection_input
            .as_ref()
            .filter(|input| menu_focused || input.focus_handle(cx).is_focused(window))
            .cloned()
    };
    {
        let mut shared_state = shared_state.borrow_mut();
        // Clear any existing menu view to allow immediate replacement
        // Set the new position and open the menu
        shared_state.menu_view = None;
        shared_state.selection_input = None;
        shared_state._subscription = None;
        shared_state.position = position;
        shared_state.open = true;
    }

    // Use defer to build the menu in the next frame, avoiding race conditions
    window.defer(cx, {
        let shared_state = shared_state.clone();
        let builder = builder.clone();
        move |window, cx| {
            // Resolve after pointer dispatch: the inner input has
            // now taken focus and published its state, even if it
            // was unfocused in the last rendered frame.
            let selection_input =
                WindowState::try_update(window, cx, |state, _, _| state.focused_input.clone())
                    .flatten()
                    .filter(|input| input.focus_handle(cx).is_focused(window))
                    .or(previous_selection_input);
            let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
                let Some(build) = &builder else {
                    return menu;
                };
                build(menu, window, cx)
            });
            let trigger_focus_handle = shared_state.borrow().trigger_focus_handle.clone();
            menu.update(cx, |menu, cx| {
                menu.set_trigger_focus(trigger_focus_handle, cx);
                menu.set_previous_focus(previous_focus_handle, cx);
            });

            // A menu on another element must not keep the last
            // focused input highlighted. An explicit action target
            // takes precedence over where the press landed.
            let selection_input = selection_input.filter(|input| {
                let Some(bounds) = input.input_bounds(cx) else {
                    return false;
                };
                match menu.read(cx).action_context.as_ref() {
                    Some(target) => target == &input.focus_handle(cx),
                    None => bounds.contains(&position),
                }
            });
            if let Some(input) = selection_input.as_ref() {
                input.set_selection_focus(Some(menu.focus_handle(cx)), cx);
            }

            // Focus before the menu's first frame, not while prepainting it:
            // a frame reports one focused accessibility node, and the element
            // that held focus may already have claimed it by then.
            menu.focus_handle(cx).focus(window, cx);

            // Set up the subscription for dismiss handling.
            // Hold a Weak here, not a strong clone: the closure
            // would otherwise close the cycle
            // `shared_state -> _subscription -> closure ->
            // shared_state`, so a menu left open when the window
            // closes leaks its PopupMenu entity.
            let _subscription = window.subscribe(&menu, cx, {
                let shared_state = Rc::downgrade(&shared_state);
                move |_, _: &DismissEvent, window, _cx| {
                    if let Some(shared_state) = shared_state.upgrade() {
                        let mut state = shared_state.borrow_mut();
                        state.open = false;
                        state.selection_input = None;
                        window.refresh();
                    }
                }
            });

            // Update the shared state with the built menu and subscription
            {
                let mut state = shared_state.borrow_mut();
                state.menu_view = Some(menu.clone());
                state.selection_input = selection_input;
                state._subscription = Some(_subscription);
                window.refresh();
            }
        }
    });
}
