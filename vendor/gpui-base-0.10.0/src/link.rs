use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FocusHandle, InteractiveElement, Interactivity,
    IntoElement, MouseButton, ParentElement, Refineable as _, RenderOnce, Role, SharedString,
    Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _,
};
use smallvec::SmallVec;

use crate::{StateStyle, StyledExt as _, a11y_disabled::A11yDisabledExt as _};

type ActivationHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type OpenHandler = Rc<dyn Fn(&str, &ClickEvent, &mut Window, &mut App)>;

/// An unstyled link that owns focus, activation, and accessibility semantics.
///
/// `href` is target data, not an instruction to launch a browser. Applications
/// inject navigation through [`Link::open_with`], allowing internal routes,
/// embedded web views, and external browsers to share the same behavior module.
/// `href` is also not rendered as text; applications provide visible content
/// through the element's child slot.
#[derive(IntoElement)]
pub struct Link {
    id: ElementId,
    base: Stateful<Div>,
    style: StyleRefinement,
    semantic_styles: LinkStyles,
    href: Option<SharedString>,
    disabled: bool,
    children: SmallVec<[AnyElement; 2]>,
    on_activate: Option<ActivationHandler>,
    open_with: Option<OpenHandler>,
    accessibility_label: Option<SharedString>,
    tab_index: isize,
    tab_stop: bool,
}

/// Semantic root styles supported by [`Link`].
#[derive(Default)]
pub struct LinkStyles {
    disabled: StyleRefinement,
}

impl LinkStyles {
    pub fn disabled(mut self, build: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.disabled
            .refine(&build(StateStyle::default()).into_refinement());
        self
    }
}

impl Link {
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()),
            id,
            style: StyleRefinement::default(),
            semantic_styles: LinkStyles::default(),
            href: None,
            disabled: false,
            children: SmallVec::new(),
            on_activate: None,
            open_with: None,
            accessibility_label: None,
            tab_index: 0,
            tab_stop: true,
        }
    }

    /// Sets the application-defined navigation target.
    pub fn href(mut self, href: impl Into<SharedString>) -> Self {
        self.href = Some(href.into());
        self
    }

    /// Injects the strategy used to open `href` on activation.
    ///
    /// Base never calls [`App::open_url`] on its own.
    pub fn open_with(
        mut self,
        open: impl Fn(&str, &ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.open_with = Some(Rc::new(open));
        self
    }

    /// Observes pointer or keyboard activation after the open strategy runs.
    pub fn on_activate(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_activate = Some(Rc::new(handler));
        self
    }

    /// Sets whether pointer and keyboard activation are ignored.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Configures application-owned styles for the link's semantic states.
    pub fn styles(mut self, build: impl FnOnce(LinkStyles) -> LinkStyles) -> Self {
        self.semantic_styles = build(self.semantic_styles);
        self
    }

    fn resolved_style(&self) -> StyleRefinement {
        crate::state_style::resolve_style(
            &self.style,
            self.disabled.then_some(&self.semantic_styles.disabled),
        )
    }

    /// Sets the name exposed to accessibility clients.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    /// Sets the focus traversal index. Use this within a GPUI tab group.
    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = tab_index;
        self
    }

    /// Sets whether the link participates in keyboard focus traversal.
    pub fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.tab_stop = tab_stop;
        self
    }

    fn focus_handle(&self, window: &mut Window, cx: &mut App) -> FocusHandle {
        window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone()
    }
}

impl Styled for Link {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Link {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for Link {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Link {}

impl RenderOnce for Link {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus_handle = self.focus_handle(window, cx);
        let disabled = self.disabled;
        let style = self.resolved_style();
        let href = self.href;
        let open_with = self.open_with;
        let on_activate = self.on_activate;
        let activates = on_activate.is_some() || href.is_some() && open_with.is_some();

        self.base
            .role(Role::Link)
            .when_some(self.accessibility_label, |this, label| {
                this.aria_label(label)
            })
            .when(!disabled, |this| {
                this.track_focus(
                    &focus_handle
                        .tab_index(self.tab_index)
                        .tab_stop(self.tab_stop),
                )
            })
            .when(disabled, |this| {
                this.on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
            })
            .when(!disabled && activates, |this| {
                this.on_click(move |event, window, cx| {
                    if let (Some(href), Some(open)) = (&href, &open_with) {
                        open(href.as_ref(), event, window, cx);
                    }
                    if let Some(on_activate) = &on_activate {
                        on_activate(event, window, cx);
                    }
                })
            })
            .children(self.children)
            .refine_style(&style)
            .a11y_disabled(disabled)
    }
}
