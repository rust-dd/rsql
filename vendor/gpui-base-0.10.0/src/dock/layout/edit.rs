use std::collections::HashMap;

use gpui::Pixels;

use crate::Placement;

use super::node::{NodeId, NodeKind, PaneNode, PanelId};
use super::tree::PaneTree;

/// Where a panel should land.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InsertTarget {
    /// Into an existing tab group, optionally at a specific index.
    Tabs {
        node: NodeId,
        ix: Option<usize>,
        activate: bool,
    },
    /// Beside an existing node, creating a new tab group for the panel.
    Split {
        node: NodeId,
        placement: Placement,
        size: Option<Pixels>,
    },
}

/// What one edit changed.
///
/// Only whether anything changed, for now. An earlier revision also carried
/// the created and removed nodes, the removed panels, and the activation
/// edges — but nothing outside tests ever read them, and computing them meant
/// cloning the whole tree on every edit to diff against. Fields are private, so
/// any of them can come back the day something needs one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditResult {
    changed: bool,
}

impl EditResult {
    pub fn changed(&self) -> bool {
        self.changed
    }
}

impl PaneTree {
    pub fn insert_panel(&mut self, panel: PanelId, target: InsertTarget) -> EditResult {
        self.edit(|tree| tree.apply_insert(panel, target))
    }

    pub fn remove_panel(&mut self, panel: PanelId) -> EditResult {
        self.edit(|tree| tree.detach_panel(panel))
    }

    /// Move a panel to a new home without ever removing it from the tree's
    /// perspective, so the caller never fires `on_removed` for a drag.
    pub fn move_panel(&mut self, panel: PanelId, target: InsertTarget) -> EditResult {
        self.edit(|tree| {
            let detached = tree.detach_panel(panel);
            let inserted = tree.apply_insert(panel, target);
            detached || inserted
        })
    }

    pub fn split(
        &mut self,
        at: NodeId,
        panel: PanelId,
        placement: Placement,
        size: Option<Pixels>,
    ) -> EditResult {
        self.insert_panel(
            panel,
            InsertTarget::Split {
                node: at,
                placement,
                size,
            },
        )
    }

    pub fn set_active(&mut self, node: NodeId, ix: usize) -> EditResult {
        self.edit(|tree| {
            let Some(path) = tree.path_of_node(node) else {
                return false;
            };
            let NodeKind::Tabs { active_ix, .. } = tree.node_at_mut(&path).kind_mut() else {
                return false;
            };
            if *active_ix == ix {
                return false;
            }
            *active_ix = ix;
            true
        })
    }

    /// Replace a split's slot sizes wholesale.
    ///
    /// A no-op, like every other operation given input it cannot resolve, if
    /// `new_sizes.len()` does not match the split's child count: no rule in
    /// `normalize` repairs a length mismatch, so applying it would otherwise
    /// leave `children.len() != sizes.len()` and trip `normalize`'s
    /// `debug_assert!`.
    pub fn set_sizes(&mut self, node: NodeId, new_sizes: Vec<Option<Pixels>>) -> EditResult {
        self.edit(|tree| {
            let Some(path) = tree.path_of_node(node) else {
                return false;
            };
            let NodeKind::Split {
                children, sizes, ..
            } = tree.node_at_mut(&path).kind_mut()
            else {
                return false;
            };
            if new_sizes.len() != children.len() || *sizes == new_sizes {
                return false;
            }
            *sizes = new_sizes;
            true
        })
    }
}

impl PaneTree {
    /// Replace every split's slot sizes with what that split is actually drawn
    /// at.
    ///
    /// A layout is usually built with unconstrained slots, and `None` stays
    /// `None` in the tree no matter how the split is later measured or
    /// dragged. That is fine until an edit has to divide space — dropping a
    /// panel beside another one has to halve *something*, and it cannot halve
    /// an unknown. Adopting the measured sizes first turns the question into
    /// arithmetic on real pixels, and it matches what the person dragging
    /// sees: the layout on screen is the layout being divided.
    pub(crate) fn adopt_measured_sizes(&mut self, measured: &HashMap<NodeId, Vec<Pixels>>) {
        fn walk(node: &mut PaneNode, measured: &HashMap<NodeId, Vec<Pixels>>) {
            let id = node.id();
            let NodeKind::Split {
                children, sizes, ..
            } = node.kind_mut()
            else {
                return;
            };

            if let Some(actual) = measured.get(&id) {
                if actual.len() == sizes.len() {
                    for (slot, size) in sizes.iter_mut().zip(actual) {
                        // A zero means the split has not been laid out yet, so
                        // the slot keeps whatever it already had.
                        if *size > Pixels::ZERO {
                            *slot = Some(*size);
                        }
                    }
                }
            }

            for child in children.iter_mut() {
                walk(child, measured);
            }
        }

        walk(self.root_mut(), measured);
    }
}

impl PaneTree {
    /// Apply one mutation, then collapse.
    ///
    /// `apply` reports whether it changed anything and normalization reports
    /// the same, so no snapshot of the previous tree is needed to answer the
    /// only question a caller asks. A mutation that cannot resolve its target
    /// returns `false` and the whole edit is a no-op.
    fn edit(&mut self, apply: impl FnOnce(&mut Self) -> bool) -> EditResult {
        let mutated = apply(self);
        let collapsed = self.normalize_reporting();

        EditResult {
            changed: mutated || collapsed,
        }
    }

    /// Whether `panel` is anywhere in this tree.
    pub fn contains_panel(&self, panel: PanelId) -> bool {
        self.panels().any(|candidate| candidate == panel)
    }
}

impl PaneTree {
    /// Returns whether the panel actually landed. A target this tree cannot
    /// resolve — a stale node id, or one whose kind does not match the
    /// target — is a no-op rather than an error, and reports `false`.
    fn apply_insert(&mut self, panel: PanelId, target: InsertTarget) -> bool {
        match target {
            InsertTarget::Tabs { node, ix, activate } => {
                let Some(path) = self.path_of_node(node) else {
                    return false;
                };
                let NodeKind::Tabs { panels, active_ix } = self.node_at_mut(&path).kind_mut()
                else {
                    return false;
                };
                let ix = ix.unwrap_or(panels.len()).min(panels.len());
                panels.insert(ix, panel);
                if activate {
                    *active_ix = ix;
                } else if ix <= *active_ix && panels.len() > 1 {
                    // Keep the displayed panel displayed.
                    *active_ix += 1;
                }
                true
            }
            InsertTarget::Split {
                node,
                placement,
                size,
            } => self.insert_beside(node, panel, placement, size),
        }
    }

    /// Place `panel` in a new tab group beside `node`.
    ///
    /// When the parent split already runs along the placement's axis the new
    /// group becomes a sibling. Otherwise `node` is wrapped in a fresh split of
    /// the placement's axis. Rule 3 of `normalize` then flattens any redundant
    /// nesting this creates, which is why no "reuse the parent split" special
    /// case is needed here.
    fn insert_beside(
        &mut self,
        node: NodeId,
        panel: PanelId,
        placement: Placement,
        size: Option<Pixels>,
    ) -> bool {
        let Some(path) = self.path_of_node(node) else {
            return false;
        };
        let group_id = self.allocate_node_id();
        let group = PaneNode::new(
            group_id,
            NodeKind::Tabs {
                panels: vec![panel],
                active_ix: 0,
            },
        );
        let before = matches!(placement, Placement::Left | Placement::Top);

        if let Some((parent_path, ix)) = split_parent_of(&path) {
            let parent_axis = match self.node_at(&parent_path).kind_ref() {
                NodeKind::Split { axis, .. } => Some(*axis),
                _ => None,
            };

            if parent_axis == Some(placement.axis()) {
                let NodeKind::Split {
                    children, sizes, ..
                } = self.node_at_mut(&parent_path).kind_mut()
                else {
                    return false;
                };
                // The new group splits the slot it landed beside, rather
                // than the whole row being re-divided: drop a panel next to
                // one neighbour and it is that neighbour's space you take.
                // A caller that named a size gets exactly it, and the
                // neighbour is left alone.
                //
                // An unconstrained neighbour halves to another `None`, which
                // is right for the same reason — the two of them go on
                // sharing whatever the fixed slots leave over, now between
                // themselves.
                let share = match size {
                    Some(size) => Some(size),
                    None => {
                        let half = sizes[ix].map(|slot| slot / 2.);
                        sizes[ix] = half;
                        half
                    }
                };
                let at = if before { ix } else { ix + 1 };
                children.insert(at, group);
                sizes.insert(at, share);
                return true;
            }
        }

        // Wrap the target in a new split of the placement's axis. The target
        // is swapped out rather than cloned: it can be a whole subtree, and
        // the placeholder left behind is overwritten two statements later.
        let wrapper_id = self.allocate_node_id();
        let target = std::mem::replace(
            self.node_at_mut(&path),
            PaneNode::new(
                wrapper_id,
                NodeKind::Tabs {
                    panels: Vec::new(),
                    active_ix: 0,
                },
            ),
        );
        let (children, sizes) = if before {
            (vec![group, target], vec![size, None])
        } else {
            (vec![target, group], vec![None, size])
        };
        let wrapper = PaneNode::new(
            wrapper_id,
            NodeKind::Split {
                axis: placement.axis(),
                children,
                sizes,
            },
        );
        *self.node_at_mut(&path) = wrapper;
        true
    }

    /// Remove `panel` wherever it lives. Returns whether it was found.
    fn detach_panel(&mut self, panel: PanelId) -> bool {
        let Some(node) = self.find_panel_node(panel) else {
            return false;
        };
        let Some(path) = self.path_of_node(node) else {
            return false;
        };

        match self.node_at_mut(&path).kind_mut() {
            NodeKind::Tabs { panels, active_ix } => {
                let Some(ix) = panels.iter().position(|p| *p == panel) else {
                    return false;
                };
                panels.remove(ix);
                if ix < *active_ix {
                    *active_ix -= 1;
                }
                true
            }
            NodeKind::Split { .. } => false,
        }
    }
}

/// Split the path into its parent path and the child index, or `None` at the root.
fn split_parent_of(path: &super::tree::NodePath) -> Option<(super::tree::NodePath, usize)> {
    let (&ix, parent) = path.split_last()?;
    Some((parent.iter().copied().collect(), ix))
}
