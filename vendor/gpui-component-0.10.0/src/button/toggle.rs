use gpui_base::LogicalStyledExt as _;
use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{
    AnyElement, App, Axis, Corners, Edges, ElementId, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, StyleRefinement, Styled,
    Window, prelude::FluentBuilder as _, relative,
};
use gpui_base::{Toggle as BaseToggle, ToggleGroup as BaseToggleGroup};
use smallvec::{SmallVec, smallvec};

use crate::{ActiveTheme, Disableable, Icon, Sizable, Size, StyledExt, tooltip::ComponentTooltip};

#[derive(Default, Copy, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToggleVariant {
    #[default]
    Ghost,
    Outline,
}

pub trait ToggleVariants: Sized {
    /// Set the variant of the toggle.
    fn with_variant(self, variant: ToggleVariant) -> Self;
    /// Set the variant to ghost.
    fn ghost(self) -> Self {
        self.with_variant(ToggleVariant::Ghost)
    }
    /// Set the variant to outline.
    fn outline(self) -> Self {
        self.with_variant(ToggleVariant::Outline)
    }
}

#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    style: StyleRefinement,
    checked: bool,
    size: Size,
    variant: ToggleVariant,
    disabled: bool,
    border_corners: Corners<bool>,
    border_edges: Edges<bool>,
    children: SmallVec<[AnyElement; 1]>,
    on_click: Option<Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>>,
    tooltip: ComponentTooltip,
}

impl Toggle {
    /// Create a new Toggle element.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            checked: false,
            size: Size::default(),
            variant: ToggleVariant::default(),
            disabled: false,
            border_corners: Corners {
                top_left: true,
                top_right: true,
                bottom_left: true,
                bottom_right: true,
            },
            border_edges: Edges::all(true),
            children: smallvec![],
            on_click: None,
            tooltip: ComponentTooltip::default(),
        }
    }

    /// Set tooltip text for the toggle.
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip.text = Some((tooltip.into(), None));
        self
    }

    /// Overrides how long the pointer must rest before the tooltip shows.
    ///
    /// Defaults to [`gpui_base::TooltipDefaults::show_delay`].
    pub fn tooltip_show_delay(mut self, delay: Duration) -> Self {
        self.tooltip.show_delay = Some(delay);
        self
    }

    /// Add a label to the toggle.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        let label: SharedString = label.into();
        self.children.push(label.into_any_element());
        self
    }

    /// Add icon to the toggle.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        let icon: Icon = icon.into();
        self.children.push(icon.into());
        self
    }

    /// Set the checked state of the toggle, default: false
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Set the callback to be called when the toggle is clicked.
    ///
    /// The `&bool` parameter represents the new checked state of the toggle.
    pub fn on_click(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    pub(crate) fn border_corners(mut self, corners: impl Into<Corners<bool>>) -> Self {
        self.border_corners = corners.into();
        self
    }

    pub(crate) fn border_edges(mut self, edges: impl Into<Edges<bool>>) -> Self {
        self.border_edges = edges.into();
        self
    }
}

impl ToggleVariants for Toggle {
    fn with_variant(mut self, variant: ToggleVariant) -> Self {
        self.variant = variant;
        self
    }
}

impl ParentElement for Toggle {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Disableable for Toggle {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Sizable for Toggle {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl Styled for Toggle {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Toggle {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let checked = self.checked;
        let disabled = self.disabled;
        let hoverable = !disabled && !checked;
        let rounding = cx.theme().radius;
        let pressed_background = cx.theme().tokens.accent;
        let pressed_foreground = cx.theme().accent_foreground;
        let instance_style = self.style.clone();
        let (border_corners, border_edges) =
            super::logical_corners_edges(self.border_corners, self.border_edges, cx);

        BaseToggle::new(self.id)
            .pressed(checked)
            .disabled(disabled)
            .when_some(
                self.tooltip.text.as_ref().map(|(text, _)| text.clone()),
                |this, label| this.accessibility_label(label),
            )
            .when_some(self.on_click, |this, on_click| {
                this.on_change(move |next, _, window, cx| on_click(&next, window, cx))
            })
            .flex_row_dir(cx)
            .line_height(relative(1.25))
            .items_center()
            .justify_center()
            .map(|this| match self.size {
                Size::XSmall => this.min_w_5().h_5().px_0p5().text_xs(),
                Size::Small => this.min_w_6().h_6().px_1().text_sm(),
                Size::Large => this.min_w_9().h_9().px_3().text_base(),
                _ => this.min_w_8().h_8().px_2().text_sm(),
            })
            .when(border_corners.top_left, |this| this.rounded_tl(rounding))
            .when(border_corners.top_right, |this| this.rounded_tr(rounding))
            .when(border_corners.bottom_left, |this| this.rounded_bl(rounding))
            .when(border_corners.bottom_right, |this| {
                this.rounded_br(rounding)
            })
            .when(self.variant == ToggleVariant::Outline, |this| {
                this.when(border_edges.left, |this| this.border_l_1())
                    .when(border_edges.right, |this| this.border_r_1())
                    .when(border_edges.top, |this| this.border_t_1())
                    .when(border_edges.bottom, |this| this.border_b_1())
                    .border_color(cx.theme().border)
                    .bg(cx.theme().tokens.background)
            })
            // Always register the hover style: GPUI only refreshes the cached
            // hover state while one is present. If a checked toggle skipped it,
            // the stale state would paint its label in the hover color once
            // something else unchecks it while the pointer rests elsewhere.
            .hover(|this| {
                if !hoverable {
                    return this;
                }
                this.bg(cx.theme().tokens.accent)
                    .text_color(cx.theme().accent_foreground)
            })
            .styles(|styles| {
                styles.pressed(|style| {
                    style
                        .bg(pressed_background)
                        .text_color(pressed_foreground)
                        .refine_style(&instance_style)
                })
            })
            .refine_style(&self.style)
            .children(self.children)
            .map(|this| self.tooltip.apply(this))
    }
}

/// A group of toggles.
#[derive(IntoElement)]
pub struct ToggleGroup {
    id: ElementId,
    style: StyleRefinement,
    size: Size,
    variant: ToggleVariant,
    disabled: bool,
    segmented: bool,
    single: bool,
    items: Vec<Toggle>,
    on_click: Option<Rc<dyn Fn(&Vec<bool>, &mut Window, &mut App) + 'static>>,
}

impl ToggleGroup {
    /// Create a new ToggleGroup element.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            size: Size::default(),
            variant: ToggleVariant::default(),
            disabled: false,
            segmented: false,
            single: false,
            items: Vec::new(),
            on_click: None,
        }
    }

    /// Add a child [`Toggle`] to the group.
    pub fn child(mut self, toggle: impl Into<Toggle>) -> Self {
        self.items.push(toggle.into());
        self
    }

    /// Add multiple [`Toggle`]s to the group.
    pub fn children(mut self, children: impl IntoIterator<Item = impl Into<Toggle>>) -> Self {
        self.items.extend(children.into_iter().map(Into::into));
        self
    }

    /// Set the callback to be called when the toggle group changes.
    ///
    /// The `&Vec<bool>` parameter represents the new check state of each [`Toggle`] in the group.
    /// In a [`single`](Self::single) group at most one of them is `true`.
    pub fn on_click(
        mut self,
        on_click: impl Fn(&Vec<bool>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(on_click));
        self
    }

    /// Render the group as a connected segmented control.
    ///
    /// This only changes the look: it removes the default gap and joins
    /// adjacent item borders into a single segmented outline. Pair it with
    /// [`single`](Self::single) for an exclusive choice.
    pub fn segmented(mut self) -> Self {
        self.segmented = true;
        self
    }

    /// Let at most one item be checked at a time.
    ///
    /// A click on an unchecked item checks it and clears the others. A click
    /// on the checked item clears it, so the group can also hold no selection.
    /// The caller keeps at most one item `checked`; the group does not
    /// reconcile a state in which several are.
    pub fn single(mut self) -> Self {
        self.single = true;
        self
    }
}

impl Sizable for ToggleGroup {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl ToggleVariants for ToggleGroup {
    fn with_variant(mut self, variant: ToggleVariant) -> Self {
        self.variant = variant;
        self
    }
}

impl Disableable for ToggleGroup {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for ToggleGroup {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ToggleGroup {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let disabled = self.disabled;
        let single = self.single;
        let items_len = self.items.len();
        let checks = self
            .items
            .iter()
            .map(|item| item.checked)
            .collect::<Vec<bool>>();
        let clicked_index = Rc::new(Cell::new(None));

        BaseToggleGroup::new(self.id)
            .axis(Axis::Horizontal)
            .child(
                crate::h_flex()
                    .flex_row_dir(cx)
                    .items_center()
                    .when(!self.segmented, |this| this.gap_2())
                    .refine_style(&self.style)
                    .children(self.items.into_iter().enumerate().map({
                        {
                            let clicked_index = clicked_index.clone();
                            move |(ix, item)| {
                                let item = if !self.segmented || items_len == 1 {
                                    item
                                } else if ix == 0 {
                                    item.border_corners(Corners {
                                        top_left: true,
                                        top_right: false,
                                        bottom_left: true,
                                        bottom_right: false,
                                    })
                                    .border_edges(Edges {
                                        left: true,
                                        top: true,
                                        right: true,
                                        bottom: true,
                                    })
                                } else if ix == items_len - 1 {
                                    item.border_corners(Corners {
                                        top_left: false,
                                        top_right: true,
                                        bottom_left: false,
                                        bottom_right: true,
                                    })
                                    .border_edges(Edges {
                                        left: false,
                                        top: true,
                                        right: true,
                                        bottom: true,
                                    })
                                } else {
                                    item.border_corners(Corners {
                                        top_left: false,
                                        top_right: false,
                                        bottom_left: false,
                                        bottom_right: false,
                                    })
                                    .border_edges(Edges {
                                        left: false,
                                        top: true,
                                        right: true,
                                        bottom: true,
                                    })
                                };

                                let effective_disabled = disabled || item.disabled;
                                let clicked_index = clicked_index.clone();
                                item.disabled(effective_disabled)
                                    .with_size(self.size)
                                    .with_variant(self.variant)
                                    .on_click(move |_, _, cx| {
                                        clicked_index.set(Some(ix));
                                        cx.propagate();
                                    })
                            }
                        }
                    })),
            )
            .when_some(
                (!disabled).then_some(self.on_click).flatten(),
                |this, on_click| {
                    this.on_click(move |_, window, cx| {
                        let Some(ix) = clicked_index.take() else {
                            return;
                        };
                        // A single group clears every other item; clicking its
                        // checked item still flips that one off.
                        let mut next = if single {
                            vec![false; checks.len()]
                        } else {
                            checks.clone()
                        };
                        next[ix] = !checks[ix];
                        on_click(&next, window, cx);
                    })
                },
            )
    }
}
