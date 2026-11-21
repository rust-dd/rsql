use gpui::{
    AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, StyleRefinement, Styled,
    Window,
};
use gpui_base::spring;

use crate::{ActiveTheme as _, StyledExt};

/// An interactive element which expands/collapses.
#[derive(IntoElement)]
pub struct Collapsible {
    base: gpui_base::Collapsible,
    style: StyleRefinement,
    motion_id: Option<ElementId>,
    open: bool,
}

impl Collapsible {
    /// Creates a new `Collapsible` instance.
    pub fn new() -> Self {
        Self {
            base: gpui_base::Collapsible::new(),
            style: StyleRefinement::default(),
            motion_id: None,
            open: false,
        }
    }

    /// Sets whether the collapsible is open. default is false.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self.base = self.base.open(open);
        self
    }

    /// Enables a reversible measured reveal under a stable identity.
    pub fn motion_id(mut self, id: impl Into<ElementId>) -> Self {
        self.motion_id = Some(id.into());
        self
    }

    /// Sets the content of the collapsible.
    ///
    /// If `open` is false, content will be hidden.
    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.base = self.base.content(content);
        self
    }
}

impl Styled for Collapsible {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Collapsible {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for Collapsible {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let base = match self.motion_id {
            Some(id) => {
                let progress = spring(
                    (id.clone(), "reveal"),
                    if self.open { 1.0 } else { 0.0 },
                    cx.theme().motion_tokens().spring_control,
                    window,
                    cx,
                );
                self.base.reveal(id, progress)
            }
            None => self.base,
        };
        base.v_flex().refine_style(&self.style)
    }
}
