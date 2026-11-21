use crate::TestSupportExt as _;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, InteractiveElement, Interactivity, IntoElement,
    ParentElement, RenderOnce, Role, StatefulInteractiveElement, StyleRefinement, Styled, Window,
    div, prelude::FluentBuilder as _,
};
use smallvec::SmallVec;

use crate::StyledExt as _;

type ChangeHandler = Rc<dyn Fn(bool, &ClickEvent, &mut Window, &mut App)>;

/// An unstyled accordion root for application-owned items.
#[derive(IntoElement)]
pub struct Accordion {
    base: crate::ObservedElement<gpui::Stateful<Div>>,
    style: StyleRefinement,
    children: SmallVec<[AnyElement; 2]>,
}

impl Accordion {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id).test_support(),
            style: StyleRefinement::default(),
            children: SmallVec::new(),
        }
    }
}

impl Styled for Accordion {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Accordion {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for Accordion {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(handle);
        self
    }
}

impl StatefulInteractiveElement for Accordion {}

impl RenderOnce for Accordion {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base
            .role(Role::Group)
            .children(self.children)
            .refine_style(&self.style)
    }
}

/// An unstyled accordion item connecting one trigger with its panel content.
#[derive(IntoElement)]
pub struct AccordionItem {
    base: Div,
    style: StyleRefinement,
    open: bool,
    disabled: bool,
    header: Option<AccordionHeader>,
    panel: Option<AccordionPanel>,
    children: SmallVec<[AnyElement; 2]>,
}

impl AccordionItem {
    pub fn new() -> Self {
        Self {
            base: div(),
            style: StyleRefinement::default(),
            open: false,
            disabled: false,
            header: None,
            panel: None,
            children: SmallVec::new(),
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

    pub fn header(mut self, header: AccordionHeader) -> Self {
        self.header = Some(header);
        self
    }

    pub fn panel(mut self, panel: AccordionPanel) -> Self {
        self.panel = Some(panel);
        self
    }
}

impl Styled for AccordionItem {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for AccordionItem {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for AccordionItem {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base
            .when_some(self.header, |this, header| {
                this.child(header.open(self.open).disabled(self.disabled))
            })
            .when_some(self.panel, |this, panel| this.child(panel.open(self.open)))
            .children(self.children)
            .refine_style(&self.style)
    }
}

/// An unstyled heading that owns the trigger for one accordion item.
#[derive(IntoElement)]
pub struct AccordionHeader {
    id: Option<ElementId>,
    style: StyleRefinement,
    level: usize,
    trigger: AccordionTrigger,
    children: SmallVec<[AnyElement; 1]>,
}

impl AccordionHeader {
    pub fn new(trigger: AccordionTrigger) -> Self {
        Self {
            id: None,
            style: StyleRefinement::default(),
            level: 3,
            trigger,
            children: SmallVec::new(),
        }
    }
    pub fn level(mut self, level: usize) -> Self {
        self.level = level;
        self
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
    fn open(mut self, open: bool) -> Self {
        self.trigger = self.trigger.open(open);
        self
    }

    fn disabled(mut self, disabled: bool) -> Self {
        self.trigger = self.trigger.disabled(disabled);
        self
    }
}

impl Styled for AccordionHeader {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for AccordionHeader {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for AccordionHeader {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let content = div()
            .child(self.trigger)
            .children(self.children)
            .refine_style(&self.style);
        match self.id {
            Some(id) => content
                .id(id)
                .test_support()
                .role(Role::Heading)
                .aria_level(self.level)
                .into_any_element(),
            None => content.into_any_element(),
        }
    }
}

/// An unstyled accordion panel with controlled mounting.
#[derive(IntoElement)]
pub struct AccordionPanel {
    id: Option<ElementId>,
    style: StyleRefinement,
    open: bool,
    keep_mounted: bool,
    children: SmallVec<[AnyElement; 2]>,
}

impl AccordionPanel {
    pub fn new() -> Self {
        Self {
            id: None,
            style: StyleRefinement::default(),
            open: false,
            keep_mounted: false,
            children: SmallVec::new(),
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn keep_mounted(mut self, keep_mounted: bool) -> Self {
        self.keep_mounted = keep_mounted;
        self
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl Styled for AccordionPanel {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for AccordionPanel {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for AccordionPanel {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        if !self.open && !self.keep_mounted {
            return gpui::Empty.into_any_element();
        }

        let content = div().children(self.children).refine_style(&self.style);
        match self.id {
            Some(id) => content
                .id(id)
                .test_support()
                .role(Role::Region)
                .into_any_element(),
            None => content.into_any_element(),
        }
    }
}

/// An unstyled accordion trigger with controlled expanded state.
///
/// The application owns the title, disclosure icon, content, layout, and
/// animation. Activating the trigger requests the opposite of `open`.
#[derive(IntoElement)]
pub struct AccordionTrigger {
    base: crate::ObservedElement<gpui::Stateful<Div>>,
    style: StyleRefinement,
    children: SmallVec<[AnyElement; 2]>,
    open: bool,
    disabled: bool,
    on_change: Option<ChangeHandler>,
}

impl AccordionTrigger {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id).test_support(),
            style: StyleRefinement::default(),
            children: SmallVec::new(),
            open: false,
            disabled: false,
            on_change: None,
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

    /// Handles a requested change to the controlled expanded state.
    pub fn on_change(
        mut self,
        handler: impl Fn(bool, &ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl Styled for AccordionTrigger {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for AccordionTrigger {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for AccordionTrigger {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(handle);
        self
    }
}

impl StatefulInteractiveElement for AccordionTrigger {}

impl RenderOnce for AccordionTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let next_open = !self.open;

        self.base
            .role(Role::Button)
            .aria_expanded(self.open)
            .when_some(
                (!self.disabled).then_some(self.on_change).flatten(),
                move |this, on_change| {
                    this.on_click(move |event, window, cx| on_change(next_open, event, window, cx))
                },
            )
            .children(self.children)
            .refine_style(&self.style)
    }
}
