use gpui::{Axis, Pixels};

use super::layout::{NodeKind, PaneNode, PaneRef, PaneTree, PanelId, RootKind};
use super::state::{PanelInfo, PanelState};

/// The names written to persisted layouts. These are contract, not type names:
/// they must keep their values even if the Rust types are renamed.
pub(crate) const STACK_PANEL_NAME: &str = "StackPanel";
pub(crate) const TAB_PANEL_NAME: &str = "TabPanel";

/// How the layout tree reads properties of panels it only knows by id.
///
/// Keeping this behind a trait is what lets the whole layout algebra be tested
/// without an `App`.
pub trait PanelSource {
    fn panel_name(&self, id: PanelId) -> &'static str;
    fn is_visible(&self, id: PanelId) -> bool;
    fn dump(&self, id: PanelId) -> PanelState;
}

impl PaneTree {
    pub fn to_state(&self, source: &dyn PanelSource) -> PanelState {
        node_to_state(self.root(), source)
    }
}

fn node_to_state(node: &PaneNode, source: &dyn PanelSource) -> PanelState {
    match node.kind() {
        PaneRef::Split {
            axis,
            children,
            sizes,
        } => PanelState {
            panel_name: STACK_PANEL_NAME.to_string(),
            children: children
                .iter()
                .map(|child| node_to_state(child, source))
                .collect(),
            // `None` is a representation this rewrite introduces, not
            // something the old writer ever produced: the schema's `sizes`
            // field is `Vec<Pixels>`, with no slot for "unconstrained". `0.0`
            // is the sentinel this writer chooses for that case, and the
            // corresponding reader maps a `0.0` it loads back to `None`.
            //
            // That makes a `None` slot safe to persist only transiently: a
            // caller building a tree meant to be written out must resolve
            // every slot to concrete pixels first. An older build reading a
            // persisted `0.0` back has no notion of the sentinel and
            // constructs a real, zero-pixel-wide panel from it.
            info: PanelInfo::stack(
                sizes.iter().map(|size| size.unwrap_or_default()).collect(),
                axis,
            ),
        },
        PaneRef::Tabs { panels, active_ix } => PanelState {
            panel_name: TAB_PANEL_NAME.to_string(),
            children: panels.iter().map(|panel| source.dump(*panel)).collect(),
            // Unconditional, unlike the old writer which assigned this inside
            // its loop and left an empty group looking like a bare panel.
            info: PanelInfo::tabs(active_ix),
        },
    }
}

/// Turns a persisted leaf into a live panel id.
///
/// The production implementation (at the `gpui-component` layer, above this
/// crate) consults `PanelRegistry` and falls back to an invalid-panel
/// placeholder that retains the original `PanelState`, so a panel type this
/// build does not know about survives a load/save round trip instead of
/// being erased.
pub trait PanelBuilder {
    fn build(&mut self, state: &PanelState, info: &PanelInfo) -> PanelId;
}

impl PaneTree {
    /// Read a persisted layout.
    ///
    /// Compatibility rules, all previously implicit in `PanelState::to_item`:
    ///
    /// - a `Tabs` whose children are themselves `Tabs` is flattened;
    /// - a bare `Panel` leaf appearing where a container belongs is wrapped in
    ///   a `Tabs`;
    /// - a node named `TabPanel` carrying `PanelInfo::Panel` is read as an
    ///   empty tab group, recovering data written by the old dump defect (an
    ///   empty `TabPanel` never entered the loop that set its `info` to
    ///   `Tabs`, so it kept `PanelState`'s default `Panel(Value::Null)`). The
    ///   old reader had no such rule: it looked "TabPanel" up in the panel
    ///   registry, found nothing, and rendered an `InvalidPanel` placeholder
    ///   where an empty tab group belonged. This rule is a genuine fix, not a
    ///   preserved behavior;
    /// - a node named `StackPanel` carrying `PanelInfo::Panel` is the same
    ///   defect in the old split writer, and is read as an empty split. Inside
    ///   a tab group it is dropped: a placeholder built from it would dump the
    ///   defective state back unchanged, so the layout could never heal.
    pub fn from_state(
        state: &PanelState,
        root_kind: RootKind,
        builder: &mut dyn PanelBuilder,
    ) -> Self {
        let mut tree = PaneTree::new(root_kind);
        let root = build_node(&mut tree, state, builder);

        let root = match (root_kind, &root) {
            (RootKind::Split, node) if !matches!(node.kind_ref(), NodeKind::Split { .. }) => {
                let id = tree.allocate_node_id();
                PaneNode::new(
                    id,
                    NodeKind::Split {
                        axis: Axis::Horizontal,
                        children: vec![node.clone()],
                        sizes: vec![None],
                    },
                )
            }
            _ => root,
        };

        tree.replace_root(root);
        tree.normalize();
        tree
    }
}

fn build_node(tree: &mut PaneTree, state: &PanelState, builder: &mut dyn PanelBuilder) -> PaneNode {
    let id = tree.allocate_node_id();

    match &state.info {
        PanelInfo::Stack { sizes, axis } => {
            let axis = if *axis == 0 {
                Axis::Horizontal
            } else {
                Axis::Vertical
            };
            let children: Vec<PaneNode> = state
                .children
                .iter()
                .map(|child| build_node(tree, child, builder))
                .collect();
            let sizes = (0..children.len())
                .map(|ix| sizes.get(ix).copied().filter(|size| *size > Pixels::ZERO))
                .collect();
            PaneNode::new(
                id,
                NodeKind::Split {
                    axis,
                    children,
                    sizes,
                },
            )
        }
        PanelInfo::Tabs { active_index } => {
            let panels = collect_tab_panels(&state.children, builder);
            PaneNode::new(
                id,
                NodeKind::Tabs {
                    panels,
                    active_ix: *active_index,
                },
            )
        }
        // A container name carrying a leaf info means the writer that produced
        // this file had the empty-container defect.
        PanelInfo::Panel(_) if state.panel_name == STACK_PANEL_NAME => PaneNode::new(
            id,
            NodeKind::Split {
                axis: Axis::Horizontal,
                children: Vec::new(),
                sizes: Vec::new(),
            },
        ),
        PanelInfo::Panel(_) => {
            let panels = if state.panel_name == TAB_PANEL_NAME {
                Vec::new()
            } else {
                vec![builder.build(state, &state.info)]
            };
            PaneNode::new(
                id,
                NodeKind::Tabs {
                    panels,
                    active_ix: 0,
                },
            )
        }
    }
}

fn is_container_name(panel_name: &str) -> bool {
    panel_name == TAB_PANEL_NAME || panel_name == STACK_PANEL_NAME
}

/// Flatten one level of tab nesting, which the old writer could produce.
fn collect_tab_panels(children: &[PanelState], builder: &mut dyn PanelBuilder) -> Vec<PanelId> {
    children
        .iter()
        .flat_map(|child| match &child.info {
            PanelInfo::Tabs { .. } => collect_tab_panels(&child.children, builder),
            PanelInfo::Panel(_) if is_container_name(&child.panel_name) => Vec::new(),
            _ => vec![builder.build(child, &child.info)],
        })
        .collect()
}
