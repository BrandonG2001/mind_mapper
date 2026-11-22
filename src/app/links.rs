//! Cross-links between nodes: starting one (Ctrl+K or the right-click menu), finishing
//! it with a click on the target, and removing links.

use super::{MindMapApp, node_label};
use crate::model::NodeId;

impl MindMapApp {
    /// Start linking from the selected node: the next click on a node links to it.
    pub(super) fn start_link(&mut self) {
        let Some(source) = self.selected else {
            "Select the node to link from first".clone_into(&mut self.status);
            return;
        };
        self.linking = Some(source);
        self.status = format!(
            "Click the node to link \"{}\" to (Esc cancels)",
            node_label(&self.map, source)
        );
    }

    /// Finish the link started at `source` with a click on `target` (`None`: empty
    /// canvas, which cancels).
    pub(super) fn finish_link(&mut self, source: NodeId, target: Option<NodeId>) {
        let target = target.filter(|_| self.map.node(source).is_some());
        let Some(target) = target else {
            "Linking cancelled".clone_into(&mut self.status);
            return;
        };
        // On failure (same node, already linked) `edit_map` reports why in the status.
        if self
            .edit_map(|map| map.add_cross_link(source, target))
            .is_some()
        {
            self.status = format!(
                "Linked \"{}\" to \"{}\"",
                node_label(&self.map, source),
                node_label(&self.map, target)
            );
        }
    }

    /// Stop linking, if a link was started.
    ///
    /// # Returns
    ///
    /// Whether there was one to cancel.
    pub(super) fn cancel_link(&mut self) -> bool {
        let cancelled = self.linking.take().is_some();
        if cancelled {
            "Linking cancelled".clone_into(&mut self.status);
        }
        cancelled
    }

    /// Remove the link between `a` and `b`, as one undo step.
    pub(super) fn remove_link(&mut self, a: NodeId, b: NodeId) {
        if self.edit_map(|map| map.remove_cross_link(a, b)).is_some() {
            self.status = format!(
                "Removed the link between \"{}\" and \"{}\"",
                node_label(&self.map, a),
                node_label(&self.map, b)
            );
        }
    }

    /// The nodes linked to or from `id`, each with whether the arrow points away from
    /// `id`, in the order the links were made.
    pub(super) fn links_of(&self, id: NodeId) -> Vec<(NodeId, bool)> {
        self.map
            .cross_links()
            .iter()
            .filter(|link| link.touches(id))
            .map(|link| {
                if link.from() == id {
                    (link.to(), true)
                } else {
                    (link.from(), false)
                }
            })
            .collect()
    }
}
