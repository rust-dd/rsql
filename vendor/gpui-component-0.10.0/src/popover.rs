use gpui::{
    Anchor, Animation, AnimationExt as _, AnyElement, App, Bounds, Context, Div, ElementId,
    FocusHandle, InteractiveElement as _, IntoElement, MouseButton, ParentElement, PathBuilder,
    Pixels, Point, RenderOnce, Stateful, StyleRefinement, Styled, Window, canvas, point,
    prelude::FluentBuilder as _, px,
};
use std::{cell::Cell, rc::Rc, time::Duration};

use crate::{ActiveTheme as _, ThemeStyled as _};
use crate::{
    Selectable, StyledExt as _,
    animation::ease_out_cubic,
    styled::{popover_ring, popover_shadow},
    v_flex,
};
use gpui_base::Placement;
use gpui_base::Popover as BasePopover;
pub use gpui_base::PopoverState;

pub(crate) fn init(_: &mut App) {}

/// How long a dropdown takes to settle into place after it opens.
///
/// This is shadcn/ui's figure: its popup surfaces carry `animate-in`, whose
/// duration is 150ms.
const DROPDOWN_ENTER_DURATION: Duration = Duration::from_millis(150);

/// Where a dropdown starts out, relative to where it comes to rest.
///
/// Negative is above, so the surface slides *down* out of the trigger's edge —
/// what shadcn/ui expresses as `data-[side=bottom]:slide-in-from-top-2`. Its
/// `2` is `0.5rem`, which is 8px at the default root size.
const DROPDOWN_ENTER_OFFSET: Pixels = px(-8.);

fn dropdown_positioner(bounds: Bounds<Pixels>) -> gpui_base::Positioner {
    gpui_base::Positioner::side(bounds)
        .placement(gpui_base::Placement::Bottom)
        .align(gpui_base::Align::Start)
        .offset(px(6.))
        .margin(px(8.))
}

/// Positions a dropdown surface under its trigger and animates it in.
///
/// This is the shared open motion for Select, Combobox and DatePicker, modelled
/// on shadcn/ui: over 150ms the surface fades up from nothing while sliding the
/// last 8px out of the trigger's edge, on an ease-out curve so it decelerates
/// into place.
///
/// `surface` must be the panel itself — the element carrying
/// [`ThemeStyled::popover_style`] — and not a wrapper around it. GPUI takes a
/// shadow's shape from the element it is set on, so a wrapper of a different
/// size would throw the shadow out of register with the panel.
///
/// # Why the shadow is animated too
///
/// GPUI has no group compositing: `opacity` multiplies into each primitive's
/// alpha separately rather than fading a composited subtree. A drop shadow is
/// painted as a full blurred rect *under* the element — the shader only cuts the
/// element out of `inset` shadows — so a translucent panel does not hide its own
/// shadow, and mid-fade the shadow shows straight through the panel as a dark
/// slab. Ramping the ink by the cube of the fade keeps it out of sight until the
/// panel is opaque enough to cover it, and still lands on the resting shadow
/// [`popover_shadow`] gives every other popup.
///
/// # Departures from shadcn
///
/// - shadcn also scales the surface up from 95% (`zoom-in-95`). GPUI has no
///   element transform — only images and SVGs take a `TransformationMatrix` —
///   so there is nothing to scale a subtree with, and the fade and slide carry
///   the motion on their own.
/// - There is no exit motion. A closing dropdown stops being rendered in the
///   same frame its state flips, so playing one would mean keeping the surface
///   mounted past the close, which is a change to how each of these components
///   tracks `open`.
/// - The slide always comes from above. [`gpui_base::Positioner`] resolves the
///   side the surface actually lands on during layout and does not report it
///   back, so a dropdown that flips above its trigger for want of room below
///   slides the opposite way — 8px over 150ms, in the rare case where it
///   happens.
///
/// Reduced motion needs no handling here: GPUI's animation element adopts the
/// final value on the first frame when the system asks for it.
pub(crate) fn dropdown_popup(
    id: impl Into<ElementId>,
    bounds: Bounds<Pixels>,
    surface: impl IntoElement + Styled + 'static,
    cx: &App,
) -> gpui_base::Positioner {
    let travel: f32 = DROPDOWN_ENTER_OFFSET.into();
    // Read out here: the animation runs long after `cx` is gone.
    let ring = popover_ring(cx);

    dropdown_positioner(bounds).child(surface.with_animation(
        id,
        Animation::new(DROPDOWN_ENTER_DURATION).with_easing(ease_out_cubic),
        move |surface, delta| {
            surface
                .top(px(travel * (1. - delta)))
                .opacity(delta)
                .shadow(popover_shadow(ring, delta * delta * delta))
        },
    ))
}

/// A popover element that can be triggered by a button or any other element.
#[derive(IntoElement)]
pub struct Popover {
    id: ElementId,
    style: StyleRefinement,
    anchor: Anchor,
    offset: Option<Pixels>,
    arrow: bool,
    default_open: bool,
    open: Option<bool>,
    tracked_focus_handle: Option<FocusHandle>,
    trigger: Option<Box<dyn FnOnce(bool, &Window, &App) -> AnyElement + 'static>>,
    content: Option<
        Rc<
            dyn Fn(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> AnyElement
                + 'static,
        >,
    >,
    children: Vec<AnyElement>,
    /// Style for the trigger container, the element laid out in the parent
    /// and measured to anchor the popup.
    trigger_style: Option<StyleRefinement>,
    mouse_button: MouseButton,
    appearance: bool,
    overlay_closable: bool,
    on_open_change: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
}

impl Popover {
    /// Create a new Popover with `view` mode.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            anchor: Anchor::TopLeft,
            offset: None,
            arrow: false,
            trigger: None,
            trigger_style: None,
            content: None,
            tracked_focus_handle: None,
            children: vec![],
            mouse_button: MouseButton::Left,
            appearance: true,
            overlay_closable: true,
            default_open: false,
            open: None,
            on_open_change: None,
        }
    }

    /// Set the anchor corner of the popover, default is [`Anchor::TopLeft`].
    ///
    /// This names the popover's own anchor, not a corner of the trigger.
    /// `TopLeft` opens below the trigger, left-aligned; `BottomRight` opens
    /// above it, right-aligned. Legacy anchoring clamps without flipping.
    pub fn anchor(mut self, anchor: impl Into<Anchor>) -> Self {
        self.anchor = anchor.into();
        self
    }

    /// Gap from the trigger to the surface (or arrow tip), default 0.25rem.
    /// Preserves the anchor and does not enable automatic flipping.
    pub fn offset(mut self, offset: impl Into<Pixels>) -> Self {
        self.offset = Some(offset.into());
        self
    }

    /// Show an arrow pointing toward the trigger. Default is `false`.
    /// Follows the anchor, with its base inset to avoid rounded corners.
    /// Uses the surface background, falling back to the theme's popover color.
    pub fn arrow(mut self, arrow: bool) -> Self {
        self.arrow = arrow;
        self
    }

    /// Set the mouse button to trigger the popover, default is `MouseButton::Left`.
    pub fn mouse_button(mut self, mouse_button: MouseButton) -> Self {
        self.mouse_button = mouse_button;
        self
    }

    /// Set the trigger element of the popover.
    pub fn trigger<T>(mut self, trigger: T) -> Self
    where
        T: Selectable + IntoElement + 'static,
    {
        self.trigger = Some(Box::new(|is_open, _, _| {
            let open = trigger.is_open();
            trigger.open(open || is_open).into_any_element()
        }));
        self
    }

    /// Set the default open state of the popover, default is `false`.
    ///
    /// This is only used to initialize the open state of the popover.
    ///
    /// And please note that if you use the `open` method, this value will be ignored.
    pub fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    /// Force set the open state of the popover.
    ///
    /// If this is set, the popover will be controlled by this value.
    ///
    /// NOTE: You must be used in conjunction with `on_open_change` to handle state changes.
    pub fn open(mut self, open: bool) -> Self {
        self.open = Some(open);
        self
    }

    /// Add a callback to be called when the open state changes.
    ///
    /// The first `&bool` parameter is the **new open state**.
    ///
    /// This is useful when using the `open` method to control the popover state.
    pub fn on_open_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(&bool, &mut Window, &mut App) + 'static,
    {
        self.on_open_change = Some(Rc::new(callback));
        self
    }

    /// Set the style for the trigger container.
    ///
    /// The container is the element laid out in the parent and measured to
    /// anchor the popup, so this is where `w_full` or `flex_1` must go for the
    /// trigger to fill its slot.
    pub fn trigger_style(mut self, style: StyleRefinement) -> Self {
        self.trigger_style = Some(style);
        self
    }

    /// Set whether clicking outside the popover will dismiss it, default is `true`.
    pub fn overlay_closable(mut self, closable: bool) -> Self {
        self.overlay_closable = closable;
        self
    }

    /// Set the content builder for content of the Popover.
    ///
    /// This callback will called every time on render the popover.
    /// So, you should avoid creating new elements or entities in the content closure.
    pub fn content<F, E>(mut self, content: F) -> Self
    where
        E: IntoElement,
        F: Fn(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> E + 'static,
    {
        self.content = Some(Rc::new(move |state, window, cx| {
            content(state, window, cx).into_any_element()
        }));
        self
    }

    /// Set whether the popover no style, default is `false`.
    ///
    /// If no style:
    ///
    /// - The popover will not have a bg, border, shadow, or padding.
    /// - The click out of the popover will not dismiss it.
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }

    /// Bind the focus handle to receive focus when the popover is opened.
    /// If you not set this, a new focus handle will be created for the popover to
    ///
    /// If popover is opened, the focus will be moved to the focus handle.
    pub fn track_focus(mut self, handle: &FocusHandle) -> Self {
        self.tracked_focus_handle = Some(handle.clone());
        self
    }
}

impl ParentElement for Popover {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Popover {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Popover {
    pub(crate) fn render_popover_content(
        anchor: Anchor,
        appearance: bool,
        _: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        v_flex()
            .id("content")
            .occlude()
            .tab_group()
            .when(appearance, |this| this.popover_style(cx).p_3())
            .map(|this| match anchor {
                Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => this.top_1(),
                Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => this.bottom_1(),
                Anchor::LeftCenter | Anchor::RightCenter => this.top_1(), // Fallback for centered
            })
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // The arrow points at the physical corner the popup resolved to.
        let anchor = gpui_base::logical_anchor(self.anchor, gpui_base::LayoutDirection::global(cx));
        let arrow_size = if self.arrow {
            window.rem_size() * 0.375
        } else {
            px(0.)
        };
        let offset = self.offset.unwrap_or(window.rem_size() * 0.25) + arrow_size;
        let resolved = Rc::new(Cell::new(None));
        let arrow_position = resolved.clone();
        let arrow = self.arrow;
        let background = self
            .style
            .background
            .as_ref()
            .and_then(gpui::Fill::color)
            .unwrap_or_else(|| cx.theme().popover.into());
        let ring = popover_ring(cx);
        let radius = cx.theme().radius;
        let appearance = self.appearance;
        let style = self.style;
        let children = self.children;
        let content = self.content;
        let trigger_style = self.trigger_style;

        BasePopover::new(self.id)
            .anchor(self.anchor)
            .offset(offset)
            .on_position(move |position, trigger| resolved.set(Some((position, trigger))))
            .mouse_button(self.mouse_button)
            .default_open(self.default_open)
            .overlay_closable(self.overlay_closable)
            .content(move |state, window, cx| {
                v_flex()
                    .id("content")
                    .occlude()
                    .tab_group()
                    .when(appearance, |this| this.popover_style(cx).p_3())
                    .when_some(content, |this, content| {
                        this.child((content)(state, window, cx))
                    })
                    .children(children)
                    .refine_style(&style)
                    .when(arrow, |this| {
                        this.child(
                            canvas(
                                |_, _, _| {},
                                move |bounds, _, window, _| {
                                    let Some((_, trigger)) = arrow_position.get() else {
                                        return;
                                    };
                                    let (side, target) = arrow_anchor(anchor, trigger);
                                    let trigger = Bounds::new(target, gpui::size(px(0.), px(0.)));
                                    let points =
                                        arrow_points(bounds, trigger, side, arrow_size, radius);
                                    let mut fill = PathBuilder::fill();
                                    fill.move_to(points[0]);
                                    fill.line_to(points[1]);
                                    fill.line_to(points[2]);
                                    fill.close();
                                    if let Ok(path) = fill.build() {
                                        window.paint_path(path, background);
                                    }
                                    // The triangle ends exactly at the surface edge.
                                    // Cover the ring and the antialiased base on both
                                    // sides of that edge before drawing its two slopes.
                                    window.paint_quad(gpui::fill(
                                        arrow_join_bounds(points, side, px(1.)),
                                        background,
                                    ));
                                    if appearance {
                                        let mut outline = PathBuilder::stroke(px(1.));
                                        outline.move_to(points[0]);
                                        outline.line_to(points[1]);
                                        outline.line_to(points[2]);
                                        if let Ok(path) = outline.build() {
                                            window.paint_path(path, ring);
                                        }
                                    }
                                },
                            )
                            .absolute()
                            .inset_0()
                            .size_full(),
                        )
                    })
            })
            .when_some(self.trigger, |this, trigger| this.trigger_with(trigger))
            .when_some(trigger_style, |this, style| this.refine_style(&style))
            .when_some(self.open, |this, open| this.open(open))
            .when_some(self.tracked_focus_handle, |this, handle| {
                this.track_focus(&handle)
            })
            .when_some(self.on_open_change, |this, callback| {
                this.on_open_change(move |open, window, cx| callback(open, window, cx))
            })
            .into_any_element()
    }
}

/// The arrow follows the named anchor instead of always aiming at trigger center.
fn arrow_anchor(anchor: Anchor, trigger: Bounds<Pixels>) -> (Placement, Point<Pixels>) {
    match anchor {
        Anchor::TopLeft => (Placement::Bottom, trigger.bottom_left()),
        Anchor::TopCenter => (Placement::Bottom, trigger.bottom_center()),
        Anchor::TopRight => (Placement::Bottom, trigger.bottom_right()),
        Anchor::BottomLeft => (Placement::Top, trigger.origin),
        Anchor::BottomCenter => (Placement::Top, trigger.top_center()),
        Anchor::BottomRight => (Placement::Top, trigger.top_right()),
        Anchor::LeftCenter => (Placement::Right, trigger.right_center()),
        Anchor::RightCenter => (Placement::Left, trigger.left_center()),
    }
}

/// Clamp the arrow base clear of rounded corners while aiming at the trigger.
fn arrow_points(
    surface: Bounds<Pixels>,
    trigger: Bounds<Pixels>,
    side: Placement,
    depth: Pixels,
    radius: Pixels,
) -> [Point<Pixels>; 3] {
    let horizontal = side.is_horizontal();
    let (start, end, target) = if horizontal {
        (surface.top(), surface.bottom(), trigger.center().y)
    } else {
        (surface.left(), surface.right(), trigger.center().x)
    };
    let half = depth.min((end - start) * 0.5);
    let inset = (radius + half).min((end - start) * 0.5);
    let center = target.clamp(start + inset, end - inset);
    match side {
        Placement::Bottom => [
            point(center - half, surface.top()),
            point(center, surface.top() - depth),
            point(center + half, surface.top()),
        ],
        Placement::Top => [
            point(center - half, surface.bottom()),
            point(center, surface.bottom() + depth),
            point(center + half, surface.bottom()),
        ],
        Placement::Right => [
            point(surface.left(), center - half),
            point(surface.left() - depth, center),
            point(surface.left(), center + half),
        ],
        Placement::Left => [
            point(surface.right(), center - half),
            point(surface.right() + depth, center),
            point(surface.right(), center + half),
        ],
    }
}

fn arrow_join_bounds(
    points: [Point<Pixels>; 3],
    side: Placement,
    stroke: Pixels,
) -> Bounds<Pixels> {
    // Inset by the stroke width so the patch remains inside the triangle's
    // slopes at the outer edge of the ring, including on very small surfaces.
    if side.is_horizontal() {
        let inset = stroke.min((points[2].y - points[0].y) * 0.5);
        Bounds::from_corners(
            point(points[0].x - stroke, points[0].y + inset),
            point(points[2].x + stroke, points[2].y - inset),
        )
    } else {
        let inset = stroke.min((points[2].x - points[0].x) * 0.5);
        Bounds::from_corners(
            point(points[0].x + inset, points[0].y - stroke),
            point(points[2].x - inset, points[2].y + stroke),
        )
    }
}
