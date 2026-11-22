//! Which nodes are selected, and the edits that act on all of them.
//!
//! There is one main node (`selected`), which renaming, the details panel and the arrow
//! keys follow, plus any others picked with Ctrl+click or a selection box
//! (`also_selected`). Dragging, colouring, deleting and copying act on all of them.

use super::MindMapApp;
use crate::details::StyleChange;
use crate::model::{MapError, NodeColor, NodeId};
use crate::navigation::{self, Direction};

impl MindMapApp {
    /// Select only `id`, or nothing.
    pub(super) fn select(&mut self, id: Option<NodeId>) {
        self.selected = id;
        self.also_selected.clear();
    }

    /// Whether `id` is selected (as the main node or one of the others).
    pub(super) fn is_selected(&self, id: NodeId) -> bool {
        self.selected == Some(id) || self.also_selected.contains(&id)
    }

    /// Every selected node, the main one first.
    pub(super) fn selection(&self) -> impl Iterator<Item = NodeId> + '_ {
        // `Option` iterates like a list of zero or one items.
        self.selected
            .into_iter()
            .chain(self.also_selected.iter().copied())
    }

    /// Make `id` the main node. A node that is already selected keeps the others
    /// selected (so a drag or right-click acts on all of them); any other node becomes the
    /// only one selected.
    pub(super) fn focus(&mut self, id: NodeId) {
        if !self.is_selected(id) {
            self.select(Some(id));
        } else if self.also_selected.remove(&id)
            && let Some(previous) = self.selected.replace(id)
        {
            self.also_selected.insert(previous);
        }
    }

    /// Ctrl+click: add `id` to the selection as the main node, or take it out.
    pub(super) fn toggle_selected(&mut self, id: NodeId) {
        if self.selected == Some(id) {
            // Another selected node (if any) takes over as the main one.
            self.selected = self.also_selected.iter().next().copied();
            if let Some(next) = self.selected {
                self.also_selected.remove(&next);
            }
        } else if !self.also_selected.remove(&id)
            && let Some(previous) = self.selected.replace(id)
        {
            self.also_selected.insert(previous);
        }
        self.report_selection();
    }

    /// Select the nodes in `ids` (from a selection box). With `add`, they join the current
    /// selection; otherwise they replace it, the first becoming the main node.
    pub(super) fn select_many(&mut self, ids: impl IntoIterator<Item = NodeId>, add: bool) {
        let mut ids = ids.into_iter();
        if !add {
            self.select(None);
        }
        if self.selected.is_none() {
            self.selected = ids.next();
        }
        let main = self.selected;
        self.also_selected
            .extend(ids.filter(|&id| Some(id) != main));
        self.report_selection();
    }

    /// Drop selected nodes that no longer exist (after undo, redo or a delete).
    pub(super) fn prune_selection(&mut self) {
        // Borrowing two different fields at once is fine; borrowing `self` twice is not.
        let map = &self.map;
        self.also_selected.retain(|&id| map.node(id).is_some());
        self.selected = self.selected.filter(|&id| map.node(id).is_some());
        if self.selected.is_none() {
            self.selected = self.also_selected.iter().next().copied();
            if let Some(main) = self.selected {
                self.also_selected.remove(&main);
            }
        }
    }

    /// Show how many nodes are selected, when it is more than one.
    fn report_selection(&mut self) {
        let count = usize::from(self.selected.is_some()) + self.also_selected.len();
        if count > 1 {
            self.status = format!("{count} nodes selected");
        }
    }

    /// The selected branches: each selected node that is not inside another selected
    /// node's branch, in tree order. Moving, copying or deleting these covers every
    /// selected node exactly once.
    pub(super) fn selected_branches(&self) -> Vec<NodeId> {
        self.branches_where(|id| self.is_selected(id))
    }

    /// The nodes for which `member` is true that have no such ancestor, in tree order.
    fn branches_where(&self, member: impl Fn(NodeId) -> bool) -> Vec<NodeId> {
        if self.also_selected.is_empty() {
            // The common case, without walking the whole map.
            return self.selected.filter(|&id| member(id)).into_iter().collect();
        }
        // `&&` stops early, so ancestors are only walked for the few selected nodes.
        self.map
            .preorder()
            .map(|visit| visit.id)
            .filter(|&id| member(id) && !self.map.ancestors(id).any(&member))
            .collect()
    }

    /// Delete every selected branch except the central idea, as one undo step, then
    /// select the nearest surviving ancestor of the main node.
    pub(super) fn delete_selection(&mut self) {
        let root = self.map.root();
        let branches = self.branches_where(|id| id != root && self.is_selected(id));
        if branches.is_empty() {
            return;
        }
        let map = &self.map;
        let deleted = |id: NodeId| branches.iter().any(|&branch| map.is_in_subtree(id, branch));
        let survivor = self.selected.and_then(|main| {
            std::iter::once(main)
                .chain(map.ancestors(main))
                .find(|&id| !deleted(id))
        });
        let removed = self.edit_map(|map| {
            branches
                .iter()
                .try_for_each(|&branch| map.remove_subtree(branch).map(drop))
        });
        if removed.is_some() {
            self.select(survivor);
        }
    }

    /// Give every selected node `color` (`None` = automatic), as one undo step.
    pub(super) fn color_selection(&mut self, color: Option<NodeColor>) {
        // Text typed in the side panel is saved first, so reloading it below loses nothing.
        self.flush_details();
        let targets: Vec<NodeId> = self.selection().collect();
        self.edit_map(|map| targets.iter().try_for_each(|&id| map.set_color(id, color)));
        if let Some(panel) = &mut self.details {
            panel.follow(&self.map, panel.node());
        }
    }

    /// Make one style change (bold, italic or size) to every selected node, as one undo
    /// step.
    pub(super) fn style_selection(&mut self, change: StyleChange) {
        // Text typed in the side panel is saved first, so reloading it below loses nothing.
        self.flush_details();
        let targets: Vec<NodeId> = self.selection().collect();
        self.edit_map(|map| {
            targets.iter().try_for_each(|&id| {
                let style = map.node(id).ok_or(MapError::NodeNotFound(id))?.style();
                map.set_style(id, change.apply(style))
            })
        });
        if let Some(panel) = &mut self.details {
            panel.follow(&self.map, panel.node());
        }
    }

    /// Ctrl+B: make the selected nodes bold, or plain again if the main one is bold.
    pub(super) fn toggle_bold(&mut self) {
        let Some(main) = self.selected.and_then(|id| self.map.node(id)) else {
            return;
        };
        let bold = !main.style().bold;
        self.style_selection(StyleChange::Bold(bold));
    }

    /// Alt+Up/Down: swap the main node (with its branch) with the sibling above or below
    /// it on screen.
    pub(super) fn reorder(&mut self, direction: Direction) {
        let Some(id) = self
            .selected
            .filter(|_| matches!(direction, Direction::Up | Direction::Down))
        else {
            return;
        };
        if let Some(other) = navigation::neighbor(&self.map, id, direction) {
            self.edit_map(|map| map.swap_siblings(id, other));
            self.reveal = Some(id);
        }
    }
}
