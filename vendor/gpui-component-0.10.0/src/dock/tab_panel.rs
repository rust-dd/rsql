//! The gpui-component appearance for a tab group.
//!
//! `gpui_base::dock::TabGroup` owns the behavior — membership, the displayed
//! tab, drag hit-testing, the zoom flag — and draws none of it. Everything
//! visible is here: the tab bar, the toolbar, the ellipsis menu, the dock
//! collapse affordances, the drop placeholder, and the styled drag preview.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
    sync::Arc,
};

use gpui::{
    Anchor, AnyElement, AnyView, App, AppContext as _, Context, Div, Empty,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    Stateful, StatefulInteractiveElement as _, StyleRefinement, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, rems,
};
use gpui_base::{
    LogicalStyledExt as _,
    dock::{
        AnyDrag, DockPlacement, DragPanel, DropIndicator, NodeId, PaneNode, PaneRef, PanelId,
        TabGroupContext, TabGroupRenderer,
    },
    spring,
};
use rust_i18n::t;

use crate::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    dock::{
        ClosePanel, PanelControl, PanelHandle, PanelStyle, SkinShared, ToggleZoom,
        toggle_dock_by_user,
    },
    h_flex,
    menu::DropdownMenu as _,
    tab::{Tab, TabBar},
};

/// Names the tab bar's zoom button in the debug-bounds map, so a test can ask
/// a really-drawn frame whether the control was offered.
const ZOOM_CONTROL_SELECTOR: &str = "dock-tab-bar-zoom-control";

/// Debug-bounds selector for a tab's close (X) button, for tests.
const CLOSE_BUTTON_SELECTOR: &str = "dock-tab-close-button";

/// Debug-bounds selector prefix for a tab, suffixed with its panel index.
const TAB_SELECTOR: &str = "dock-tab";

/// Debug-bounds selector prefix for a dock's collapse button, suffixed with
/// its placement.
const DOCK_TOGGLE_SELECTOR: &str = "dock-toggle";

/// The size the styled drag preview occupies, reported to base so a drop
/// placeholder knows where to fly in from.
const DRAG_PREVIEW_SIZE: gpui::Size<gpui::Pixels> = gpui::size(px(96.), px(30.));

/// The preview that follows the cursor while a panel is dragged.
///
/// `gpui_base::dock::DragPanel` is the payload and draws nothing; this is the
/// appearance half, reintroduced here.
pub struct DragPanelPreview {
    panel: Arc<dyn gpui_base::dock::PanelView>,
}

impl Render for DragPanelPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("drag-panel")
            .cursor_grab()
            .py_1()
            .px_3()
            .w_24()
            .overflow_hidden()
            .whitespace_nowrap()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .text_color(cx.theme().tab_foreground)
            .bg(cx.theme().tokens.tab_active)
            .opacity(0.75)
            .child(panel_title(&self.panel, window, cx))
    }
}

/// A panel's title, or its registered name when it reached base without this
/// crate's handle and so carries no presentation. See [`PanelHandle::of`].
pub(crate) fn panel_title(
    panel: &Arc<dyn gpui_base::dock::PanelView>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(handle) = PanelHandle::of(panel) else {
        let name = panel.panel_name(cx);
        warn_unwrapped_once(panel.panel_id(cx), name);
        return SharedString::from(name).into_any_element();
    };
    handle.title(window, cx)
}

thread_local! {
    /// Panels already warned about. `panel_title` sits on the render path, so
    /// an unguarded warning would repeat at frame rate and bury the very
    /// signal it exists to give.
    ///
    /// Keyed by panel rather than a bare `Once` so a second wrongly installed
    /// panel is still named, and a runtime set rather than a `debug_assert!`
    /// so a release build says it too — the consequence is a shipped app whose
    /// tabs are titleless, which is exactly when someone needs to be told.
    /// Thread-local because rendering happens on one thread, so no lock is
    /// needed.
    static WARNED_UNWRAPPED: RefCell<HashSet<PanelId>> = RefCell::new(HashSet::new());
}

/// Say once, per panel, that a panel reached the skin without its
/// presentation handle. Silent otherwise, and visual-only: the panel docks,
/// drags and persists, it just has no title. The shorter method is the wrong
/// one — `DockLayout::panel` and `DockArea::add_panel` accept a
/// `gpui_component::dock::Panel` and store the bare entity — so this says
/// which panel and what to call instead.
fn warn_unwrapped_once(panel: PanelId, name: &'static str) {
    if !WARNED_UNWRAPPED.with(|warned| warned.borrow_mut().insert(panel)) {
        return;
    }
    tracing::warn!(
        panel = name,
        "dock panel reached the skin without its presentation handle, so it \
         draws its panel name instead of its title; install it with \
         `gpui_component::dock::panel_handle(..)` and `DockLayout::panel_view` \
         / `DockArea::add_panel_view` rather than `DockLayout::panel` / \
         `DockArea::add_panel`"
    );
}

/// Where the zoom affordance goes for the group's displayed panel, or `None`
/// when there is none to offer.
///
/// Two questions, and both have to be asked. [`Panel::zoom_control`] says
/// *where* the control appears; [`gpui_base::dock::Panel::zoomable`] says
/// whether zooming happens at all, and base refuses a zoom that fails it. The
/// old dock had a single `zoomable() -> Option<PanelControl>` that could not
/// disagree with itself; split across the seam it can, and a panel answering
/// `zoomable() == false` with `zoom_control() == Some(Toolbar)` would
/// otherwise draw a button that does nothing.
fn zoom_control(group: &TabGroupContext, cx: &App) -> Option<PanelControl> {
    let panel = group.active_panel()?;
    panel
        .zoomable(cx)
        .then(|| PanelHandle::of(panel).and_then(|handle| handle.zoom_control(cx)))
        .flatten()
}

/// The payload for dragging the tab at `ix` out of its group, or `None` when
/// this group must not be rearranged.
///
/// The guard is the skin's. [`TabGroupContext::drag_panel`] answers for any
/// tab in range whether or not the group may be rearranged, so a tab bar that
/// forgets to ask [`TabGroupContext::is_draggable`] makes a group that has
/// nowhere to go — a dock's last group, a locked dock — draggable anyway. The
/// old dock asked the same question, spelled `state.draggable`.
fn tab_drag(group: &TabGroupContext, ix: usize, cx: &App) -> Option<DragPanel> {
    group
        .is_draggable()
        .then(|| group.drag_panel(ix, cx))
        .flatten()
}

/// The left-most, top-most tab group in a container — where a left dock's
/// collapse affordance goes. Mirrors the old `StackPanel::left_top_tab_panel`.
fn left_top_group(node: &PaneNode) -> Option<NodeId> {
    match node.kind() {
        PaneRef::Tabs { .. } => Some(node.id()),
        PaneRef::Split { children, .. } => children.first().and_then(left_top_group),
    }
}

/// The right-most, top-most tab group. A vertical split stacks its children,
/// so its *first* child is the top one; a horizontal split's last child is the
/// right-most. Mirrors the old `StackPanel::right_top_tab_panel`.
fn right_top_group(node: &PaneNode) -> Option<NodeId> {
    match node.kind() {
        PaneRef::Tabs { .. } => Some(node.id()),
        PaneRef::Split { axis, children, .. } => match axis {
            gpui::Axis::Vertical => children.first(),
            gpui::Axis::Horizontal => children.last(),
        }
        .and_then(right_top_group),
    }
}

/// One tab group's appearance.
///
/// Built per group — `DockAreaRenderer::tab_group_renderer` is called once per
/// container — so the tab bar's scroll position belongs to the group whose
/// tabs it scrolls.
pub(crate) struct TabGroupSkin {
    shared: Rc<SkinShared>,
    scroll_handle: ScrollHandle,
    /// The displayed tab the last frame drew, so a change scrolls the new tab
    /// into view. The old dock recorded this at the moment of selection; the
    /// group now owns selection, so the skin notices instead of being told.
    last_active_ix: Rc<Cell<Option<usize>>>,
}

impl TabGroupSkin {
    pub(crate) fn new(shared: Rc<SkinShared>) -> Self {
        Self {
            shared,
            scroll_handle: ScrollHandle::default(),
            last_active_ix: Rc::new(Cell::new(None)),
        }
    }

    /// Whether a dock's collapse affordance belongs in *this* group's tab bar,
    /// and which way it points. `None` means this group draws none.
    fn dock_toggle_button(
        &self,
        placement: DockPlacement,
        group: &TabGroupContext,
        cx: &mut App,
    ) -> Option<Button> {
        if group.is_zoomed() || !self.shared.is_toggle_button_visible() {
            return None;
        }

        let area = self.shared.area().upgrade()?;
        let area = area.read(cx);
        // A dock that does not exist is not collapsible, so this covers the
        // old `left_dock.is_some()` test too.
        if !area.is_dock_collapsible(placement) {
            return None;
        }

        let designated = match placement {
            DockPlacement::Left => area
                .layout(DockPlacement::Center)
                .and_then(|tree| left_top_group(tree.root())),
            DockPlacement::Right => area
                .layout(DockPlacement::Center)
                .and_then(|tree| right_top_group(tree.root())),
            DockPlacement::Bottom => area
                .layout(DockPlacement::Bottom)
                .and_then(|tree| left_top_group(tree.root())),
            DockPlacement::Center => None,
        };
        if designated != Some(group.node()) {
            return None;
        }

        let is_open = area.is_dock_open(placement);
        // The icon shows the physical side the dock sits on.
        let icon = match (
            placement.physical(gpui_base::LayoutDirection::global(cx)),
            is_open,
        ) {
            (DockPlacement::Left, true) => IconName::PanelLeft,
            (DockPlacement::Left, false) => IconName::PanelLeftOpen,
            (DockPlacement::Right, true) => IconName::PanelRight,
            (DockPlacement::Right, false) => IconName::PanelRightOpen,
            (DockPlacement::Bottom, true) => IconName::PanelBottom,
            (DockPlacement::Bottom, false) => IconName::PanelBottomOpen,
            (DockPlacement::Center, _) => return None,
        };

        let area = self.shared.area().clone();
        Some(
            Button::new(SharedString::from(format!("toggle-dock:{:?}", placement)))
                .icon(icon)
                .xsmall()
                .ghost()
                .tab_stop(false)
                .tooltip(match is_open {
                    true => t!("Dock.Collapse"),
                    false => t!("Dock.Expand"),
                })
                .debug_selector(move || format!("{DOCK_TOGGLE_SELECTOR}-{placement:?}"))
                .on_click(move |_, window, cx| {
                    _ = area.update(cx, |area, cx| {
                        toggle_dock_by_user(area, placement, window, cx)
                    });
                }),
        )
    }

    /// The trailing controls: the panel's own buttons, the zoom affordance,
    /// and the ellipsis menu.
    fn render_toolbar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        if group.is_collapsed() {
            return div();
        }

        let zoomed = group.is_zoomed();
        let handle = group.active_panel().and_then(PanelHandle::of);
        let control = zoom_control(group, cx);
        let toolbar_zoom = control.is_some_and(|control| control.toolbar_visible());
        let menu_zoom = control.is_some_and(|control| control.menu_visible());
        let closable = group.is_closable();
        let buttons = handle.and_then(|handle| handle.toolbar_buttons(window, cx));
        let panel = handle.map(|handle| handle.panel());

        h_flex()
            .gap_1()
            .occlude()
            .when_some(buttons, |this, buttons| {
                this.children(
                    buttons
                        .into_iter()
                        .map(|button| button.xsmall().ghost().tab_stop(false)),
                )
            })
            .when_some(
                match (zoomed, toolbar_zoom) {
                    (true, _) => Some(("zoom-out", IconName::Minimize, t!("Dock.Zoom Out"))),
                    (false, true) => Some(("zoom-in", IconName::Maximize, t!("Dock.Zoom In"))),
                    (false, false) => None,
                },
                |this, (id, icon, tooltip)| {
                    this.child(
                        Button::new(id)
                            .icon(icon)
                            .xsmall()
                            .ghost()
                            .tab_stop(false)
                            .tooltip_with_action(tooltip, &ToggleZoom, None)
                            .selected(zoomed)
                            // Whether this button was drawn is the whole of
                            // the `zoom_control` decision, and there is no
                            // other way to ask a drawn tree about it. A no-op
                            // outside test builds; see `debug_selector`.
                            .debug_selector(|| ZOOM_CONTROL_SELECTOR.to_string())
                            .on_click({
                                let group = group.clone();
                                move |_, window, cx| group.toggle_zoom(window, cx)
                            }),
                    )
                },
            )
            .child(
                Button::new("menu")
                    .icon(IconName::Ellipsis)
                    .xsmall()
                    .ghost()
                    .tab_stop(false)
                    .dropdown_menu(move |menu, window, cx| {
                        menu.when_some(panel.clone(), |menu, panel| {
                            panel.dropdown_menu(menu, window, cx)
                        })
                        .separator()
                        .menu_with_disabled(
                            match zoomed {
                                true => t!("Dock.Zoom Out"),
                                false => t!("Dock.Zoom In"),
                            },
                            Box::new(ToggleZoom),
                            !menu_zoom,
                        )
                        .when(closable, |menu| {
                            menu.separator()
                                .menu(t!("Dock.Close"), Box::new(ClosePanel))
                        })
                    })
                    .anchor(Anchor::TopRight),
            )
    }

    /// The one-panel title bar: no tabs, just the title and the controls.
    fn render_title(
        &self,
        group: &TabGroupContext,
        ix: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let panel = &group.panels()[ix];
        let left_button = self.dock_toggle_button(DockPlacement::Left, group, cx);
        let bottom_button = self.dock_toggle_button(DockPlacement::Bottom, group, cx);
        let right_button = self.dock_toggle_button(DockPlacement::Right, group, cx);
        let has_leading = left_button.is_some() || bottom_button.is_some();
        let handle = PanelHandle::of(panel);
        let title_style = handle.and_then(|handle| handle.title_style(cx));
        let drag = tab_drag(group, ix, cx);

        h_flex()
            .flex_row_dir(cx)
            .justify_between()
            .h(px(30.))
            .py_2()
            .ps(rems(0.75), cx)
            .pe(rems(0.5), cx)
            .when(left_button.is_some(), |this| this.ps(rems(0.5), cx))
            .when(right_button.is_some(), |this| this.pe(rems(0.5), cx))
            .when_some(title_style, |this, style| {
                this.bg(style.background).text_color(style.foreground)
            })
            .when(has_leading, |this| {
                this.child(
                    h_flex()
                        .flex_shrink_0()
                        .me(rems(0.25), cx)
                        .gap_1()
                        .children(left_button)
                        .children(bottom_button),
                )
            })
            .child(
                div()
                    .id("tab")
                    .flex_1()
                    .min_w_16()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(panel_title(panel, window, cx))
                    .when_some(drag, |this, drag| {
                        this.on_drag(drag, {
                            let panel = panel.clone();
                            move |drag, offset, _, cx| {
                                cx.stop_propagation();
                                drag.set_drag_offset(offset);
                                drag.set_preview_size(DRAG_PREVIEW_SIZE);
                                cx.new(|_| DragPanelPreview {
                                    panel: panel.clone(),
                                })
                            }
                        })
                    }),
            )
            .children(handle.and_then(|handle| handle.title_suffix(window, cx)))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .ms(rems(0.25), cx)
                    .gap_1()
                    .child(self.render_toolbar(group, window, cx))
                    .children(right_button),
            )
            .into_any_element()
    }

    /// The full tab bar.
    fn render_tabs(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let left_button = self.dock_toggle_button(DockPlacement::Left, group, cx);
        let bottom_button = self.dock_toggle_button(DockPlacement::Bottom, group, cx);
        let right_button = self.dock_toggle_button(DockPlacement::Right, group, cx);
        let has_leading = left_button.is_some() || bottom_button.is_some();
        let is_bottom_dock = bottom_button.is_some();
        let collapsed = group.is_collapsed();

        let droppable = group.is_droppable();
        let tabs_count = group.panels().len();
        let active_ix = group.active_ix();
        let displayed = group.active_panel().map(|panel| panel.panel_id(cx));
        let visible: Vec<usize> = group
            .panels()
            .iter()
            .enumerate()
            .filter(|(_, panel)| panel.visible(cx))
            .map(|(ix, _)| ix)
            .collect();
        let displayed_ix = displayed.and_then(|displayed| {
            group
                .panels()
                .iter()
                .position(|panel| panel.panel_id(cx) == displayed)
        });

        // Bring a newly displayed tab into view. The group owns selection now,
        // so the skin notices the change rather than being told about it.
        if self.last_active_ix.replace(Some(active_ix)) != Some(active_ix) {
            if let Some(visible_ix) = visible.iter().position(|ix| *ix == active_ix) {
                self.scroll_handle.scroll_to_item(visible_ix);
            }
        }

        TabBar::new("tab-bar")
            .track_scroll(&self.scroll_handle)
            .when(has_leading, |this| {
                this.prefix(
                    h_flex()
                        .items_center()
                        .top_0()
                        // Overlap the first tab by 1px so their borders don't double up.
                        .inset_e(-px(1.), cx)
                        .border_e(px(1.), cx)
                        .border_b_1()
                        .h_full()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().tokens.tab_bar)
                        .px_2()
                        .children(left_button)
                        .children(bottom_button),
                )
            })
            .children(
                visible
                    .into_iter()
                    .map(|ix| {
                        let panel = &group.panels()[ix];
                        let handle = PanelHandle::of(panel);
                        let drag = tab_drag(group, ix, cx);

                        Tab::new()
                            .ix(ix)
                            .debug_selector(move || format!("{TAB_SELECTOR}-{ix}"))
                            .tab_bar_prefix(has_leading)
                            .map(|this| match handle.and_then(|handle| handle.tab_name(cx)) {
                                Some(tab_name) => this.child(tab_name),
                                None => this.child(panel_title(panel, window, cx)),
                            })
                            // Per-tab close (X) button. The gate mirrors
                            // `TabGroup::close_panel`, plus `!collapsed`, since a
                            // collapsed strip is a way back in, not a place to
                            // close. Stops propagation so the click closes by id
                            // without also selecting the tab.
                            .when(
                                !collapsed
                                    && self.shared.close_button_visible.get()
                                    && group.is_panel_closable(panel.panel_id(cx), cx),
                                |this| {
                                    this.suffix(
                                        Button::new(("close-tab", ix))
                                            .icon(IconName::Close)
                                            .xsmall()
                                            // The regular ghost hover matches the
                                            // inactive tab bar background in the
                                            // default theme. Use a stronger theme
                                            // surface while keeping a transparent
                                            // idle background and no border.
                                            .custom(
                                                ButtonCustomVariant::new(cx)
                                                    .foreground(cx.theme().secondary_foreground)
                                                    .hover(*cx.theme().tokens.secondary_hover)
                                                    .active(*cx.theme().tokens.secondary_active),
                                            )
                                            // The 20px XS button has a 12px icon:
                                            // 4px inside plus 8px outside matches
                                            // the label's 12px leading padding.
                                            // Offset the label's own 12px right
                                            // padding and the tab's 4px gap so the
                                            // text-to-icon distance is also 12px.
                                            .ms(-px(8.), cx)
                                            .me(rems(0.5), cx)
                                            .tab_stop(false)
                                            .debug_selector(|| CLOSE_BUTTON_SELECTOR.to_string())
                                            .on_click({
                                                let group = group.clone();
                                                let panel_id = panel.panel_id(cx);
                                                move |_, window, cx| {
                                                    cx.stop_propagation();
                                                    group.close(panel_id, window, cx);
                                                }
                                            }),
                                    )
                                },
                            )
                            // A collapsed group shows no tab as active: the
                            // strip is a way back in, not a selection. The
                            // comparison is against the panel on screen, not
                            // the stored index: a hidden displayed tab falls
                            // back to the first visible one.
                            .selected(!collapsed && Some(ix) == displayed_ix)
                            .on_click({
                                let group = group.clone();
                                let area = self.shared.area().clone();
                                let last_active_ix = self.last_active_ix.clone();
                                move |_, window, cx| {
                                    // The tab bar reveals a clicked tab itself,
                                    // so the change must not snap it into view.
                                    last_active_ix.set(Some(ix));
                                    group.select_tab(ix, window, cx);

                                    // Clicking the strip of a collapsed bottom
                                    // dock is how it is opened again.
                                    if is_bottom_dock && collapsed {
                                        _ = area.update(cx, |area, cx| {
                                            toggle_dock_by_user(
                                                area,
                                                DockPlacement::Bottom,
                                                window,
                                                cx,
                                            )
                                        });
                                    }
                                }
                            })
                            // A collapsed group is a strip of tabs with no
                            // content, so there is nothing to rearrange in it.
                            .when(!collapsed, |this| {
                                this.when_some(drag, |this, drag| {
                                    this.on_drag(drag, {
                                        let panel = panel.clone();
                                        move |drag, offset, _, cx| {
                                            cx.stop_propagation();
                                            drag.set_drag_offset(offset);
                                            drag.set_preview_size(DRAG_PREVIEW_SIZE);
                                            cx.new(|_| DragPanelPreview {
                                                panel: panel.clone(),
                                            })
                                        }
                                    })
                                })
                                .when(droppable, |this| {
                                    this.drag_over::<DragPanel>(|this, _, _, cx| {
                                        this.rounded_s(px(0.), cx)
                                            .border_s(px(2.), cx)
                                            .border_e(px(0.), cx)
                                            .border_color(cx.theme().drag_border)
                                    })
                                    .on_drop({
                                        let group = group.clone();
                                        move |drag: &DragPanel, window, cx| {
                                            group.drop_panel(
                                                drag.clone(),
                                                Some(ix),
                                                true,
                                                window,
                                                cx,
                                            );
                                        }
                                    })
                                    .drag_over::<AnyDrag>(|this, _, _, cx| {
                                        this.rounded_s(px(0.), cx)
                                            .border_s(px(2.), cx)
                                            .border_e(px(0.), cx)
                                            .border_color(cx.theme().drag_border)
                                    })
                                    .on_drop({
                                        let group = group.clone();
                                        move |item: &AnyDrag, window, cx| {
                                            group.drop_item(item.clone(), None, window, cx);
                                        }
                                    })
                                })
                            })
                    })
                    .collect::<Vec<_>>(),
            )
            .last_empty_space(
                // Empty space so a panel can be moved past the last tab.
                div()
                    .id("tab-bar-empty-space")
                    .h_full()
                    .flex_grow_1()
                    .min_w_16()
                    .when(droppable, |this| {
                        this.drag_over::<DragPanel>(|this, _, _, cx| {
                            this.bg(cx.theme().tokens.drop_target)
                        })
                        .on_drop({
                            let group = group.clone();
                            let node = group.node();
                            move |drag: &DragPanel, window, cx| {
                                // A panel dropped past its own last tab lands
                                // in the final slot; one from elsewhere is
                                // appended in the background.
                                let ix = (drag.source() == node).then(|| tabs_count - 1);
                                group.drop_panel(drag.clone(), ix, false, window, cx);
                            }
                        })
                        .drag_over::<AnyDrag>(|this, _, _, cx| {
                            this.bg(cx.theme().tokens.drop_target)
                        })
                        .on_drop({
                            let group = group.clone();
                            move |item: &AnyDrag, window, cx| {
                                group.drop_item(item.clone(), None, window, cx);
                            }
                        })
                    }),
            )
            .when(!collapsed, |this| {
                this.suffix(
                    h_flex()
                        .items_center()
                        .top_0()
                        .inset_e(px(0.), cx)
                        .border_s(px(1.), cx)
                        .border_b_1()
                        .h_full()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().tokens.tab_bar)
                        .px_2()
                        .gap_1()
                        .children(
                            group
                                .active_panel()
                                .and_then(PanelHandle::of)
                                .and_then(|handle| handle.title_suffix(window, cx)),
                        )
                        .child(self.render_toolbar(group, window, cx))
                        .children(right_button),
                )
            })
            .into_any_element()
    }
}

impl TabGroupRenderer for TabGroupSkin {
    fn frame(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        let control = zoom_control(group, cx);

        // The column, the fill and the clip are base's now, applied around
        // this. What is left is the background and the two actions.
        div()
            .id("tab-panel")
            .bg(cx.theme().tokens.background)
            // A collapsed group is a strip of tabs with no content, and the
            // actions act on content. The old dock gated them the same way.
            .when(!group.is_collapsed(), |this| {
                this.on_action({
                    let group = group.clone();
                    move |_: &ToggleZoom, window, cx| {
                        // The affordance decides the control, so a panel that
                        // offers none is not zoomed *in* by the keybinding
                        // either. Zooming out is never refused: a panel that
                        // stopped offering the control while zoomed would
                        // otherwise strand the user with no way back.
                        if !group.is_zoomed() && control.is_none() {
                            return;
                        }
                        group.toggle_zoom(window, cx);
                    }
                })
                .on_action({
                    let group = group.clone();
                    move |_: &ClosePanel, window, cx| {
                        let Some(panel) = group.active_panel() else {
                            return;
                        };
                        let panel = panel.panel_id(cx);
                        group.close(panel, window, cx);
                    }
                })
            })
    }

    fn content_frame(
        &self,
        group: &TabGroupContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let padded = group.panels().len() > 1
            && group
                .active_panel()
                .and_then(PanelHandle::of)
                .is_none_or(|handle| handle.inner_padding(cx));

        // The fill and the collapsed-group exception are base's; the padding
        // is this skin's, and is the only reason this hook is implemented.
        div().id("active-panel").when(padded, |this| this.pt_2())
    }

    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let visible: Vec<usize> = group
            .panels()
            .iter()
            .enumerate()
            .filter(|(_, panel)| panel.visible(cx))
            .map(|(ix, _)| ix)
            .collect();

        match visible.as_slice() {
            [] => Empty.into_any_element(),
            [ix] if self.shared.panel_style() == PanelStyle::Auto => {
                // A panel that draws its own chrome declines the title bar.
                let panel = &group.panels()[*ix];
                if PanelHandle::of(panel).is_some_and(|handle| !handle.title_bar(cx)) {
                    return Empty.into_any_element();
                }
                self.render_title(group, *ix, window, cx)
            }
            _ => self.render_tabs(group, window, cx),
        }
    }

    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        if group.is_collapsed() {
            return Empty.into_any_element();
        }

        div()
            .id("tab-content")
            .overflow_y_scroll()
            .overflow_x_hidden()
            .flex_1()
            .child(panel.cached(StyleRefinement::default().absolute().size_full()))
            .into_any_element()
    }

    fn render_drop_indicator(
        &self,
        indicator: DropIndicator,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let to = indicator.to();
        // The placeholder chases the drop it would land in. Its rect was
        // previously replayed from the drag source on every epoch, so crossing
        // several drop zones in one drag restarted the walk at each one; the
        // springs carry it through instead, and the element no longer needs an
        // outer frame to hold the destination while an inner one walks to it.
        let id = "drop-placeholder";
        let placeholder_spring = cx.theme().motion_tokens().spring_move.with_epsilon(0.5);
        let left = spring((id, "left"), to.origin().x, placeholder_spring, window, cx);
        let top = spring((id, "top"), to.origin().y, placeholder_spring, window, cx);
        let width = spring(
            (id, "width"),
            to.size().width,
            placeholder_spring,
            window,
            cx,
        );
        let height = spring(
            (id, "height"),
            to.size().height,
            placeholder_spring,
            window,
            cx,
        );

        Some(
            div()
                .absolute()
                .bg(cx.theme().tokens.drop_target)
                .left(left)
                .top(top)
                .w(width)
                .h(height)
                .into_any_element(),
        )
    }
}
