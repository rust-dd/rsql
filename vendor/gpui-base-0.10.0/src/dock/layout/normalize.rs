use gpui::Pixels;

use super::node::{NodeKind, PaneNode};
use super::tree::{PaneTree, RootKind};

/// Upper bound on `normalize` passes. Every pass that changes anything
/// strictly reduces node count or nesting depth, so real dock layouts
/// converge in a small handful of passes; this is a generous ceiling against
/// a future rule change that fights another rule rather than a bound tuned
/// to today's rule set.
const MAX_NORMALIZE_PASSES: u32 = 64;

impl PaneTree {
    /// Collapse the tree to canonical shape.
    ///
    /// One post-order pass repeated to a fixpoint. This is the only place a
    /// container is removed for being empty, replacing the mutually recursive
    /// `remove_self_if_empty` pair the old implementation used. It needs no
    /// parent pointers and no deferred work, so the tree is self-consistent
    /// the instant an edit returns.
    ///
    /// Rules, applied bottom up:
    ///
    /// 1. An empty `Tabs` or `Split` is removed from its parent.
    /// 2. A `Split` with one child is replaced by that child. The child keeps
    ///    its own `NodeId` and inherits the split's slot size.
    /// 3. A `Split` whose child is a `Split` of the same axis splices that
    ///    child's children into itself.
    /// 4. `active_ix` is clamped.
    /// 5. The root is preserved according to [`RootKind`].
    ///
    /// Idempotent: `normalize(normalize(t)) == normalize(t)`.
    pub fn normalize(&mut self) {
        self.normalize_reporting();
    }

    /// [`Self::normalize`], reporting whether it changed anything.
    ///
    /// `edit` uses this instead of comparing whole trees: collapse is the only
    /// thing that can change a tree after a mutation has already reported what
    /// it did, so the two booleans together are exactly the answer, and no
    /// snapshot of the previous tree has to be kept to reach it.
    pub(crate) fn normalize_reporting(&mut self) -> bool {
        let changed = self.run_normalize_passes().1;
        debug_assert!(self.is_normalized(), "normalize did not reach a fixpoint");
        changed
    }

    /// Run passes until nothing changes, or until [`MAX_NORMALIZE_PASSES`] is
    /// exhausted. Returns the number of passes run, and whether any pass
    /// changed the tree.
    fn run_normalize_passes(&mut self) -> (u32, bool) {
        let mut passes = 0;
        let mut changed = true;
        let mut any_change = false;
        // Bounded because every pass that changes anything strictly reduces
        // node count or nesting depth.
        while changed && passes < MAX_NORMALIZE_PASSES {
            changed = false;
            normalize_node(self.root_mut(), &mut changed);
            collapse_root(self, &mut changed);
            any_change |= changed;
            passes += 1;
        }

        // `debug_assert!` in `normalize` disappears in release builds, so a
        // desktop build left silently short of the fixpoint would otherwise
        // render a non-canonical layout with no trace of why. This keeps the
        // failure observable without turning it into a user-facing panic:
        // rendering a slightly non-canonical layout beats crashing the app.
        if changed {
            tracing::warn!(
                passes,
                "PaneTree::normalize exhausted {MAX_NORMALIZE_PASSES} passes without reaching \
                 a fixpoint; the tree may still contain an empty container, a single-child \
                 split, same-axis split nesting, or an unclamped Tabs active_ix"
            );
        }

        (passes, any_change)
    }

    /// Whether the tree satisfies every structural invariant.
    pub(crate) fn is_normalized(&self) -> bool {
        let mut ok = true;
        let root_id = self.root().id();
        self.root().walk(&mut |node| match node.kind_ref() {
            NodeKind::Split {
                children,
                sizes,
                axis,
            } => {
                ok &= children.len() == sizes.len();
                // The root may legitimately be an empty or single-child split.
                if node.id() != root_id {
                    ok &= children.len() > 1;
                }
                ok &= !children.iter().any(|child| {
                    matches!(child.kind_ref(), NodeKind::Split { axis: inner, .. } if inner == axis)
                });
            }
            NodeKind::Tabs { panels, active_ix } => {
                ok &= panels.is_empty() || *active_ix < panels.len();
                if node.id() != root_id {
                    ok &= !panels.is_empty();
                }
            }
        });
        ok
    }
}

fn normalize_node(node: &mut PaneNode, changed: &mut bool) {
    match node.kind_mut() {
        NodeKind::Tabs { panels, active_ix } => {
            let clamped = (*active_ix).min(panels.len().saturating_sub(1));
            if *active_ix != clamped {
                *active_ix = clamped;
                *changed = true;
            }
        }
        NodeKind::Split {
            axis,
            children,
            sizes,
        } => {
            let axis = *axis;

            for child in children.iter_mut() {
                normalize_node(child, changed);
            }

            // Rule 1: drop empty children.
            let mut ix = 0;
            while ix < children.len() {
                if is_empty_container(&children[ix]) {
                    children.remove(ix);
                    sizes.remove(ix);
                    *changed = true;
                } else {
                    ix += 1;
                }
            }

            // Rule 2: a single-child split child is replaced by its child,
            // which inherits the slot size the split occupied. The child is
            // moved out rather than cloned — it can carry an arbitrarily deep
            // subtree, and this runs on every edit.
            for ix in 0..children.len() {
                let is_single = matches!(
                    children[ix].kind_ref(),
                    NodeKind::Split { children: inner, .. } if inner.len() == 1
                );
                if !is_single {
                    continue;
                }
                let NodeKind::Split {
                    children: inner, ..
                } = children[ix].kind_mut()
                else {
                    continue;
                };
                let replacement = inner.remove(0);
                children[ix] = replacement;
                *changed = true;
            }

            // Rule 3: splice same-axis nesting.
            let mut ix = 0;
            while ix < children.len() {
                let same_axis = matches!(
                    children[ix].kind_ref(),
                    NodeKind::Split { axis: inner, .. } if *inner == axis
                );
                if !same_axis {
                    ix += 1;
                    continue;
                }

                // Taken, not cloned: the spliced children move up a level
                // rather than being copied and discarded.
                let NodeKind::Split {
                    children: inner,
                    sizes: inner_sizes,
                    ..
                } = children[ix].kind_mut()
                else {
                    ix += 1;
                    continue;
                };
                let inner = std::mem::take(inner);
                let inner_sizes = std::mem::take(inner_sizes);

                let slot = sizes[ix];
                let inner_sizes = distribute_slot(slot, inner_sizes);
                let count = inner.len();
                children.splice(ix..=ix, inner);
                sizes.splice(ix..=ix, inner_sizes);
                ix += count;
                *changed = true;
            }
        }
    }
}

/// Spread an outer slot size across the inner sizes that replace it.
///
/// When the outer slot is unconstrained the inner sizes pass through. When it
/// is fixed and every inner size is known, they are scaled to fill the slot;
/// otherwise the slot is dropped, matching how an unconstrained child behaves.
fn distribute_slot(slot: Option<Pixels>, inner: Vec<Option<Pixels>>) -> Vec<Option<Pixels>> {
    let Some(slot) = slot else { return inner };
    // `Option<Pixels>` has no `Sum` impl; fold so one unknown size makes the
    // whole total unknown.
    let total = inner
        .iter()
        .try_fold(Pixels::ZERO, |acc, size| size.map(|size| acc + size));
    match total {
        Some(total) if total > Pixels::ZERO => inner
            .into_iter()
            .map(|size| size.map(|size| size * (slot / total)))
            .collect(),
        _ => inner,
    }
}

fn is_empty_container(node: &PaneNode) -> bool {
    match node.kind_ref() {
        NodeKind::Split { children, .. } => children.is_empty(),
        NodeKind::Tabs { panels, .. } => panels.is_empty(),
    }
}

/// Rule 5. A `RootKind::Split` tree keeps a split root no matter what, so an
/// empty center still serializes as a `StackPanel`. A `RootKind::Any` tree
/// lets rule 2 collapse the root like any other node.
fn collapse_root(tree: &mut PaneTree, changed: &mut bool) {
    if tree.root_kind() == RootKind::Split {
        return;
    }

    let replacement = match tree.root().kind_ref() {
        NodeKind::Split { children, .. } if children.len() == 1 => Some(children[0].clone()),
        _ => None,
    };

    if let Some(replacement) = replacement {
        tree.replace_root(replacement);
        *changed = true;
    }
}
