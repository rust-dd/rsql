use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FocusHandle, InteractiveElement, Interactivity,
    IntoElement, MouseButton, ParentElement, Refineable as _, RenderOnce, Role, SharedString,
    Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _, relative,
};
use smallvec::SmallVec;

use crate::{
    RoleOverride, Selectable, StateStyle, StyledExt as _, TestSupportExt as _,
    a11y_disabled::A11yDisabledExt as _,
};

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// An unstyled button that owns interaction, focus, keyboard, and accessibility behavior.
///
/// Layout and visual states are intentionally supplied by the application through
/// GPUI's [`Styled`] API.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    base: crate::ObservedElement<Stateful<Div>>,
    style: StyleRefinement,
    semantic_styles: ButtonStyles,
    selected: bool,
    disabled: bool,
    hover_style: Option<StyleRefinement>,
    active_style: Option<StyleRefinement>,
    group_hover_style: Option<(SharedString, StyleRefinement)>,
    group_active_style: Option<(SharedString, StyleRefinement)>,
    children: SmallVec<[AnyElement; 2]>,
    on_click: Option<ClickHandler>,
    accessibility_label: Option<SharedString>,
    role: RoleOverride,
    provided_focus_handle: Option<FocusHandle>,
    tab_index: isize,
    tab_stop: bool,
    focusable: bool,
}

impl Button {
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()).test_support(),
            id,
            style: StyleRefinement::default(),
            semantic_styles: ButtonStyles::default(),
            selected: false,
            disabled: false,
            hover_style: None,
            active_style: None,
            group_hover_style: None,
            group_active_style: None,
            children: SmallVec::new(),
            on_click: None,
            accessibility_label: None,
            role: RoleOverride::Implicit,
            provided_focus_handle: None,
            tab_index: 0,
            tab_stop: true,
            focusable: true,
        }
    }

    /// Sets whether the button ignores pointer and keyboard activation.
    ///
    /// Disabled buttons are skipped during Tab traversal. If this button already
    /// owns focus when it becomes disabled, it retains the focus binding until
    /// focus moves away, so keyboard traversal can continue in either direction.
    ///
    /// While disabled, the `hover`, `active`, `group_hover` and `group_active`
    /// styles are not applied; use [`ButtonStyles::disabled`] for its look.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Whether the button is disabled.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Sets the application-controlled selected presentation state.
    ///
    /// This supports persistent trigger presentation while an associated menu
    /// or popover is open. It is distinct from momentary `active`, Toggle's
    /// `pressed` state, and accessibility toggle metadata; selecting a Button
    /// does not automatically set `aria_toggled`.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Defines application-owned styles for the button's semantic states.
    pub fn styles(mut self, build: impl FnOnce(ButtonStyles) -> ButtonStyles) -> Self {
        self.semantic_styles = build(self.semantic_styles);
        self
    }

    /// Sets the label exposed to accessibility clients.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    /// Overrides the accessibility role. The default is [`Role::Button`].
    pub fn role(mut self, role: impl Into<RoleOverride>) -> Self {
        self.role = role.into();
        self
    }

    /// Uses a caller-owned focus handle instead of creating keyed state.
    pub fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.provided_focus_handle = Some(focus_handle.clone());
        self
    }

    /// Sets the activation handler for pointer, Enter, and Space input.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Sets the focus traversal index. The default is `0`.
    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = tab_index;
        self
    }

    /// Sets whether the button participates in keyboard focus traversal.
    pub fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.tab_stop = tab_stop;
        self
    }

    /// Sets whether pressing the button moves focus onto it. The default is `true`.
    ///
    /// A non-focusable button leaves focus wherever it already is, which keeps a
    /// composed control from flickering its focus ring on every press. It also
    /// gives up Enter and Space activation, so only use it for a button that a
    /// focused sibling already exposes to the keyboard, such as a number input's
    /// step buttons.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    fn focus_handle(&self, window: &mut Window, cx: &mut App) -> FocusHandle {
        self.provided_focus_handle.clone().unwrap_or_else(|| {
            window
                .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
                .read(cx)
                .clone()
        })
    }

    fn resolved_style(&self) -> StyleRefinement {
        crate::state_style::resolve_style(
            &self.style,
            [
                self.selected.then_some(&self.semantic_styles.selected),
                self.disabled.then_some(&self.semantic_styles.disabled),
            ]
            .into_iter()
            .flatten(),
        )
    }
}

impl Selectable for Button {
    fn selected(self, selected: bool) -> Self {
        Button::selected(self, selected)
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

/// Semantic styles supported by [`Button`].
#[derive(Default)]
pub struct ButtonStyles {
    selected: StyleRefinement,
    disabled: StyleRefinement,
}

impl ButtonStyles {
    /// Refines the root style when the button is selected.
    pub fn selected(mut self, build: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.selected
            .refine(&build(StateStyle::default()).into_refinement());
        self
    }

    /// Refines the root style when the button is disabled.
    pub fn disabled(mut self, build: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.disabled
            .refine(&build(StateStyle::default()).into_refinement());
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn hover(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.hover_style = Some(f(StyleRefinement::default()));
        self
    }

    fn group_hover(
        mut self,
        group_name: impl Into<SharedString>,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self.group_hover_style = Some((group_name.into(), f(StyleRefinement::default())));
        self
    }
}

impl StatefulInteractiveElement for Button {
    fn active(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.active_style = Some(f(StyleRefinement::default()));
        self
    }

    fn group_active(
        mut self,
        group_name: impl Into<SharedString>,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self.group_active_style = Some((group_name.into(), f(StyleRefinement::default())));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus_handle = self.focus_handle(window, cx);
        let disabled = self.disabled;
        let style = self.resolved_style();
        let on_click = self.on_click;
        let pointer_style = move |style: StyleRefinement| {
            if disabled {
                StyleRefinement::default()
            } else {
                style
            }
        };

        self.base
            // Centering is part of Button's control geometry. Without a flex
            // formatting context an ordinary child starts at the root's
            // leading edge, so a fixed-height unstyled Button cannot align its
            // label even though its neutral line height is correct.
            .flex()
            .items_center()
            .justify_center()
            // A neutral line height is geometry, not visual policy: with the
            // inherited value the text box is taller than the glyphs, so the
            // caller's padding no longer determines the control's height and a
            // button cannot be sized precisely. `relative(1.)` makes the text
            // box exactly the font size; anything the caller sets refines over
            // it, so a product that wants looser text still can.
            .line_height(relative(1.))
            .when_some(self.role.resolve(|| Role::Button), |this, role| {
                this.role(role)
            })
            .when_some(self.accessibility_label, |this, label| {
                this.aria_label(label)
            })
            // Keep the current focus in the dispatch tree when activation
            // disables this button, so Tab can still leave it in either direction.
            .when(
                self.focusable && (!disabled || focus_handle.is_focused(window)),
                |this| {
                    this.track_focus(
                        &focus_handle
                            .tab_index(self.tab_index)
                            .tab_stop(self.tab_stop && !disabled),
                    )
                },
            )
            .when(disabled, |this| {
                this.on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
            })
            .when_some(
                (!disabled).then_some(on_click).flatten(),
                |this, on_click| {
                    this.on_click(move |event, window, cx| {
                        on_click(event, window, cx);
                    })
                },
            )
            .when_some(self.hover_style, |this, style| {
                this.hover(|_| pointer_style(style))
            })
            .when_some(self.active_style, |this, style| {
                this.active(|_| pointer_style(style))
            })
            .when_some(self.group_hover_style, |this, (group, style)| {
                this.group_hover(group, |_| pointer_style(style))
            })
            .when_some(self.group_active_style, |this, (group, style)| {
                this.group_active(group, |_| pointer_style(style))
            })
            .children(self.children)
            .refine_style(&style)
            .a11y_disabled(disabled)
    }
}
