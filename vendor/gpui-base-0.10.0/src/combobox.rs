use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FocusHandle, IntoElement, KeyBinding, ParentElement, RenderOnce,
    StyleRefinement, Styled, Window, prelude::FluentBuilder as _,
};

use crate::{
    Select,
    actions::{Cancel, Confirm, SelectDown, SelectUp},
    styled::StyledExt as _,
};

const CONTEXT: &str = "Combobox";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
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

/// An unstyled controlled combobox root.
///
/// Applications own the trigger, popup, searchable collection, selection, and
/// appearance. This root owns combobox semantics and keyboard focus transfer.
#[derive(IntoElement)]
pub struct Combobox {
    id: ElementId,
    open: bool,
    disabled: bool,
    focus_handle: Option<FocusHandle>,
    content_focus_handle: Option<FocusHandle>,
    style: StyleRefinement,
    children: Vec<AnyElement>,
    on_open_change: Option<OpenChangeHandler>,
    on_confirm: Option<ActionHandler>,
    on_dismiss: Option<ActionHandler>,
}

impl Combobox {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            open: false,
            disabled: false,
            focus_handle: None,
            content_focus_handle: None,
            style: StyleRefinement::default(),
            children: Vec::new(),
            on_open_change: None,
            on_confirm: None,
            on_dismiss: None,
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn focus_handle(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }

    pub fn content_focus_handle(mut self, focus_handle: &FocusHandle) -> Self {
        self.content_focus_handle = Some(focus_handle.clone());
        self
    }

    pub fn on_open_change(
        mut self,
        handler: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(handler));
        self
    }

    /// Handles the Confirm action while the combobox is open.
    pub fn on_confirm(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_confirm = Some(Rc::new(handler));
        self
    }

    /// Handles a dismissal requested through the Cancel action.
    ///
    /// This runs before the controlled open state is asked to close, so a
    /// combobox that commits its pending value on dismissal can still read
    /// that value here.
    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }
}

impl Styled for Combobox {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Combobox {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Combobox {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Select::new(self.id)
            .open(self.open)
            .disabled(self.disabled)
            .key_context(CONTEXT)
            .when_some(self.focus_handle, |this, handle| this.focus_handle(&handle))
            .when_some(self.content_focus_handle, |this, handle| {
                this.content_focus_handle(&handle)
            })
            .when_some(self.on_open_change, |this, handler| {
                this.on_open_change(move |open, window, cx| handler(open, window, cx))
            })
            .when_some(self.on_confirm, |this, handler| {
                this.on_confirm(move |window, cx| handler(window, cx))
            })
            .when_some(self.on_dismiss, |this, handler| {
                this.on_dismiss(move |window, cx| handler(window, cx))
            })
            .children(self.children)
            .refine_style(&self.style)
    }
}
