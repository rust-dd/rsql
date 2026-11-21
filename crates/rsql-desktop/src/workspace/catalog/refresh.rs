use super::*;
use crate::workspace::toast::Toast;

/// The nodes one explicit sidebar refresh still waits for, scoped to its catalog epoch.
pub(super) struct Feedback {
    epoch: u64,
    waiting: BTreeSet<Node>,
    failed: bool,
}

impl Catalog {
    /// Settles `node` for a pending explicit refresh; once every node settles, returns whether any failed.
    pub(super) fn settle_feedback(&mut self, node: &Node) -> Option<bool> {
        let feedback = self.feedback.as_mut()?;
        if feedback.epoch != self.epoch {
            self.feedback = None;
            return None;
        }
        match self.entries.get(node) {
            Some(Load::Pending(_)) => return None,
            Some(Load::Failed(_)) => feedback.failed |= feedback.waiting.remove(node),
            // A node collapsed mid-refresh is cancelled, which is neither success nor failure.
            _ => {
                feedback.waiting.remove(node);
            }
        }
        if !feedback.waiting.is_empty() {
            return None;
        }
        Some(self.feedback.take()?.failed)
    }
}

pub(super) fn toast(failed: bool) -> Toast {
    if failed {
        Toast::error("Failed to refresh connection data")
    } else {
        Toast::success("Connection refreshed")
    }
}

impl Workspace {
    pub(in crate::workspace) fn refresh_connection(&mut self, id: &str, cx: &mut Context<Self>) {
        self.refresh_catalog(id, cx);
        let Some(catalog) = self.navigation.catalogs.get_mut(id) else {
            return;
        };
        let waiting = catalog
            .entries
            .iter()
            .filter(|(_, load)| matches!(load, Load::Pending(_)))
            .map(|(node, _)| node.clone())
            .collect::<BTreeSet<_>>();
        catalog.feedback = Some(Feedback {
            epoch: catalog.epoch,
            waiting,
            failed: false,
        });
    }
}
