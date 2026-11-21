use std::rc::Rc;

use gpui::{
    Anchor, AnyElement, App, Context, DismissEvent, Element, ElementId, Entity, FocusHandle,
    Focusable, GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId,
    RenderOnce, SharedString, Styled, Window, prelude::FluentBuilder,
};

use crate::{Selectable, button::Button, menu::PopupMenu, popover::Popover};

/// A dropdown menu trait for buttons and other interactive elements
pub trait DropdownMenu: Styled + Selectable + InteractiveElement + IntoElement + 'static {
    /// Create a dropdown menu with the given items, anchored to the TopLeft corner
    fn dropdown_menu(
        self,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        self.dropdown_menu_with_anchor(Anchor::TopLeft, f)
    }

    /// Create a dropdown menu with the given items, anchored to the given corner
    fn dropdown_menu_with_anchor(
        mut self,
        anchor: impl Into<Anchor>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        let id = self.interactivity().element_id.clone();

        DropdownMenuPopover::new(id.unwrap_or(0.into()), anchor, self, f)
    }
}

impl DropdownMenu for Button {}

#[derive(IntoElement)]
pub struct DropdownMenuPopover<T: Selectable + IntoElement + 'static> {
    id: ElementId,
    anchor: Anchor,
    trigger: T,
    builder: Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>,
    on_open_change: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
}

impl<T> DropdownMenuPopover<T>
where
    T: Selectable + IntoElement + 'static,
{
    fn new(
        id: ElementId,
        anchor: impl Into<Anchor>,
        trigger: T,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        Self {
            id: SharedString::from(format!("dropdown-menu:{:?}", id)).into(),
            anchor: anchor.into(),
            trigger,
            builder: Rc::new(builder),
            on_open_change: None,
        }
    }

    /// Set the anchor corner for the dropdown menu popover.
    pub fn anchor(mut self, anchor: impl Into<Anchor>) -> Self {
        self.anchor = anchor.into();
        self
    }

    /// Add a callback to be called when the menu opens or closes.
    ///
    /// The `&bool` parameter is the **new open state**.
    pub fn on_open_change(
        mut self,
        callback: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(callback));
        self
    }
}

#[derive(Default)]
struct DropdownMenuState {
    menu: Option<Entity<PopupMenu>>,
}

impl<T> RenderOnce for DropdownMenuPopover<T>
where
    T: Selectable + IntoElement + 'static,
{
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        TriggerFocus::new(self.id.clone(), move |trigger_focus, window, cx| {
            self.render_popover(trigger_focus, window, cx)
                .into_any_element()
        })
    }
}

impl<T> DropdownMenuPopover<T>
where
    T: Selectable + IntoElement + 'static,
{
    fn render_popover(
        self,
        trigger_focus: FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> Popover {
        let builder = self.builder.clone();
        let menu_state =
            window.use_keyed_state(self.id.clone(), cx, |_, _| DropdownMenuState::default());

        Popover::new(SharedString::from(format!("popover:{}", self.id)))
            .appearance(false)
            .overlay_closable(false)
            .trigger(self.trigger)
            .anchor(self.anchor)
            .when_some(self.on_open_change, |this, callback| {
                this.on_open_change(move |open, window, cx| callback(open, window, cx))
            })
            .content(move |_, window, cx| {
                // Here is special logic to only create the PopupMenu once and reuse it.
                // Because this `content` will called in every time render, so we need to store the menu
                // in state to avoid recreating at every render.
                //
                // And we also need to rebuild the menu when it is dismissed, to rebuild menu items
                // dynamically for support `dropdown_menu` method, so we listen for DismissEvent below.
                let menu = match menu_state.read(cx).menu.clone() {
                    Some(menu) => menu,
                    None => {
                        let builder = builder.clone();
                        let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
                            builder(menu, window, cx)
                        });
                        menu.update(cx, |menu, cx| {
                            menu.set_trigger_focus(Some(trigger_focus.clone()), cx)
                        });
                        menu_state.update(cx, |state, _| {
                            state.menu = Some(menu.clone());
                        });
                        menu.focus_handle(cx).focus(window, cx);

                        // Listen for dismiss events from the PopupMenu to close the popover.
                        //
                        // Hold Weak handles here, not strong clones: the listener
                        // lives as long as `menu`, which `menu_state` owns, so a
                        // strong capture would close the cycle
                        // `menu_state -> menu -> listener -> menu_state` and leak
                        // the `PopupMenu` and `DropdownMenuState` when the trigger
                        // stops being rendered while the menu is open.
                        let popover_state = cx.entity().downgrade();
                        window
                            .subscribe(&menu, cx, {
                                let menu_state = menu_state.downgrade();
                                move |_, _: &DismissEvent, window, cx| {
                                    if let Some(popover_state) = popover_state.upgrade() {
                                        popover_state.update(cx, |state, cx| {
                                            state.dismiss(window, cx);
                                        });
                                    }
                                    _ = menu_state.update(cx, |state, _| {
                                        state.menu = None;
                                    });
                                }
                            })
                            .detach();

                        menu
                    }
                };

                menu
            })
    }
}

type TriggerFocusBuild = Box<dyn FnOnce(FocusHandle, &mut Window, &mut App) -> AnyElement>;

/// Registers a focus handle on the trigger's dispatch node without ever
/// focusing it, so the menu opened from the trigger can resolve its shortcut
/// hints against the trigger's key contexts on the frame it opens. GPUI looks
/// a handle up in the previously rendered frame; the trigger was in it when
/// the menu was not yet.
struct TriggerFocus {
    id: ElementId,
    build: Option<TriggerFocusBuild>,
}

#[derive(Default)]
struct TriggerFocusState {
    focus_handle: Option<FocusHandle>,
}

struct TriggerFocusFrame {
    focus_handle: FocusHandle,
    child: AnyElement,
}

impl TriggerFocus {
    fn new(
        id: ElementId,
        build: impl FnOnce(FocusHandle, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id,
            build: Some(Box::new(build)),
        }
    }
}

#[cfg(not(feature = "gpui-fast"))]
impl IntoElement for TriggerFocus {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TriggerFocus {
    type RequestLayoutState = TriggerFocusFrame;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let focus_handle =
            window.with_optional_element_state::<TriggerFocusState, _>(id, |state, _| {
                let mut state = state.flatten().unwrap_or_default();
                let focus_handle = state
                    .focus_handle
                    .get_or_insert_with(|| cx.focus_handle())
                    .clone();
                (focus_handle, Some(state))
            });
        let build = self.build.take().expect("TriggerFocus is laid out once");
        let mut child = build(focus_handle.clone(), window, cx);
        let layout_id = child.request_layout(window, cx);

        (
            layout_id,
            TriggerFocusFrame {
                focus_handle,
                child,
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        frame: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.set_focus_handle(&frame.focus_handle, cx);
        frame.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        frame: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        frame.child.paint(window, cx);
    }
}
