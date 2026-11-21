//! The dock area: the trees, the entity cache that mirrors them, and the
//! reconciliation that keeps the two in step.

use crate::TestSupportExt as _;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use anyhow::Result;
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Axis, Bounds, Context, Div, Empty, Entity,
    EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement, ParentElement,
    Pixels, Point, Render, SharedString, Stateful, Styled as _, Subscription, WeakEntity, Window,
    div, prelude::FluentBuilder as _, px,
};

use crate::{
    ElementExt as _, LogicalStyledExt as _, Placement, ResizablePanelEvent, ResizableState,
    ResizeHandleContext, h_resizable, resizable::PANEL_MIN_SIZE, resizable_panel, v_resizable,
};

use super::{
    dock_placement::{Dock, DockSizing},
    drag::{AnyDrag, DropTarget},
    layout::{
        DockLayout, EditResult, InsertTarget, NodeId, NodeKind, PaneNode, PaneRef, PaneTree,
        PanelId, RootKind,
    },
    panel::{LivePanels, Panel, PanelEvent, PanelView},
    registry::{PanelBuildContext, PanelRegistry},
    state::{DockAreaState, DockPlacement, DockState, PanelInfo, PanelState},
    state_convert::{PanelBuilder, PanelSource as _},
    tab_group::{BareTabGroup, TabGroup, TabGroupConstraints, TabGroupEvent, TabGroupRenderer},
};

/// What the dock area reports outward.
pub enum DockEvent {
    /// The layout changed. Subscribe to persist it; this fires on every edit,
    /// so a subscriber that writes to disk should debounce.
    LayoutChanged,
    /// A host-owned drag item was dropped inside the dock.
    DragDrop { item: AnyDrag, target: DropTarget },
}

/// What fills the whole area, when something does.
///
/// A zoom names a *container*, never the panel inside it. The old dock zoomed
/// the `TabPanel`: `TabPanel` is the only thing that ever emitted
/// `PanelEvent::ZoomIn` — `subscribe_panel` was handed `StackPanel`s too, but
/// those never zoom — so `set_zoomed_in` only ever received a whole tab panel,
/// and a zoomed panel kept its tab bar, its toolbar and its menu. That is
/// where the control that zooms back out lives. Naming the panel instead would
/// strip all of it and leave the user with no way back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Zoomed {
    /// A tab group, rendered whole through its own [`TabGroupRenderer`].
    Group(NodeId),
}

/// One dock: its own layout tree plus the open/size/collapsible state.
struct DockRegion {
    tree: PaneTree,
    dock: Dock,
}

/// A cached container entity together with the subscription that carries its
/// intents back here. Kept as one value so dropping the cache entry drops the
/// subscription with it.
struct Cached<T> {
    entity: Entity<T>,
    _subscription: Subscription,
}

/// A split's cached `ResizableState`, plus the child order that state's panel
/// list currently mirrors.
///
/// The order is what makes a reconcile index-precise. `ResizableState` keeps
/// the authoritative size on `panels[ix]` and only ever consults the tree's
/// size as an initial value, so appending and truncating at the tail —
/// which is all `sync_panels_count` can do — leaves the survivors of a
/// non-tail removal wearing their predecessors' widths.
struct CachedSplit {
    entity: Entity<ResizableState>,
    children: Vec<NodeId>,
    /// The tree sizes the state last adopted. A reconcile that finds them
    /// unchanged leaves the state alone: re-adopting a `None` un-pins the
    /// measurement the layout pass resolved it to, and the split re-flexes
    /// although the edit never touched it.
    sizes: Vec<Option<Pixels>>,
    _subscription: Subscription,
}

/// The main area of the dock.
///
/// It owns one [`PaneTree`] per region and the entity cache that mirrors
/// them. Nothing else turns a tree edit into live entities.
pub struct DockArea {
    id: SharedString,
    version: Option<usize>,
    bounds: Bounds<Pixels>,
    this: WeakEntity<Self>,

    center: PaneTree,
    docks: HashMap<DockPlacement, DockRegion>,

    groups: HashMap<NodeId, Cached<TabGroup>>,
    splits: HashMap<NodeId, CachedSplit>,
    panels: HashMap<PanelId, Arc<dyn PanelView>>,

    locked: bool,
    zoomed: Option<Zoomed>,
    focus_handle: FocusHandle,
    renderer: Rc<dyn DockAreaRenderer>,
}

impl DockArea {
    /// An empty area that draws nothing but its panels.
    ///
    /// `id` names the area for the host's own persistence; `version` is
    /// written into [`Self::dump`] and read back by [`Self::load`], for a host
    /// that wants to reject or migrate a layout an older build wrote.
    ///
    /// Install appearance with [`Self::with_renderer`].
    pub fn new(
        id: impl Into<SharedString>,
        version: Option<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        PanelRegistry::init(cx);

        Self {
            id: id.into(),
            version,
            bounds: Bounds::default(),
            this: cx.weak_entity(),
            center: PaneTree::new(RootKind::Split),
            docks: HashMap::new(),
            groups: HashMap::new(),
            splits: HashMap::new(),
            panels: HashMap::new(),
            locked: false,
            zoomed: None,
            focus_handle: cx.focus_handle(),
            renderer: Rc::new(BareDockArea),
        }
    }

    /// Install the appearance for this area and everything under it: the
    /// renderer also supplies the [`TabGroupRenderer`] every group it builds
    /// will use.
    pub fn with_renderer(mut self, renderer: Rc<dyn DockAreaRenderer>) -> Self {
        self.renderer = renderer;
        self
    }

    pub fn id(&self) -> SharedString {
        self.id.clone()
    }

    pub fn version(&self) -> Option<usize> {
        self.version
    }

    /// Change the schema version a later [`dump`](Self::dump) writes.
    ///
    /// Set at construction for an area whose layout never changes shape. A
    /// host that installs one of several preset layouts into the same area
    /// picks the version with the preset, which is after the area exists.
    pub fn set_version(&mut self, version: Option<usize>, cx: &mut Context<Self>) {
        self.version = version;
        cx.notify();
    }

    /// The area's own bounds, recorded each frame. Dock resizing measures
    /// against it.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    /// The tree for one region, or `None` for a dock that does not exist.
    ///
    /// The `Option` is in the signature rather than hidden behind a panic
    /// because a dock is genuinely optional — it is `Option<DockState>` in the
    /// persisted schema — and there is no borrowable empty tree to hand back
    /// for one that is absent.
    pub fn layout(&self, placement: DockPlacement) -> Option<&PaneTree> {
        match placement {
            DockPlacement::Center => Some(&self.center),
            _ => self.docks.get(&placement).map(|pane| &pane.tree),
        }
    }

    /// The live view for a panel, if the dock still holds it.
    pub fn panel(&self, panel: PanelId) -> Option<&Arc<dyn PanelView>> {
        self.panels.get(&panel)
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Whether a region currently holds no visible panel.
    ///
    /// This is the question the old `DockItem::is_empty` answered, and it is
    /// the same one [`Self::is_node_visible`] answers per container: a region
    /// is empty when nothing in it would be drawn. A region that does not
    /// exist — a dock that was never installed — is empty too.
    pub fn is_empty(&self, placement: DockPlacement, cx: &App) -> bool {
        self.layout(placement)
            .is_none_or(|tree| !self.is_node_visible(tree.root(), cx))
    }

    /// Lock the layout against rearranging. Resizing stays available.
    pub fn set_locked(&mut self, locked: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked == locked {
            return;
        }
        self.locked = locked;
        // The lock is one of the facts every group is told, so it only takes
        // effect once the groups have been re-told.
        self.reconcile(window, cx);
    }
}

/// Installing layouts: the center region and the three docks.
impl DockArea {
    /// Replace the center region with a described layout. Whatever was there
    /// leaves the dock, so its panels are told [`Panel::on_removed`].
    pub fn set_center(&mut self, layout: DockLayout, window: &mut Window, cx: &mut Context<Self>) {
        let (tree, panels) = PaneTree::from_layout(layout, RootKind::Split);
        self.center = tree;
        self.panels.extend(panels);
        self.reconcile(window, cx);
        cx.emit(DockEvent::LayoutChanged);
    }

    /// Replace one dock with a described layout, creating the dock if the area
    /// does not have one there yet. A new dock keeps the size and open state of
    /// the one it replaces, so re-filling a dock does not resize it.
    ///
    /// [`DockPlacement::Center`] defers to [`Self::set_center`]: the center is
    /// not a dock and has no size or open state of its own.
    pub fn set_dock(
        &mut self,
        placement: DockPlacement,
        layout: DockLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if placement == DockPlacement::Center {
            return self.set_center(layout, window, cx);
        }

        let (tree, panels) = PaneTree::from_layout(layout, RootKind::Any);
        let dock = self
            .docks
            .get(&placement)
            .map(|pane| pane.dock)
            .unwrap_or_else(|| Dock::new(PANEL_MIN_SIZE * 2.));
        self.docks.insert(placement, DockRegion { tree, dock });
        self.panels.extend(panels);
        self.reconcile(window, cx);
        cx.emit(DockEvent::LayoutChanged);
    }

    /// Take a dock away entirely, panels and all. Distinct from
    /// [`Self::toggle_dock`], which only takes it off screen.
    pub fn remove_dock(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.docks.remove(&placement).is_none() {
            return;
        }
        // The dock's panels leave the dock entirely, so they are told, unlike
        // the panels of a dock that merely closed.
        self.reconcile(window, cx);
        cx.emit(DockEvent::LayoutChanged);
    }

    pub fn has_dock(&self, placement: DockPlacement) -> bool {
        self.docks.contains_key(&placement)
    }

    /// Whether a dock is on screen. A dock the area does not have is never
    /// open, so this answers the question a caller usually means without a
    /// preceding [`Self::has_dock`].
    pub fn is_dock_open(&self, placement: DockPlacement) -> bool {
        self.docks
            .get(&placement)
            .is_some_and(|pane| pane.dock.is_open())
    }

    /// Open a closed dock or close an open one. A dock that is not
    /// collapsible refuses to close; there is nothing to refuse when opening.
    pub fn toggle_dock(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.docks.get_mut(&placement) else {
            return;
        };
        if !pane.dock.is_collapsible() && pane.dock.is_open() {
            return;
        }
        let open = pane.dock.is_open();
        pane.dock.set_open(!open);
        // A closed dock takes its displayed panel off screen, which the
        // active-state contract counts as no panel being displayed — that is
        // what `TabGroupConstraints::collapsed` carries.
        self.reconcile(window, cx);
        cx.emit(DockEvent::LayoutChanged);
    }

    /// Whether a dock may be collapsed at all. A skin drawing a collapse
    /// affordance in a tab bar reads this to decide whether to offer one.
    pub fn is_dock_collapsible(&self, placement: DockPlacement) -> bool {
        self.docks
            .get(&placement)
            .is_some_and(|pane| pane.dock.is_collapsible())
    }

    pub fn set_dock_collapsible(
        &mut self,
        placement: DockPlacement,
        collapsible: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane) = self.docks.get_mut(&placement) {
            pane.dock.set_collapsible(collapsible);
            cx.notify();
        }
    }

    /// The dock's size along its axis.
    pub fn dock_size(&self, placement: DockPlacement) -> Option<Pixels> {
        self.docks.get(&placement).map(|pane| pane.dock.size())
    }

    pub fn set_dock_size(
        &mut self,
        placement: DockPlacement,
        size: Pixels,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane) = self.docks.get_mut(&placement) {
            let previous = pane.dock.size();
            pane.dock.set_size(size);
            if pane.dock.size() == previous {
                return;
            }
            cx.notify();
            cx.emit(DockEvent::LayoutChanged);
        }
    }
}

/// Editing the layout.
impl DockArea {
    /// Add a panel to a region, merging it into the first tab group there,
    /// or starting a group when the region is empty.
    pub fn add_panel<P: Panel>(
        &mut self,
        panel: Entity<P>,
        placement: DockPlacement,
        size: Option<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = PanelId::from(panel.entity_id());
        self.add_panel_inner(id, Arc::new(panel), placement, size, window, cx);
    }

    /// Add an already-wrapped panel handle to a region.
    ///
    /// The companion to [`Self::add_panel`], for a layer that hands base its
    /// own concrete handle — see [`PanelView::as_any`] — rather than a bare
    /// entity. The id comes from [`PanelView::panel_id`], which is the only
    /// place it can come from once the entity is behind the handle.
    pub fn add_panel_view(
        &mut self,
        panel: Arc<dyn PanelView>,
        placement: DockPlacement,
        size: Option<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = panel.panel_id(cx);
        self.add_panel_inner(id, panel, placement, size, window, cx);
    }

    fn add_panel_inner(
        &mut self,
        id: PanelId,
        panel: Arc<dyn PanelView>,
        placement: DockPlacement,
        size: Option<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The registration is written before the target is resolved, because
        // both want `&mut self`, so an add that finds nowhere to put the panel
        // has to undo it. *Undo*, not remove: adding a panel the dock already
        // holds is a legitimate call — a host re-placing one it owns — and
        // dropping its view would strand it in a tree with no entity, which is
        // what `reconcile`'s `views_of` asserts against.
        let previous = self.panels.insert(id, panel);

        // A dock is created to hold the panel; `size` seeds it.
        if placement != DockPlacement::Center && !self.docks.contains_key(&placement) {
            self.docks.insert(
                placement,
                DockRegion {
                    tree: PaneTree::new(RootKind::Any),
                    dock: Dock::new(size.unwrap_or(PANEL_MIN_SIZE * 2.)),
                },
            );
        }

        let Some(tree) = self.tree_mut(placement) else {
            self.restore_registration(id, previous);
            return;
        };
        let target = match first_tab_group(tree.root()) {
            Some(node) => InsertTarget::Tabs {
                node,
                ix: None,
                activate: true,
            },
            // An empty region has no container to merge into, so the panel
            // makes one beside the root. `normalize` then removes the emptied
            // root and, for a dock, collapses the wrapper away again.
            None => InsertTarget::Split {
                node: tree.root().id(),
                placement: Placement::Right,
                size,
            },
        };
        let result = tree.insert_panel(id, target);
        if !result.changed() {
            // Nothing took the panel, so a newly registered one must not
            // linger in the view map and be told `on_removed` by the next
            // reconcile.
            self.restore_registration(id, previous);
            return;
        }
        self.commit(result, window, cx);
    }

    /// Put the view map back the way an add found it, for one that placed
    /// nothing. `previous` is what [`HashMap::insert`] handed back.
    fn restore_registration(&mut self, id: PanelId, previous: Option<Arc<dyn PanelView>>) {
        match previous {
            Some(view) => self.panels.insert(id, view),
            None => self.panels.remove(&id),
        };
    }

    /// Remove a panel from wherever it lives, telling it that it was removed.
    pub fn remove_panel<P: Panel>(
        &mut self,
        panel: Entity<P>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remove_panel_id(PanelId::from(panel.entity_id()), window, cx);
    }

    /// Move a panel to a new home. The panel never leaves the dock, so it is
    /// never told it was removed.
    pub fn move_panel(
        &mut self,
        panel: PanelId,
        target: InsertTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A panel this area does not own (e.g. dropped from a nested dock) has
        // no backing entity here; inserting it would strand a ghost tab.
        if self.panel(panel).is_none() {
            return;
        }
        let Some(destination) = self.placement_of_node(target_node(&target)) else {
            return;
        };
        let source = self.placement_of_panel(panel);

        // A split target divides an existing slot, so the tree needs real
        // pixels to divide.
        if matches!(target, InsertTarget::Split { .. }) {
            self.adopt_measured_sizes(destination, cx);
        }

        // Read what the source group last told the panel *before* the edit,
        // so the destination can be seeded with it. Without this a panel
        // dragged between groups while displayed is told `true` twice.
        let was_active = self
            .layout(source.unwrap_or(destination))
            .and_then(|tree| tree.find_panel_node(panel))
            .and_then(|node| self.groups.get(&node))
            .and_then(|cached| cached.entity.read(cx).last_notified_active(panel));

        let changed = match source {
            Some(source) if source == destination => {
                let Some(tree) = self.tree_mut(destination) else {
                    return;
                };
                tree.move_panel(panel, target).changed()
            }
            source => {
                // Across trees a move is a detach plus an insert. The detach's
                // `removed_panels` is deliberately dropped on the floor: the
                // panel is still in the dock, so it must not hear `on_removed`.
                //
                // Both halves are committed on, not just the insert. A target
                // whose node kind does not match the insert is a silent no-op
                // in `apply_insert`, and committing on the insert alone would
                // early-return with the panel already gone from the source —
                // stranded in `self.panels`, belonging to no tree, for the
                // next reconcile to prune and destroy.
                let detached = source
                    .and_then(|source| self.tree_mut(source))
                    .is_some_and(|tree| tree.remove_panel(panel).changed());
                let Some(tree) = self.tree_mut(destination) else {
                    return;
                };
                let inserted = tree.insert_panel(panel, target).changed();
                detached || inserted
            }
        };

        self.commit_changed(changed, window, cx);

        if let Some(active) = was_active {
            if let Some(cached) = self
                .layout(destination)
                .and_then(|tree| tree.find_panel_node(panel))
                .and_then(|node| self.groups.get(&node))
            {
                let group = cached.entity.clone();
                group.update(cx, |group, _| group.seed_active(panel, active));
            }
        }
    }

    /// Display `panel` in the tab group that holds it, wherever that is.
    ///
    /// A host that is handed a file already open, or restores the tab a
    /// layout recorded as active, wants that tab shown where it sits.
    /// [`Self::move_panel`] with `activate` would also move it, and the group
    /// entities are the dock's own, so this is the one way to select a panel
    /// by identity. Nothing happens for a panel the dock does not hold, or
    /// one already displayed.
    pub fn select_panel(&mut self, panel: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(placement) = self.placement_of_panel(panel) else {
            return;
        };
        let Some(tree) = self.tree_mut(placement) else {
            return;
        };
        let Some(node) = tree.find_panel_node(panel) else {
            return;
        };
        let ix = tree.find_node(node).and_then(|node| match node.kind() {
            PaneRef::Tabs { panels, .. } => panels.iter().position(|held| *held == panel),
            PaneRef::Split { .. } => None,
        });
        let Some(ix) = ix else {
            return;
        };
        let result = tree.set_active(node, ix);
        self.commit(result, window, cx);
    }

    /// Replace the slot sizes of the split at `node` in place.
    pub fn set_split_sizes(
        &mut self,
        node: NodeId,
        sizes: Vec<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(placement) = self.placement_of_node(node) else {
            return;
        };
        let Some(tree) = self.tree_mut(placement) else {
            return;
        };
        let result = tree.set_sizes(node, sizes.into_iter().map(Some).collect());
        self.commit(result, window, cx);
    }

    /// Put `panel` in a new tab group beside `node`.
    pub fn split_at(
        &mut self,
        node: NodeId,
        panel: PanelId,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(region) = self.placement_of_node(node) else {
            return;
        };
        self.adopt_measured_sizes(region, cx);
        let Some(tree) = self.tree_mut(region) else {
            return;
        };
        let result = tree.split(node, panel, placement, None);
        self.commit(result, window, cx);
    }

    fn remove_panel_id(&mut self, panel: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(region) = self.placement_of_panel(panel) else {
            return;
        };
        let focused = window.focused(cx);
        let had_focus = self
            .panel(panel)
            .is_some_and(|view| view.focus_handle(cx).contains_focused(window, cx));
        // Prefer the same group, then the groups following it in layout order,
        // then preceding groups from nearest to farthest. Preserve this order
        // before normalization removes an emptied group or collapses its split.
        let mut candidates = Vec::new();
        if had_focus && let Some(tree) = self.layout(region) {
            tree.root().walk(&mut |node| {
                if matches!(node.kind(), PaneRef::Tabs { .. }) {
                    candidates.push(node.id());
                }
            });
            if let Some(ix) = tree
                .find_panel_node(panel)
                .and_then(|node| candidates.iter().position(|candidate| *candidate == node))
            {
                candidates = candidates[ix..]
                    .iter()
                    .chain(candidates[..ix].iter().rev())
                    .copied()
                    .collect();
            }
        }
        let Some(tree) = self.tree_mut(region) else {
            return;
        };
        let result = tree.remove_panel(panel);
        self.commit(result, window, cx);

        // on_removed may deliberately focus another control (or clear focus).
        // Automatic handoff is only the fallback when the callback left it alone.
        if !had_focus || window.focused(cx) != focused {
            return;
        }
        let next = candidates
            .into_iter()
            .find_map(|node| self.focus_target_in_group(node, cx))
            .or_else(|| {
                [
                    DockPlacement::Center,
                    DockPlacement::Left,
                    DockPlacement::Right,
                    DockPlacement::Bottom,
                ]
                .into_iter()
                .filter(|placement| *placement != region)
                .filter(|placement| {
                    *placement == DockPlacement::Center
                        || self
                            .docks
                            .get(placement)
                            .is_some_and(|pane| pane.dock.is_open())
                })
                .find_map(|placement| {
                    self.layout(placement)
                        .and_then(|tree| self.focus_target_in_node(tree.root(), cx))
                })
            });
        if let Some(next) = next {
            next.focus_handle(cx).focus(window, cx);
        }
    }

    fn focus_target_in_group(&self, node: NodeId, cx: &App) -> Option<Arc<dyn PanelView>> {
        if self.zoomed.is_some_and(|zoom| zoom != Zoomed::Group(node)) {
            return None;
        }
        let group = self.groups.get(&node)?.entity.read(cx);
        if group.is_collapsed() {
            return None;
        }
        group.active_panel(cx)
    }

    fn focus_target_in_node(&self, node: &PaneNode, cx: &App) -> Option<Arc<dyn PanelView>> {
        match node.kind() {
            PaneRef::Tabs { .. } => self.focus_target_in_group(node.id(), cx),
            PaneRef::Split { children, .. } => children
                .iter()
                .find_map(|child| self.focus_target_in_node(child, cx)),
        }
    }
}

/// Zooming.
///
/// There is no `set_zoomed_in(panel)` here. A zoom is a container's own act:
/// only the container knows whether its displayed panel is zoomable, and only
/// the container can tell that panel it was zoomed. So the way in is
/// [`TabGroupContext::toggle_zoom`](super::TabGroupContext::toggle_zoom) — a
/// skin has one wherever it draws a zoom control — or
/// [`Self::set_zoomed_in`] by node, which delegates to the same place. The
/// area then installs the container that reported it.
impl DockArea {
    /// Zoom the tab group at `node` in, as if its own zoom control had been
    /// used.
    ///
    /// Nothing happens for a node that is not a live tab group, or when the
    /// group refuses — the group is the one that knows.
    pub fn set_zoomed_in(&mut self, node: NodeId, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(Some(Zoomed::Group(node)), window, cx);
    }

    /// Clear the zoom, putting the zoomed container's own flag back with it.
    ///
    /// A container toggles its zoom itself and only reports it, so an area
    /// that dropped the view without telling the container would leave it
    /// believing it still fills the dock — and a zoomed group refuses drops.
    pub fn set_zoomed_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(None, window, cx);
    }

    pub fn is_zoomed(&self) -> bool {
        self.zoomed.is_some()
    }

    /// The tab group filling the area, if a group is what is zoomed.
    pub fn zoomed_group(&self) -> Option<NodeId> {
        match self.zoomed {
            Some(Zoomed::Group(node)) => Some(node),
            _ => None,
        }
    }

    /// The one place `self.zoomed` is written.
    ///
    /// Every write drives the container's own flag through the same call, and
    /// the new target is recorded only if that container accepted it. So the
    /// area cannot show a container the container does not think is zoomed,
    /// and cannot leave a container flagged zoomed while showing something
    /// else — the split state a group would carry as a permanent lock.
    fn set_zoom(&mut self, zoomed: Option<Zoomed>, window: &mut Window, cx: &mut Context<Self>) {
        if self.zoomed == zoomed {
            return;
        }

        if let Some(previous) = self.zoomed {
            self.drive_zoom(previous, false, window, cx);
        }
        let accepted = match zoomed {
            Some(next) => self.drive_zoom(next, true, window, cx).then_some(next),
            None => None,
        };
        self.zoomed = accepted;
        cx.notify();
    }

    /// Ask one container to zoom in or out, and report whether it now agrees.
    ///
    /// A container that has already been left out of the cache — its node is
    /// gone from the tree — has nothing to say and nothing to put back, so it
    /// answers `false` and a zoom naming it is never installed.
    fn drive_zoom(
        &mut self,
        zoomed: Zoomed,
        zoom_in: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match zoomed {
            Zoomed::Group(node) => {
                let Some(group) = self.groups.get(&node).map(|cached| cached.entity.clone()) else {
                    return false;
                };
                group.update(cx, |group, cx| {
                    group.set_zoomed(zoom_in, window, cx);
                    group.is_zoomed() == zoom_in
                })
            }
        }
    }

    /// The container that fills the area when something is zoomed.
    ///
    /// The container, not the panel inside it: this is what keeps a zoomed
    /// group's tab bar on screen.
    fn zoomed_view(&self) -> Option<AnyView> {
        match self.zoomed? {
            Zoomed::Group(node) => Some(self.groups.get(&node)?.entity.clone().into()),
        }
    }
}

/// Persistence.
impl DockArea {
    /// Read a persisted layout, rebuilding every panel through
    /// [`PanelRegistry`]. A panel this build does not know about becomes a
    /// placeholder that carries the original [`PanelState`] forward, so the
    /// next save does not erase it.
    pub fn load(
        &mut self,
        state: DockAreaState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.version = state.version;
        self.zoomed = None;
        // Nothing in the old layout survives a load, so the caches are
        // emptied rather than reconciled: every node id in the new trees is
        // freshly minted and would miss the old cache anyway.
        self.groups.clear();
        self.splits.clear();
        self.docks.clear();
        // `self.panels` is deliberately *not* cleared: leaving the outgoing
        // panels in it lets `reconcile` prune them, which is what tells them
        // they were removed.

        let dock_area = self.this.clone();
        let renderer = self.renderer.clone();
        let mut built = Vec::new();
        self.center = {
            let mut builder = RegistryPanelBuilder {
                dock_area: dock_area.clone(),
                renderer: renderer.clone(),
                built: &mut built,
                window,
                cx,
            };
            PaneTree::from_state(&state.center, RootKind::Split, &mut builder)
        };

        for dock_state in [state.left_dock, state.right_dock, state.bottom_dock]
            .into_iter()
            .flatten()
        {
            let tree = {
                let mut builder = RegistryPanelBuilder {
                    dock_area: dock_area.clone(),
                    renderer: renderer.clone(),
                    built: &mut built,
                    window,
                    cx,
                };
                PaneTree::from_state(dock_state.panel(), RootKind::Any, &mut builder)
            };
            let mut dock = Dock::new(dock_state.size());
            dock.set_open(dock_state.open());
            self.docks
                .insert(dock_state.placement(), DockRegion { tree, dock });
        }

        self.panels.extend(built);
        self.reconcile(window, cx);
        cx.emit(DockEvent::LayoutChanged);
        Ok(())
    }

    /// Write the layout out.
    ///
    /// Slot sizes are resolved to concrete pixels first. The tree represents
    /// an unconstrained slot as `None` and the writer emits `0.0` for it,
    /// which *this* reader maps back to `None` — but an older build has no
    /// notion of the sentinel and would construct a real zero-pixel panel from
    /// it. Preference order is the split's measured size, then the tree's own
    /// size, then [`PANEL_MIN_SIZE`] for a slot nothing has ever measured.
    ///
    /// The measurement wins because the tree does not track every change to
    /// it. `ResizableState` only emits `Resized` from a finished drag, so that
    /// is all the subscription in [`Self::split_entity`] writes back;
    /// `adjust_to_container_size` rescales every slot silently on each window
    /// resize, insert and remove. Preferring the tree would persist load-time
    /// or last-drag pixels after a window resize, and worse, mix them: a slot
    /// left `None` by a later insert would be filled from the current
    /// measurement while its untouched siblings kept file-era numbers, so the
    /// written ratio would match neither the file nor the screen. Reading the
    /// measurement for every slot of a split writes one internally consistent
    /// set, which is what the old `StackPanel::dump` did.
    pub fn dump(&self, cx: &App) -> DockAreaState {
        let source = LivePanels::new(&self.panels, cx);

        DockAreaState {
            version: self.version,
            center: self.resolved_tree(&self.center, cx).to_state(&source),
            left_dock: self.dump_dock(DockPlacement::Left, &source, cx),
            right_dock: self.dump_dock(DockPlacement::Right, &source, cx),
            bottom_dock: self.dump_dock(DockPlacement::Bottom, &source, cx),
        }
    }

    fn dump_dock(
        &self,
        placement: DockPlacement,
        source: &LivePanels<'_>,
        cx: &App,
    ) -> Option<DockState> {
        let pane = self.docks.get(&placement)?;
        Some(DockState::new(
            self.resolved_tree(&pane.tree, cx).to_state(source),
            placement,
            pane.dock.size(),
            pane.dock.is_open(),
        ))
    }

    fn resolved_tree(&self, tree: &PaneTree, cx: &App) -> PaneTree {
        let mut tree = tree.clone();
        self.resolve_sizes(tree.root_mut(), cx);
        tree
    }

    fn resolve_sizes(&self, node: &mut PaneNode, cx: &App) {
        let measured = self
            .splits
            .get(&node.id())
            .map(|cached| cached.entity.read(cx).sizes().clone())
            .unwrap_or_default();

        let NodeKind::Split {
            children, sizes, ..
        } = node.kind_mut()
        else {
            return;
        };

        for (ix, size) in sizes.iter_mut().enumerate() {
            // A slot already holding zero is exactly as unsafe as an absent
            // one: it is the same byte in the file and the same zero-pixel
            // panel in an older build. So both the measurement and the stored
            // value have to clear zero before they can be written.
            let on_screen = measured.get(ix).copied().filter(|size| *size > px(0.));
            let stored = (*size).filter(|size| *size > px(0.));
            *size = Some(on_screen.or(stored).unwrap_or(PANEL_MIN_SIZE));
        }

        for child in children.iter_mut() {
            self.resolve_sizes(child, cx);
        }
    }
}

/// Reconciliation.
impl DockArea {
    /// Apply one edit: bring the entity cache back in line and say so.
    ///
    /// `on_removed` is not fired from `EditResult::removed_panels` here.
    /// [`Self::reconcile`] fires it instead, from the panels it prunes: that
    /// is the same set for a plain removal, and it also covers the panels a
    /// wholesale `set_center`, `set_dock`, `remove_dock` or `load` displaces,
    /// which no `EditResult` describes at all. It is also the safer default —
    /// a caller cannot forget a list it does not pass.
    fn commit(&mut self, result: EditResult, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_changed(result.changed(), window, cx);
    }

    /// [`Self::commit`] for an edit that took more than one `EditResult`.
    fn commit_changed(&mut self, changed: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !changed {
            return;
        }

        self.reconcile(window, cx);
        cx.emit(DockEvent::LayoutChanged);
    }

    /// Bring the entity cache in line with the trees.
    ///
    /// Because `NodeId` survives every edit and every normalization rule, a
    /// steady-state pass creates and drops nothing; only genuinely new or dead
    /// containers churn. That is what keeps a drag from resetting the state of
    /// panels it did not touch.
    fn reconcile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Planned first, applied second: the plan borrows the trees, and
        // applying it needs `&mut self` to fill the caches.
        let mut plans = Vec::new();
        plan_tree(&self.center, false, self.locked, &mut plans);
        for pane in self.docks.values() {
            plan_tree(&pane.tree, !pane.dock.is_open(), self.locked, &mut plans);
        }

        // Sets rather than vectors: these are membership tests, run once per
        // cached entity and once per live panel, and a drag reaches this path
        // on every mouse move.
        let mut live_nodes = HashSet::with_capacity(plans.len());
        let mut live_panels: HashSet<PanelId> = HashSet::new();

        for plan in plans {
            live_nodes.insert(plan.node());
            match plan {
                ContainerPlan::Split {
                    node,
                    axis,
                    children,
                    sizes,
                } => {
                    let state = self.split_entity(node, cx);
                    let (previous, adopted) = self
                        .splits
                        .get(&node)
                        .map(|cached| (cached.children.clone(), cached.sizes.clone()))
                        .unwrap_or_default();
                    // Only a split the edit changed is handed anything. The
                    // state of every other split may legitimately disagree
                    // with its tree — a window resize rescales it silently —
                    // and pushing the tree's sizes back would move slots the
                    // edit never named. For a slot the tree leaves `None`, it
                    // would also un-pin the measurement the first layout pass
                    // resolved it to, and the whole split re-flexes.
                    if previous != children || adopted != sizes {
                        state.update(cx, |state, cx| {
                            sync_split_panels(state, &previous, &children, &sizes, cx);
                            state.sync_panels_count(axis, children.len(), cx);
                            // The tree is authoritative on how space divides,
                            // so its sizes land last — `insert_panel`
                            // renormalizes everything it touches, which would
                            // otherwise undo the share an edit just decided.
                            //
                            // What the tree decides is the *share*, not the
                            // pixel count. The file holds absolute pixels
                            // measured in whatever window last saved it, so
                            // restoring into a different one leaves their
                            // total off the container — and
                            // `adjust_to_container_size` rescales the state to
                            // the container on the very next pass. Adopting
                            // the raw numbers would re-assert the stale total,
                            // so hand over the share instead and both sides
                            // already agree.
                            state.adopt_sizes(&scale_sizes_to(state.container_size(), &sizes), cx);
                        });
                    }
                    if let Some(cached) = self.splits.get_mut(&node) {
                        cached.children = children;
                        cached.sizes = sizes;
                    }
                }
                ContainerPlan::Group {
                    node,
                    panels,
                    active_ix,
                    constraints,
                } => {
                    live_panels.extend(panels.iter().copied());
                    let views = self.views_of(&panels);
                    let group = self.group_entity(node, window, cx);
                    group.update(cx, |group, cx| {
                        // Every group must be told this. A group nobody has
                        // constrained stays `sealed()` and silently declines
                        // drags, drops and closes.
                        group.set_constraints(constraints, window, cx);
                        group.sync_from_tree(views, active_ix, window, cx);
                    });
                }
            }
        }

        self.groups.retain(|node, _| live_nodes.contains(node));
        self.splits.retain(|node, _| live_nodes.contains(node));

        let departed: Vec<Arc<dyn PanelView>> = self
            .panels
            .iter()
            .filter(|(panel, _)| !live_panels.contains(panel))
            .map(|(_, view)| view.clone())
            .collect();
        self.panels.retain(|panel, _| live_panels.contains(panel));

        // A zoomed container fills the whole area, so one that has just left
        // the dock would otherwise keep filling it with nothing behind it.
        //
        // It is the container going away that ends the zoom, not a panel:
        // a group survives its displayed panel closing, and the next tab
        // takes over, still zoomed. The old `TabPanel::remove_panel` instead
        // emitted `ZoomOut` on every removal, which cleared the dock's zoom
        // even for a panel in some other tab panel entirely — and left the
        // zoomed `TabPanel` still flagged zoomed while the dock was not.
        let zoom_survives = match self.zoomed {
            Some(Zoomed::Group(node)) => self.groups.contains_key(&node),
            None => true,
        };
        if !zoom_survives {
            self.set_zoom(None, window, cx);
        }

        cx.notify();
        // Last, so a panel reacting to this sees a dock that already agrees
        // with its trees.
        for view in departed {
            view.on_removed(window, cx);
        }
    }

    /// The views for a group's panel ids, in tab order.
    fn views_of(&self, panels: &[PanelId]) -> Vec<Arc<dyn PanelView>> {
        debug_assert!(
            panels.iter().all(|panel| self.panels.contains_key(panel)),
            "every panel in a tree must have a live view; a missing one would \
             silently shift the group's active index"
        );
        panels
            .iter()
            .filter_map(|panel| self.panels.get(panel).cloned())
            .collect()
    }

    fn group_entity(
        &mut self,
        node: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TabGroup> {
        if let Some(cached) = self.groups.get(&node) {
            return cached.entity.clone();
        }

        let renderer = self.renderer.tab_group_renderer();
        let entity = cx.new(|cx| TabGroup::new(node, window, cx).with_renderer(renderer));
        let subscription = cx.subscribe_in(&entity, window, Self::on_tab_group_event);
        self.groups.insert(
            node,
            Cached {
                entity: entity.clone(),
                _subscription: subscription,
            },
        );
        entity
    }

    fn split_entity(&mut self, node: NodeId, cx: &mut Context<Self>) -> Entity<ResizableState> {
        if let Some(cached) = self.splits.get(&node) {
            return cached.entity.clone();
        }

        let entity = cx.new(|_| ResizableState::default());
        // A drag on a resize handle changes only the measured sizes. Writing
        // them straight back keeps the tree describing the layout the user
        // arranged, which is what a later insert or removal scales from and
        // what a region with no live split entity is dumped from.
        //
        // It does not make the tree authoritative on slot sizes generally, and
        // `dump` does not treat it as such: only a finished drag arrives here,
        // while `adjust_to_container_size` rewrites the measurements silently
        // on every window resize. See [`Self::dump`].
        let subscription =
            cx.subscribe(&entity, move |this, state, _: &ResizablePanelEvent, cx| {
                let sizes: Vec<Option<Pixels>> = state
                    .read(cx)
                    .sizes()
                    .iter()
                    .map(|size| Some(*size))
                    .collect();
                let Some(region) = this.placement_of_node(node) else {
                    return;
                };
                let Some(tree) = this.tree_mut(region) else {
                    return;
                };
                if tree.set_sizes(node, sizes.clone()).changed() {
                    cx.emit(DockEvent::LayoutChanged);
                }
                // The state is where these came from, so the next reconcile
                // has nothing to hand it.
                if let Some(cached) = this.splits.get_mut(&node) {
                    cached.sizes = sizes;
                }
            });
        self.splits.insert(
            node,
            CachedSplit {
                entity: entity.clone(),
                children: Vec::new(),
                sizes: Vec::new(),
                _subscription: subscription,
            },
        );
        entity
    }
}

/// Intents arriving from the containers.
impl DockArea {
    fn on_tab_group_event(
        &mut self,
        group: &Entity<TabGroup>,
        event: &TabGroupEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TabGroupEvent::Drop { panel, target, .. } => {
                self.move_panel(*panel, *target, window, cx)
            }
            TabGroupEvent::DragDrop { item, target } => cx.emit(DockEvent::DragDrop {
                item: item.clone(),
                target: *target,
            }),
            TabGroupEvent::ClosePanel { panel } => self.remove_panel_id(*panel, window, cx),
            TabGroupEvent::ActiveChanged { ix } => {
                let node = group.read(cx).node();
                let Some(region) = self.placement_of_node(node) else {
                    return;
                };
                let Some(tree) = self.tree_mut(region) else {
                    return;
                };
                let result = tree.set_active(node, *ix);
                self.commit(result, window, cx);
            }
            TabGroupEvent::ZoomIn => {
                let node = group.read(cx).node();
                self.set_zoom(Some(Zoomed::Group(node)), window, cx);
            }
            // Only the group that is actually on screen can give the dock
            // back. A group told to zoom out to make room for another one
            // reports it too, and that report must not undo the zoom that
            // replaced it.
            TabGroupEvent::ZoomOut => {
                let node = group.read(cx).node();
                if self.zoomed == Some(Zoomed::Group(node)) {
                    self.set_zoom(None, window, cx);
                }
            }
        }
    }
}

/// Region lookup.
impl DockArea {
    /// Feed every split's measured size back into the tree before an edit
    /// that has to divide space.
    ///
    /// Done here rather than continuously: the tree is the record of what the
    /// user arranged, and rewriting it on every layout pass would let a
    /// transient window size become the stored layout.
    fn adopt_measured_sizes(&mut self, placement: DockPlacement, cx: &App) {
        let measured: HashMap<NodeId, Vec<Pixels>> = self
            .splits
            .iter()
            // Only splits that have actually been laid out. A split created
            // earlier in this same edit has a zero container and its `sizes`
            // are whatever `insert_panel` left behind — adopting those would
            // freeze a placeholder ratio into the tree, and every later edit
            // would divide space according to it.
            .filter(|(_, cached)| cached.entity.read(cx).container_size() > Pixels::ZERO)
            .map(|(node, cached)| (*node, cached.entity.read(cx).sizes().clone()))
            .collect();

        if let Some(tree) = self.tree_mut(placement) {
            tree.adopt_measured_sizes(&measured);
        }
    }

    fn tree_mut(&mut self, placement: DockPlacement) -> Option<&mut PaneTree> {
        match placement {
            DockPlacement::Center => Some(&mut self.center),
            _ => self.docks.get_mut(&placement).map(|pane| &mut pane.tree),
        }
    }

    /// Which region a container belongs to. Unambiguous because `NodeId`s are
    /// allocated globally rather than per tree.
    fn placement_of_node(&self, node: NodeId) -> Option<DockPlacement> {
        if self.center.find_node(node).is_some() {
            return Some(DockPlacement::Center);
        }
        self.docks
            .iter()
            .find(|(_, pane)| pane.tree.find_node(node).is_some())
            .map(|(placement, _)| *placement)
    }

    fn placement_of_panel(&self, panel: PanelId) -> Option<DockPlacement> {
        if self.center.find_panel_node(panel).is_some() {
            return Some(DockPlacement::Center);
        }
        self.docks
            .iter()
            .find(|(_, pane)| pane.tree.find_panel_node(panel).is_some())
            .map(|(placement, _)| *placement)
    }

    /// Resize one dock from a pointer position, clamped so neither this dock
    /// nor the one opposite is squeezed below its minimum.
    ///
    /// A collapsible bottom dock is the exception: it follows the pointer
    /// below the minimum down to its closed strip, so closing it by drag is
    /// one continuous motion. That size is only shown, and
    /// [`Self::end_dock_resize`] settles it on release.
    fn resize_dock(
        &mut self,
        placement: DockPlacement,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let opposite = match placement {
            DockPlacement::Left => self.dock_size(DockPlacement::Right),
            DockPlacement::Right => self.dock_size(DockPlacement::Left),
            _ => None,
        };
        // Sizing reads the pointer against the edge the dock physically sits on.
        let sizing = DockSizing::new(placement.physical(crate::LayoutDirection::global(cx)))
            .with_area_bounds(self.bounds)
            .with_opposite_dock_size(opposite.unwrap_or(px(0.)));
        let size = sizing
            .size_from_pointer(pointer)
            .min(sizing.clamp(Pixels::MAX));

        let strip = self.renderer.closed_dock_extent(placement);
        let Some(pane) = self.docks.get_mut(&placement) else {
            return;
        };
        let was_open = pane.dock.is_open();
        if placement == DockPlacement::Bottom && pane.dock.is_collapsible() && size < PANEL_MIN_SIZE
        {
            pane.dock.set_open(size > strip);
            pane.dock.set_live_size(Some(size.max(strip)));
        } else {
            pane.dock.set_open(true);
            pane.dock.set_live_size(None);
            pane.dock.set_size(size);
        }
        if pane.dock.is_open() != was_open {
            self.reconcile(window, cx);
        }
        cx.notify();
    }

    /// Settle a drag that ended below the minimum: nearer the closed strip it
    /// closes, nearer the minimum it opens at the minimum.
    fn end_dock_resize(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let strip = self.renderer.closed_dock_extent(placement);
        let Some(pane) = self.docks.get_mut(&placement) else {
            return;
        };
        let Some(size) = pane.dock.live_size() else {
            return;
        };
        pane.dock.set_live_size(None);

        let open = size >= (strip + PANEL_MIN_SIZE) / 2.;
        if open {
            pane.dock.set_size(PANEL_MIN_SIZE);
        }
        if pane.dock.is_open() != open {
            pane.dock.set_open(open);
            self.reconcile(window, cx);
        }
        cx.notify();
    }
}

/// Rendering.
impl DockArea {
    /// Lower one container to an element.
    fn render_node(&self, node: &PaneNode, window: &mut Window, cx: &mut App) -> AnyElement {
        match node.kind() {
            PaneRef::Split {
                axis,
                children,
                sizes,
            } => {
                let group = match axis {
                    Axis::Horizontal => h_resizable(("dock-split", node.id().as_u64())),
                    Axis::Vertical => v_resizable(("dock-split", node.id().as_u64())),
                };
                // A container whose every panel is hidden must not keep
                // occupying its slot. No renderer hook could supply this:
                // only the area can see the panels behind a node.
                let shown: Vec<bool> = children
                    .iter()
                    .map(|child| self.is_node_visible(child, cx))
                    .collect();
                // The slot that absorbs the leftover has to be one that is
                // actually drawn. A hidden slot renders nothing and grows
                // nothing, so making it the flexible one leaves every drawn
                // slot rigid and the split ends short of its container — the
                // empty strip this picks the *last shown* slot to avoid.
                let grows = shown.iter().rposition(|shown| *shown);
                let panels: Vec<_> = children
                    .iter()
                    .zip(sizes.iter())
                    .enumerate()
                    .map(|(ix, (child, size))| {
                        resizable_panel()
                            .visible(shown[ix])
                            .child(self.render_node(child, window, cx))
                            // `flex_none` is what makes the size stick.
                            // `ResizablePanel` sets `flex_grow: 1` on itself,
                            // so a slot given a size would otherwise treat it
                            // as a flex-basis and still absorb an equal share
                            // of the leftover — a 200px sidebar rendering
                            // 1075px wide in a 1950px split.
                            // The growth slot absorbs container growth. A
                            // drag records every measured size as pixels; if
                            // all of them became `flex_none`, a later viewport
                            // resize would leave an empty strip after the
                            // split instead of keeping the Dock filled.
                            .when_some(*size, |panel, size| {
                                panel
                                    .size(size)
                                    .when(Some(ix) != grows, |panel| panel.flex_none())
                            })
                    })
                    .collect();

                let group = group
                    .when_some(self.splits.get(&node.id()), |group, cached| {
                        group.with_state(&cached.entity)
                    })
                    .with_handle_appearance({
                        let renderer = self.renderer.clone();
                        Rc::new(move |handle, window, cx| {
                            renderer.render_split_handle(handle, window, cx)
                        })
                    })
                    .children(panels);

                self.renderer
                    .split_frame(node.id(), axis, window, cx)
                    // A split frame with no size collapses: base puts it
                    // between a `resizable_panel` and the resizable group, and
                    // between `center_frame` and the centre's root split, and
                    // neither parent sizes it. `size_full` and `flex_1` are
                    // belt and braces -- either alone passes every case I could
                    // construct, so this does not depend on which one wins in a
                    // given parent.
                    .size_full()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(group)
                    .into_any_element()
            }
            PaneRef::Tabs { .. } => match self.groups.get(&node.id()) {
                Some(cached) => cached.entity.clone().into_any_element(),
                None => Empty.into_any_element(),
            },
        }
    }

    /// Whether anything in this container is on screen.
    ///
    /// Mirrors the old `StackPanel::render`, which asked each slot's
    /// `TabPanel::visible` — "does this group hold any visible panel?" — and
    /// hid the slot when it did not.
    fn is_node_visible(&self, node: &PaneNode, cx: &App) -> bool {
        let panels = LivePanels::new(&self.panels, cx);
        match node.kind() {
            PaneRef::Split { children, .. } => {
                children.iter().any(|child| self.is_node_visible(child, cx))
            }
            PaneRef::Tabs { panels: ids, .. } => ids.iter().any(|panel| panels.is_visible(*panel)),
        }
    }

    fn render_dock(
        &self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let pane = self.docks.get(&placement)?;
        let dock = self.dock_context(placement, &pane.dock);

        // A closed dock keeps what the renderer asks for: by default nothing
        // at the sides and a strip at the bottom so its tab bar stays
        // clickable. Nothing is drawn
        // for a dock with no extent, and the renderer is not asked for chrome
        // nobody can see.
        let size = if dock.is_open() {
            dock.size()
        } else {
            self.renderer.closed_dock_extent(placement)
        };
        if size <= px(0.) {
            return Some(div().into_any_element());
        }

        let content = self.render_node(pane.tree.root(), window, cx);
        // The box is applied here rather than left to the renderer, and that is
        // the whole point of it being here. A dock's extent along its own axis
        // is not presentation -- it is what makes the dock a column beside the
        // centre instead of a block in the flow below it -- and a renderer that
        // did not know to state it produced a dock with no width, every pane
        // inside it shrunk to its content. `render_dock` on the renderer is a
        // chrome hook, so a renderer that draws nothing at all still gets a
        // dock that is the right shape.
        let chrome = self.renderer.render_dock(&dock, content, window, cx);
        Some(dock_frame(&dock, size).child(chrome).into_any_element())
    }

    fn dock_context(&self, placement: DockPlacement, dock: &Dock) -> DockContext {
        let area = self.this.clone();

        DockContext {
            placement,
            size: dock.live_size().unwrap_or(dock.size()),
            open: dock.is_open(),
            collapsible: dock.is_collapsible(),
            on_toggle: {
                let area = area.clone();
                Rc::new(move |window, cx| {
                    _ = area.update(cx, |area, cx| area.toggle_dock(placement, window, cx));
                })
            },
            on_resize: {
                let area = area.clone();
                Rc::new(move |pointer, window, cx| {
                    _ = area.update(cx, |area, cx| {
                        area.resize_dock(placement, pointer, window, cx)
                    });
                })
            },
            on_resize_end: Rc::new(move |window, cx| {
                _ = area.update(cx, |area, cx| area.end_dock_resize(placement, window, cx));
            }),
        }
    }
}

impl EventEmitter<DockEvent> for DockArea {}

impl Focusable for DockArea {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DockArea {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let area = cx.entity();
        let renderer = self.renderer.clone();

        renderer
            .frame(window, cx)
            .test_support()
            // Structure, applied after the hook and not inside it. A dock area
            // lays its left dock, centre and right dock out in a row; a frame
            // that is not one stacks them down the window instead, which is
            // what every renderer that is not `DockSkin` used to get, because
            // the row lived in `DockSkin`'s override of this hook and the trait
            // default is a bare `div`.
            .relative()
            .size_full()
            .overflow_hidden()
            // Left dock on the start edge: the right edge in RTL.
            .flex_row_dir(cx)
            .on_prepaint(move |bounds, _, cx| {
                area.update(cx, |area, _| area.bounds = bounds);
            })
            .track_focus(&self.focus_handle)
            .map(|frame| match self.zoomed_view() {
                Some(view) => frame.child(view),
                None => frame
                    .when_some(
                        self.render_dock(DockPlacement::Left, window, cx),
                        ParentElement::child,
                    )
                    .child(
                        renderer
                            .center_frame(window, cx)
                            // Same reason as the frame above: the centre is
                            // whatever the side docks leave, in a column with
                            // the bottom dock. Without this it is neither, and
                            // shrinks to its content.
                            .flex()
                            .flex_1()
                            .flex_col()
                            .overflow_hidden()
                            .child(self.render_node(self.center.root(), window, cx))
                            .when_some(
                                self.render_dock(DockPlacement::Bottom, window, cx),
                                ParentElement::child,
                            ),
                    )
                    .when_some(
                        self.render_dock(DockPlacement::Right, window, cx),
                        ParentElement::child,
                    ),
            })
    }
}

/// One container's worth of reconciliation work, snapshotted out of the tree.
enum ContainerPlan {
    Split {
        node: NodeId,
        axis: Axis,
        children: Vec<NodeId>,
        sizes: Vec<Option<Pixels>>,
    },
    Group {
        node: NodeId,
        panels: Vec<PanelId>,
        active_ix: usize,
        constraints: TabGroupConstraints,
    },
}

impl ContainerPlan {
    fn node(&self) -> NodeId {
        match self {
            Self::Split { node, .. } | Self::Group { node, .. } => *node,
        }
    }
}

/// Re-express slot sizes as shares of `container`, keeping their proportions.
///
/// Returns them unchanged unless every slot is constrained and both the
/// container and the recorded total are usable: an unconstrained slot is laid
/// out by flex and takes the leftover, so scaling only the constrained ones
/// would move a divider nothing asked to move.
fn scale_sizes_to(container: Pixels, sizes: &[Option<Pixels>]) -> Vec<Option<Pixels>> {
    let total: f32 = sizes.iter().flatten().map(|size| size.as_f32()).sum();
    if container <= px(0.) || total <= 0. || sizes.iter().any(Option::is_none) {
        return sizes.to_vec();
    }

    let scale = container.as_f32() / total;
    sizes
        .iter()
        .map(|size| size.map(|size| px(size.as_f32() * scale)))
        .collect()
}

/// Bring one split's `ResizableState` panel list from `previous` to `next`,
/// inserting and removing at the exact index rather than at the tail.
///
/// `ResizableState` treats the size handed to `insert_panel` as an initial
/// value only; from then on `panels[ix].size` is authoritative. So an
/// append-and-truncate sync leaves every survivor of a non-tail removal
/// wearing the width of whoever used to sit at its index — which is the most
/// ordinary edit in the dock, a drag that empties a group, and it is carried
/// into the save file by `resolve_sizes`.
fn sync_split_panels(
    state: &mut ResizableState,
    previous: &[NodeId],
    next: &[NodeId],
    sizes: &[Option<Pixels>],
    cx: &mut Context<ResizableState>,
) {
    let mut current = previous.to_vec();

    // Removals from the tail, so the indices ahead of each removal stay valid.
    for ix in (0..current.len()).rev() {
        if !next.contains(&current[ix]) {
            if ix < state.sizes().len() {
                state.remove_panel(ix, cx);
            }
            current.remove(ix);
        }
    }

    for (ix, node) in next.iter().enumerate() {
        if current.get(ix) == Some(node) {
            continue;
        }
        let at = ix.min(state.sizes().len());
        // Passed through as-is. The tree already decided this slot's share
        // when the panel was inserted — a drop halves the neighbour it landed
        // beside — so there is nothing to compute here, and nothing that
        // depends on a container this state may not have measured yet.
        state.insert_panel(sizes.get(ix).copied().flatten(), Some(at), cx);
        current.insert(at.min(current.len()), *node);
    }

    debug_assert_eq!(
        current, next,
        "the split's panel list must end up mirroring its children exactly; \
         a reordering edit would need its own case here"
    );
}

fn plan_tree(tree: &PaneTree, collapsed: bool, locked: bool, out: &mut Vec<ContainerPlan>) {
    // The root has nothing beside it by definition.
    plan_node(tree.root(), true, collapsed, locked, out);
}

fn plan_node(
    node: &PaneNode,
    alone: bool,
    collapsed: bool,
    locked: bool,
    out: &mut Vec<ContainerPlan>,
) {
    match node.kind() {
        PaneRef::Split {
            axis,
            children,
            sizes,
        } => {
            out.push(ContainerPlan::Split {
                node: node.id(),
                axis,
                children: children.iter().map(PaneNode::id).collect(),
                sizes: sizes.to_vec(),
            });
            let children_alone = children.len() <= 1;
            for child in children {
                plan_node(child, children_alone, collapsed, locked, out);
            }
        }
        PaneRef::Tabs { panels, active_ix } => out.push(ContainerPlan::Group {
            node: node.id(),
            panels: panels.to_vec(),
            active_ix,
            constraints: TabGroupConstraints::in_split(alone)
                .dock_locked(locked)
                .collapsed(collapsed),
        }),
    }
}

fn first_tab_group(node: &PaneNode) -> Option<NodeId> {
    match node.kind() {
        PaneRef::Tabs { .. } => Some(node.id()),
        PaneRef::Split { children, .. } => children.iter().find_map(first_tab_group),
    }
}

fn target_node(target: &InsertTarget) -> NodeId {
    match target {
        InsertTarget::Tabs { node, .. } | InsertTarget::Split { node, .. } => *node,
    }
}

/// Rebuilds panels out of persisted state through [`PanelRegistry`].
struct RegistryPanelBuilder<'a, 'w, 'c> {
    dock_area: WeakEntity<DockArea>,
    renderer: Rc<dyn DockAreaRenderer>,
    built: &'a mut Vec<(PanelId, Arc<dyn PanelView>)>,
    window: &'w mut Window,
    cx: &'c mut App,
}

impl PanelBuilder for RegistryPanelBuilder<'_, '_, '_> {
    fn build(&mut self, state: &PanelState, info: &PanelInfo) -> PanelId {
        let context = PanelBuildContext::new(self.dock_area.clone(), state, info);
        let view =
            match PanelRegistry::build_panel(&state.panel_name, context, self.window, self.cx) {
                Some(view) => view,
                None => self
                    .renderer
                    .build_placeholder(state, self.window, self.cx)
                    .unwrap_or_else(|| {
                        Arc::new(self.cx.new(|cx| PlaceholderPanel::new(state.clone(), cx)))
                            as Arc<dyn PanelView>
                    }),
            };

        let id = view.panel_id(self.cx);
        self.built.push((id, view));
        id
    }
}

/// Stands in for a panel this build cannot construct.
///
/// It draws nothing — an "unknown panel" message is presentation and belongs
/// above this seam — but it keeps the original [`PanelState`] and hands it
/// back from [`Panel::dump`], so a layout written by a newer build survives a
/// load and save here instead of losing the panel.
struct PlaceholderPanel {
    state: PanelState,
    focus_handle: FocusHandle,
}

impl PlaceholderPanel {
    fn new(state: PanelState, cx: &mut Context<Self>) -> Self {
        Self {
            state,
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Panel for PlaceholderPanel {
    fn panel_name(&self) -> &'static str {
        "InvalidPanel"
    }

    fn dump(&self, _: &App) -> PanelState {
        self.state.clone()
    }
}

impl EventEmitter<PanelEvent> for PlaceholderPanel {}

impl Focusable for PlaceholderPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PlaceholderPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

type DockToggleHandler = Rc<dyn Fn(&mut Window, &mut App)>;
type DockResizeHandler = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;

/// What a skin needs to draw one dock, and the callbacks it invokes rather
/// than reimplementing the open/close and clamping behavior.
#[derive(Clone)]
pub struct DockContext {
    placement: DockPlacement,
    size: Pixels,
    open: bool,
    collapsible: bool,
    on_toggle: DockToggleHandler,
    on_resize: DockResizeHandler,
    on_resize_end: DockToggleHandler,
}

impl DockContext {
    pub fn placement(&self) -> DockPlacement {
        self.placement
    }

    /// The dock's extent along its own axis: width for left/right, height for
    /// bottom.
    pub fn size(&self) -> Pixels {
        self.size
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_collapsible(&self) -> bool {
        self.collapsible
    }

    pub fn toggle(&self, window: &mut Window, cx: &mut App) {
        (self.on_toggle)(window, cx);
    }

    /// Resize from a pointer position in window coordinates. Base clamps it
    /// against the area bounds and the opposite dock.
    pub fn resize_to(&self, pointer: Point<Pixels>, window: &mut Window, cx: &mut App) {
        (self.on_resize)(pointer, window, cx);
    }

    /// End a resize started with [`Self::resize_to`]. A bottom dock dragged
    /// below its minimum snaps shut or to the minimum, whichever is nearer.
    pub fn end_resize(&self, window: &mut Window, cx: &mut App) {
        (self.on_resize_end)(window, cx);
    }
}

/// What a closed bottom dock keeps by default, so its tab bar stays
/// clickable. See [`DockAreaRenderer::closed_dock_extent`]. A
/// closed side dock keeps nothing: there is no tab bar left to click at zero
/// width, and reopening it is the application's to offer.
pub const CLOSED_BOTTOM_STRIP: Pixels = px(29.);

/// The box a dock occupies: its extent along its own axis, full across, and
/// held at that size rather than stretched by the row it sits in.
///
/// Structural, not decorative, which is why it is built here and not in a
/// renderer. See [`DockArea::render_dock`].
pub fn dock_frame(dock: &DockContext, size: Pixels) -> Div {
    div()
        .flex()
        .flex_none()
        .relative()
        .overflow_hidden()
        .map(|this| match dock.placement() {
            DockPlacement::Left | DockPlacement::Right => this.flex_row().h_full().w(size),
            DockPlacement::Bottom => this.w_full().h(size),
            // Base never builds a dock for the centre.
            DockPlacement::Center => this,
        })
}

/// Appearance for the dock area. Base draws none of it.
///
/// The frame hooks return the element itself rather than wrapping one, for the
/// same reason [`TabGroupRenderer`]'s do: base tracks focus and records the
/// area bounds on the very element the skin styles.
///
/// There is no separate `render_resize_handle` hook. A handle needs to be
/// positioned against the dock it resizes, and positioning is the skin's; the
/// skin draws it inside [`Self::render_dock`] and drives it through
/// [`DockContext::resize_to`].
#[allow(unused_variables)]
pub trait DockAreaRenderer: 'static {
    /// The area's outer frame, which base records its bounds on.
    /// Appearance only. The area is laid out as a row around whatever this
    /// returns, because that is what makes a dock a column beside the centre
    /// rather than a block above it, and a renderer cannot be expected to know
    /// it had a row to declare.
    fn frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        div().id("dock-area")
    }

    /// One split container's frame, around base's resizable group.
    ///
    /// This one really is a wrapper, unlike the other frame hooks, and
    /// deliberately: base attaches nothing to it, so there is no hit area to
    /// separate from the painted area. It exists because the old `StackPanel`
    /// carried real appearance here — a background and an overflow clip — and
    /// without it a skin could style a dock and a tab group but nothing in
    /// between. `Stateful<Div>` rather than a plain one so the skin keeps a
    /// role, a tooltip, and scroll tracking.
    fn split_frame(
        &self,
        node: NodeId,
        axis: Axis,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        div().id(("dock-split-frame", node.as_u64()))
    }

    /// The column holding the center region and the bottom dock.
    /// Appearance only; see [`DockAreaRenderer::frame`]. The centre fills what
    /// the side docks leave and stacks with the bottom dock either way.
    fn center_frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        div().id("dock-area-center")
    }

    /// The painted part of the divider between two slots of a split.
    ///
    /// `None` keeps base's own one-pixel line, so a skin that has no opinion
    /// about dividers implements nothing. The hit area, the cursor and the
    /// drag itself stay with base either way — this hook supplies appearance
    /// only, and is told the axis and whether the divider is being dragged.
    fn render_split_handle(
        &self,
        handle: &ResizeHandleContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        None
    }

    /// How much a closed dock keeps along its own axis.
    ///
    /// The default keeps [`CLOSED_BOTTOM_STRIP`] for the bottom dock, so its
    /// tab bar stays clickable to reopen it, and nothing for a side dock. A
    /// skin whose application offers its own toggle can return `px(0.)` for
    /// the bottom dock too. The value also sets where a dragged bottom dock
    /// snaps shut.
    fn closed_dock_extent(&self, placement: DockPlacement) -> Pixels {
        match placement {
            DockPlacement::Bottom => CLOSED_BOTTOM_STRIP,
            _ => px(0.),
        }
    }

    /// One dock's chrome around its content: the title strip, the collapse
    /// affordance, and the resize handle.
    ///
    /// Chrome only. The dock's own box -- its extent along its own axis, and
    /// the `flex_none` that holds it there -- is applied by
    /// [`DockArea::render_dock`] around whatever this returns, so a renderer
    /// cannot misplace a dock by not knowing to size it, and the default here
    /// can be what it is: the content, undecorated.
    fn render_dock(
        &self,
        dock: &DockContext,
        content: AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        content
    }

    /// The stand-in for a panel this build cannot construct — one whose
    /// `panel_name` no [`PanelRegistry`] builder answers to. `None` takes
    /// base's own placeholder, which draws nothing.
    ///
    /// The hook exists because a placeholder cannot be wrapped after the
    /// fact: presentation reaches base only through the handle a panel is
    /// registered behind, so whoever creates the panel decides what it can
    /// draw. An "unknown panel" message is presentation, so the skin creates
    /// that panel or does without one.
    ///
    /// A placeholder is what gets written back out on the next save, so one
    /// supplied here should answer [`Panel::dump`] with `state` unchanged —
    /// otherwise saving after a load erases the panel it stood in for. Base's
    /// own placeholder does; nothing here can enforce it of a skin's.
    fn build_placeholder(
        &self,
        state: &PanelState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn PanelView>> {
        None
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer>;
}

/// The renderer an area starts with: the layout and nothing else.
struct BareDockArea;

impl DockAreaRenderer for BareDockArea {
    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(BareTabGroup)
    }
}
