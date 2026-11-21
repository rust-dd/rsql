use std::rc::Rc;

use gpui::{
    AccessibleAction, AnyElement, App, ElementId, FocusHandle, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _,
};

use crate::actions::{Cancel, Confirm, SelectDown, SelectUp};
use crate::{StyledExt as _, TestSupportExt as _, a11y_disabled::A11yDisabledExt as _};

const CONTEXT: &str = "Select";

#[doc(hidden)]
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "space",
            Confirm { secondary: false },
            Some("Select && !Input"),
        ),
        KeyBinding::new("up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm { secondary: false }, Some(CONTEXT)),
        KeyBinding::new(
            if cx.platform_info().is_mac() {
                "cmd-enter"
            } else {
                "ctrl-enter"
            },
            Confirm { secondary: true },
            Some(CONTEXT),
        ),
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
    ]);
}

type OpenChangeHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;
type ActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// An unstyled controlled select root.
///
/// Applications own the trigger and popup presentation, the option collection,
/// and the selected value. This root owns combobox accessibility semantics,
/// keyboard opening and dismissal, and focus transfer between the trigger and
/// popup content.
///
/// GPUI marks the active option on the option element itself rather than on the
/// container, so the application marks its highlighted option with
/// `aria_active_descendant()`; this root cannot do it on the caller's behalf.
#[derive(IntoElement)]
pub struct Select {
    base: crate::ObservedElement<gpui::Stateful<gpui::Div>>,
    open: bool,
    disabled: bool,
    focus_handle: Option<FocusHandle>,
    content_focus_handle: Option<FocusHandle>,
    accessibility_label: Option<SharedString>,
    accessibility_value: Option<SharedString>,
    style: StyleRefinement,
    children: Vec<AnyElement>,
    on_open_change: Option<OpenChangeHandler>,
    key_context: &'static str,
    on_dismiss: Option<ActionHandler>,
    on_confirm: Option<ActionHandler>,
}

impl Select {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id).test_support(),
            open: false,
            disabled: false,
            focus_handle: None,
            content_focus_handle: None,
            accessibility_label: None,
            accessibility_value: None,
            style: StyleRefinement::default(),
            children: Vec::new(),
            on_open_change: None,
            key_context: CONTEXT,
            on_dismiss: None,
            on_confirm: None,
        }
    }

    /// Sets the application-controlled open state.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Prevents keyboard and accessible activation and removes the trigger from tab traversal.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Supplies the focus handle for the select trigger.
    pub fn focus_handle(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }

    /// Supplies the focus handle that receives keyboard navigation while open.
    pub fn content_focus_handle(mut self, focus_handle: &FocusHandle) -> Self {
        self.content_focus_handle = Some(focus_handle.clone());
        self
    }

    /// Sets the accessible name exposed by the controlled root.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    /// Sets the committed value exposed by the controlled root.
    ///
    /// Supply a readable selection title, not the current search query or cursor.
    pub fn accessibility_value(mut self, value: impl Into<SharedString>) -> Self {
        self.accessibility_value = Some(value.into());
        self
    }

    /// Handles requests to update the controlled open state.
    pub fn on_open_change(
        mut self,
        handler: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(handler));
        self
    }

    #[doc(hidden)]
    pub fn key_context(mut self, key_context: &'static str) -> Self {
        self.key_context = key_context;
        self
    }

    /// Handles a dismissal, however it was requested: the Cancel action, or
    /// the accessible activation that closes an open control.
    ///
    /// This runs before the controlled open state is asked to close, so a
    /// caller that commits its pending value on dismissal can still read that
    /// value here.
    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    /// Handles the Confirm action while the select is open.
    ///
    /// Confirming a closed select opens it instead, so this never runs for
    /// that case.
    pub fn on_confirm(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_confirm = Some(Rc::new(handler));
        self
    }
}

impl Styled for Select {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Select {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Select {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let open = self.open;
        let disabled = self.disabled;
        let focus_handle = self.focus_handle;
        let content_focus_handle = self.content_focus_handle;
        let on_open_change = self.on_open_change;
        let on_dismiss = self.on_dismiss;
        let on_confirm = self.on_confirm;

        // Every way of closing runs the same steps. A caller that tracks
        // dismissal has to see one however the popup was closed, and the
        // accessible activation closes exactly what Escape closes.
        let close: ActionHandler = Rc::new({
            let on_open_change = on_open_change.clone();
            let on_dismiss = on_dismiss.clone();
            let focus_handle = focus_handle.clone();
            move |window: &mut Window, cx: &mut App| {
                if let Some(handler) = on_dismiss.as_ref() {
                    handler(window, cx);
                }
                if let Some(handler) = on_open_change.as_ref() {
                    handler(false, window, cx);
                }
                if let Some(handle) = focus_handle.as_ref() {
                    handle.focus(window, cx);
                }
            }
        });

        #[expect(
            clippy::redundant_clone,
            reason = "FocusHandle::clone refreshes tab metadata from the focus map"
        )]
        let focus_handle = focus_handle.clone().filter(|_| !disabled);

        self.base
            .role(Role::ComboBox)
            .aria_expanded(open)
            .when_some(self.accessibility_label, |this, label| {
                this.aria_label(label)
            })
            .when_some(self.accessibility_value, |this, value| {
                this.aria_value(value)
            })
            .key_context(self.key_context)
            .when_some(focus_handle, |this, handle| {
                this.track_focus(&handle.tab_stop(true))
            })
            .when(!disabled, |this| {
                let on_open_change = on_open_change.clone();
                let content_focus_handle = content_focus_handle.clone();
                let close = close.clone();

                // Platform adapters may flatten the trigger child.
                // Expose activation on the semantic root itself.
                this.on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                    if open {
                        close(window, cx);
                        return;
                    }

                    if let Some(handler) = on_open_change.as_ref() {
                        handler(true, window, cx);
                    }
                    if let Some(handle) = content_focus_handle.as_ref() {
                        handle.focus(window, cx);
                    }
                })
            })
            .on_action({
                let on_open_change = on_open_change.clone();
                let content_focus_handle = content_focus_handle.clone();
                move |_: &SelectUp, window, cx| {
                    if disabled {
                        cx.propagate();
                        return;
                    }

                    if !open {
                        if let Some(handler) = on_open_change.as_ref() {
                            handler(true, window, cx);
                        }
                    }

                    if let Some(handle) = content_focus_handle.as_ref() {
                        handle.focus(window, cx);
                    }
                    cx.propagate();
                }
            })
            .on_action({
                let on_open_change = on_open_change.clone();
                let content_focus_handle = content_focus_handle.clone();
                move |_: &SelectDown, window, cx| {
                    if disabled {
                        cx.propagate();
                        return;
                    }

                    if !open {
                        if let Some(handler) = on_open_change.as_ref() {
                            handler(true, window, cx);
                        }
                    }

                    if let Some(handle) = content_focus_handle.as_ref() {
                        handle.focus(window, cx);
                    }
                    cx.propagate();
                }
            })
            .on_action({
                let on_open_change = on_open_change.clone();
                move |_: &Confirm, window, cx| {
                    if disabled {
                        cx.propagate();
                        return;
                    }

                    if open {
                        if let Some(handler) = on_confirm.as_ref() {
                            handler(window, cx);
                        }
                    } else if let Some(handler) = on_open_change.as_ref() {
                        handler(true, window, cx);
                    }

                    if let Some(handle) = content_focus_handle.as_ref() {
                        handle.focus(window, cx);
                    }
                }
            })
            .on_action(move |_: &Cancel, window, cx| {
                if !open {
                    cx.propagate();
                    return;
                }

                cx.stop_propagation();
                close(window, cx);
            })
            .children(self.children)
            .refine_style(&self.style)
            .a11y_disabled(disabled)
    }
}
