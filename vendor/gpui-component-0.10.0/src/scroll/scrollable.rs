use std::{panic::Location, rc::Rc};

use crate::{InteractiveElementExt as _, StyledExt};

use super::{ScrollableMask, Scrollbar, ScrollbarAxis, ScrollbarHandle};
use gpui::{
    App, Div, Element, ElementId, InteractiveElement, IntoElement, Overflow, ParentElement,
    PointRefinement, RenderOnce, ScrollHandle, Stateful, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, div, prelude::FluentBuilder,
};

/// A trait for elements that can be made scrollable with scrollbars.
///
/// The wrapped element is the scroll area itself, rather than being inserted as
/// a child of a new scroll area.
pub trait ScrollableElement: InteractiveElement + Styled + ParentElement + Element {
    /// Adds a scrollbar to the element.
    #[track_caller]
    fn scrollbar<H: ScrollbarHandle + Clone>(
        self,
        scroll_handle: &H,
        axis: impl Into<ScrollbarAxis>,
    ) -> Self {
        self.child(ScrollbarLayer {
            id: caller_id(),
            axis: axis.into(),
            scroll_handle: Rc::new(scroll_handle.clone()),
        })
    }

    /// Adds a vertical scrollbar to the element.
    #[track_caller]
    fn vertical_scrollbar<H: ScrollbarHandle + Clone>(self, scroll_handle: &H) -> Self {
        self.scrollbar(scroll_handle, ScrollbarAxis::Vertical)
    }

    /// Adds a horizontal scrollbar to the element.
    #[track_caller]
    fn horizontal_scrollbar<H: ScrollbarHandle + Clone>(self, scroll_handle: &H) -> Self {
        self.scrollbar(scroll_handle, ScrollbarAxis::Horizontal)
    }

    /// Almost equivalent to [`StatefulInteractiveElement::overflow_scroll`], but adds scrollbars.
    /// Preserves the source element as the scrollable container.
    #[track_caller]
    fn overflow_scrollbar(self) -> Scrollable<Self> {
        Scrollable::new(self, ScrollbarAxis::Both)
    }

    /// Almost equivalent to [`StatefulInteractiveElement::overflow_x_scroll`], but adds Horizontal scrollbar.
    /// Preserves the source element as the scrollable container.
    #[track_caller]
    fn overflow_x_scrollbar(self) -> Scrollable<Self> {
        Scrollable::new(self, ScrollbarAxis::Horizontal)
    }

    /// Almost equivalent to [`StatefulInteractiveElement::overflow_y_scroll`], but adds Vertical scrollbar.
    /// Preserves the source element as the scrollable container.
    #[track_caller]
    fn overflow_y_scrollbar(self) -> Scrollable<Self> {
        Scrollable::new(self, ScrollbarAxis::Vertical)
    }
}

/// A scrollable element wrapper that renders the original element as the scroll area and overlays scrollbars.
#[derive(IntoElement)]
pub struct Scrollable<E: InteractiveElement + Styled + ParentElement + Element> {
    id: ElementId,
    element: E,
    axis: ScrollbarAxis,
    scroll_bounce: Option<bool>,
}

impl<E> Scrollable<E>
where
    E: InteractiveElement + Styled + ParentElement + Element,
{
    #[track_caller]
    fn new(element: E, axis: impl Into<ScrollbarAxis>) -> Self {
        Self {
            id: caller_id(),
            element,
            axis: axis.into(),
            scroll_bounce: None,
        }
    }

    /// Set a specific element id, default is the [`std::panic::Location::caller`].
    ///
    /// Only needed when one call site creates several scrollables, which would
    /// otherwise share a single scroll position.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    /// Bounce at the vertical boundaries. On by default on iOS, Android and
    /// macOS, off elsewhere; horizontal-only areas do not bounce.
    pub fn scroll_bounce(mut self, enabled: bool) -> Self {
        self.scroll_bounce = Some(enabled);
        self
    }
}

impl<E> Styled for Scrollable<E>
where
    E: InteractiveElement + Styled + ParentElement + Element,
{
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl<E> ParentElement for Scrollable<E>
where
    E: InteractiveElement + Styled + ParentElement + Element,
{
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.element.extend(elements)
    }
}

impl<E> InteractiveElement for Scrollable<E>
where
    E: InteractiveElement + Styled + ParentElement + Element,
{
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.element.interactivity()
    }

    fn track_focus(mut self, focus: &gpui::FocusHandle) -> Self {
        self.element = self.element.track_focus(focus);
        self
    }
}

impl<E> RenderOnce for Scrollable<E>
where
    E: InteractiveElement + Styled + ParentElement + Element + 'static,
{
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let scroll_handle = scroll_handle_for(&self.id, window, cx);

        // Preserve the caller-requested size on the wrapper, while keeping the
        // caller's element as the actual scroll-tracked layout container.
        let root_style = root_style_from(&mut self.element, self.axis);

        let root_id = self.id.clone();
        let area_id = (self.id.clone(), "area");
        let content_id = (self.id.clone(), "content");
        let scrollbar_id = (self.id.clone(), "scrollbar");

        let mut content = self
            .element
            .id(content_id)
            .flex_none()
            .map(|this| match self.axis {
                ScrollbarAxis::Vertical => this.h_auto().min_h_full(),
                ScrollbarAxis::Horizontal => this.w_auto().min_w_full(),
                ScrollbarAxis::Both => this.size_auto().min_size_full(),
            });
        // The wrapper holds the caller's maximum size. Left on the content
        // too, it would cap the content at the area's size on the scrolled
        // axis, so nothing would overflow and the area would not scroll.
        let max_size = &mut content.style().max_size;
        match self.axis {
            ScrollbarAxis::Vertical => max_size.height = None,
            ScrollbarAxis::Horizontal => max_size.width = None,
            ScrollbarAxis::Both => *max_size = Default::default(),
        }

        // Keep the scroll area in the normal flow: its content size must
        // propagate to auto-sized ancestors (e.g. a Dialog that grows with
        // its content). An absolutely positioned scroll area would collapse
        // such ancestors to zero height.
        let scroll_area = div()
            .id(area_id)
            .size_full()
            .flex()
            .track_scroll(&scroll_handle)
            .map(|this| match self.axis {
                ScrollbarAxis::Vertical => this.flex_col().overflow_y_scroll(),
                ScrollbarAxis::Horizontal => this.flex_row().overflow_x_scroll(),
                ScrollbarAxis::Both => this.overflow_scroll(),
            })
            // On a single-axis area gpui otherwise remaps the other axis' delta
            // onto ours, so a purely horizontal swipe scrolls this vertically.
            .lock_scroll_axis()
            .child(content);

        // The wrapper lays the area out as a flex item along the scrolled
        // axis, so the area shrinks to the wrapper's size. Sized by its 100%
        // alone, the area is not held to a maximum size on the wrapper with
        // GPUI Fast: in a column whose height follows its content, inside a
        // row, the area came out as tall as that column and the wrapper
        // clipped the rows past its maximum height instead of scrolling to
        // them.
        div()
            .id(root_id)
            .size_full()
            .refine_style(&root_style)
            .relative()
            .flex()
            .map(|this| match self.axis {
                ScrollbarAxis::Horizontal => this.flex_row(),
                ScrollbarAxis::Vertical | ScrollbarAxis::Both => this.flex_col(),
            })
            .child(
                gpui_base::ScrollBounce::new(
                    (self.id.clone(), "bounce"),
                    &scroll_handle,
                    scroll_area,
                )
                .when_some(self.scroll_bounce, |bounce, on| bounce.enabled(on)),
            )
            // A plain mouse wheel has only a vertical axis, which the locked
            // horizontal area above would ignore.
            .when(self.axis == ScrollbarAxis::Horizontal, |this| {
                this.child(ScrollableMask::vertical_wheel(&scroll_handle))
            })
            .child(render_scrollbar(
                scrollbar_id,
                &scroll_handle,
                self.axis,
                window,
                cx,
            ))
    }
}

impl ScrollableElement for Div {}
impl<E> ScrollableElement for Stateful<E>
where
    E: ParentElement + Styled + Element,
    Self: InteractiveElement,
{
}

#[derive(IntoElement)]
struct ScrollbarLayer<H: ScrollbarHandle + Clone> {
    id: ElementId,
    axis: ScrollbarAxis,
    scroll_handle: Rc<H>,
}

impl<H> RenderOnce for ScrollbarLayer<H>
where
    H: ScrollbarHandle + Clone + 'static,
{
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        render_scrollbar(self.id, self.scroll_handle.as_ref(), self.axis, window, cx)
    }
}

#[inline]
#[track_caller]
fn caller_id() -> ElementId {
    ElementId::CodeLocation(*Location::caller())
}

#[inline]
fn scroll_handle_for(id: &ElementId, window: &mut Window, cx: &mut App) -> ScrollHandle {
    window
        .use_keyed_state(id.clone(), cx, |_, _| ScrollHandle::default())
        .read(cx)
        .clone()
}

/// Copies the outer layout styles from the element, so the wrapper can
/// participate in the parent's layout the same way the source element would.
///
/// A flex item only drops its content-based automatic minimum size when its own
/// overflow is not [`Overflow::Visible`]; otherwise the item refuses to shrink
/// around its content. The scrolled overflow lives on the inner scroll area, so
/// the wrapper has to declare the scrolled axis clipped itself — without it a
/// scroll region used as a flex item pushes its siblings out of the container
/// instead of scrolling, and every call site has to remember `min_h_0()` /
/// `min_w_0()`.
///
/// [`Overflow::Hidden`] rather than a zero minimum: it is the axis' real
/// behavior, it leaves an explicit `min_size` from the caller in charge, and it
/// keeps the region's content size contributing to an auto-sized ancestor such
/// as a Dialog that grows with its body. The mask it installs matches the inner
/// scroll area's own mask, so nothing new is clipped.
#[inline]
fn root_style_from<E>(element: &mut E, axis: ScrollbarAxis) -> StyleRefinement
where
    E: Styled,
{
    let style = element.style();
    StyleRefinement {
        size: style.size.clone(),
        min_size: style.min_size.clone(),
        max_size: style.max_size.clone(),
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        flex_basis: style.flex_basis,
        align_self: style.align_self,
        overflow: PointRefinement {
            x: axis.has_horizontal().then_some(Overflow::Hidden),
            y: axis.has_vertical().then_some(Overflow::Hidden),
        },
        ..Default::default()
    }
}

#[inline]
fn render_scrollbar<H: ScrollbarHandle + Clone>(
    id: impl Into<ElementId>,
    scroll_handle: &H,
    axis: ScrollbarAxis,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    // Do not render scrollbar when inspector is picking elements,
    // to allow us to pick the background elements.
    let is_inspector_picking = window.is_inspector_picking(cx);
    if is_inspector_picking {
        return div();
    }

    div()
        .absolute()
        .inset_0()
        .debug_selector(|| "scrollbar-overlay".to_string())
        .child(
            Scrollbar::new(scroll_handle)
                .id(id)
                .axis(axis)
                .viewport_from_layout(),
        )
}

#[cfg(feature = "test-support")]
impl<E> ScrollableElement for gpui_base::test_support::Observed<E> where
    E: ScrollableElement + Element<PrepaintState = Option<gpui::Hitbox>>
{
}
