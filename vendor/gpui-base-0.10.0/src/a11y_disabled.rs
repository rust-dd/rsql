//! Disabled semantics on a control's accessibility node.
//!
//! GPUI has no `aria_disabled` setter, so a disabled control would reach
//! assistive technology as one that accepts input. [`A11yDisabledExt`] wraps the
//! control's root element without adding a layout node, a focus stop or an
//! accessibility node, and only marks the node it writes disabled.
use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, InteractiveElement,
    Interactivity, IntoElement, LayoutId, Pixels, Window, accesskit,
};

/// A control's root element whose accessibility node may be marked disabled.
pub(crate) struct A11yDisabled<E> {
    inner: E,
    disabled: bool,
}

impl<E> A11yDisabled<E> {
    pub(crate) fn new(inner: E, disabled: bool) -> Self {
        Self { inner, disabled }
    }
}

/// Marks a control's accessibility node disabled while `disabled` is true.
///
/// Call it at the end of the control's builder chain. With `test-support`, the
/// wrapper goes inside the observation, so its snapshot reports the flag too.
pub(crate) trait A11yDisabledExt: Sized {
    type Output: IntoElement;

    fn a11y_disabled(self, disabled: bool) -> Self::Output;
}

#[cfg(not(feature = "test-support"))]
impl<E: Element + InteractiveElement> A11yDisabledExt for E {
    type Output = A11yDisabled<E>;

    fn a11y_disabled(self, disabled: bool) -> Self::Output {
        A11yDisabled::new(self, disabled)
    }
}

#[cfg(feature = "test-support")]
impl<E: Element + InteractiveElement> A11yDisabledExt for gpui::Stateful<E> {
    type Output = A11yDisabled<Self>;

    fn a11y_disabled(self, disabled: bool) -> Self::Output {
        A11yDisabled::new(self, disabled)
    }
}

#[cfg(feature = "test-support")]
impl<E: Element<PrepaintState = Option<gpui::Hitbox>> + InteractiveElement> A11yDisabledExt
    for crate::test_support::Observed<E>
{
    type Output = crate::test_support::Observed<A11yDisabled<E>>;

    fn a11y_disabled(self, disabled: bool) -> Self::Output {
        self.map_inner(|inner| A11yDisabled::new(inner, disabled))
    }
}

#[cfg(not(feature = "gpui-fast"))]
impl<E: Element> IntoElement for A11yDisabled<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: InteractiveElement> InteractiveElement for A11yDisabled<E> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.inner.interactivity()
    }
}

impl<E: Element> Element for A11yDisabled<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.inner.id()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.inner.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.inner
            .prepaint(id, inspector, bounds, layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .paint(id, inspector, bounds, layout, prepaint, window, cx)
    }

    fn a11y_role(&self) -> Option<gpui::Role> {
        self.inner.a11y_role()
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        self.inner.write_a11y_info(node);
        if self.disabled {
            node.set_disabled();
        }
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut gpui::A11ySubtreeBuilder,
    ) {
        self.inner.a11y_synthetic_children(prepaint, builder);
    }
}
