//! Changing the map: the undoable edit wrapper, labels, folding, navigation and
//! clipboard.

use std::path::PathBuf;

use eframe::egui;

use super::{Action, EditSession, HISTORY_LIMITS, MindMapApp, Screen};
use crate::clipboard;
use crate::details::DetailsPanel;
use crate::history::History;
use crate::model::{MapError, MindMap, Node, NodeId};
use crate::navigation::{self, Direction};

impl MindMapApp {
    /// Apply `change` to the map, recording an undo point only if it succeeds.
    pub(super) fn edit_map<R>(
        &mut self,
        change: impl FnOnce(&mut MindMap) -> Result<R, MapError>,
    ) -> Option<R> {
        let snapshot = self.map.clone();
        match change(&mut self.map) {
            Ok(value) => {
                self.history.record(snapshot);
                self.mark_dirty();
                Some(value)
            }
            Err(err) => {
                self.status = err.to_string();
                None
            }
        }
    }

    /// Fold or unfold `id`'s branch (nodes without children are left alone). If the
    /// selection disappears into a folded branch, the folded node becomes selected.
    pub(super) fn toggle_collapse(&mut self, id: NodeId) {
        let Some(node) = self.map.node(id) else {
            return;
        };
        if node.children().is_empty() {
            return;
        }
        let collapse = !node.is_collapsed();
        if self
            .edit_map(|map| map.set_collapsed(id, collapse))
            .is_some()
            && collapse
        {
            let map = &self.map;
            let hidden = |selected: NodeId| selected != id && map.is_in_subtree(selected, id);
            self.also_selected.retain(|&selected| !hidden(selected));
            if self.selected.is_some_and(hidden) {
                self.selected = Some(id);
                self.also_selected.remove(&id);
            }
        }
    }

    /// Whether the selected node can be folded or unfolded, and which label to show.
    pub(super) fn fold_state(&self) -> (bool, &'static str) {
        let node = self
            .selected
            .filter(|&id| id != self.map.root())
            .and_then(|id| self.map.node(id));
        match node {
            Some(node) if node.is_collapsed() => (true, "Expand"),
            Some(node) => (!node.children().is_empty(), "Collapse"),
            None => (false, "Collapse"),
        }
    }

    pub(super) fn after_history_change(&mut self) {
        self.prune_selection();
        self.editing = None;
        // Reload the side panel so it shows (and will not re-apply) the undone text.
        if let Some(panel) = &mut self.details {
            panel.follow(&self.map, panel.node());
        }
        self.mark_dirty();
    }

    pub(super) fn replace_map(&mut self, map: MindMap, path: Option<PathBuf>) {
        self.map = map;
        self.history = History::new(HISTORY_LIMITS);
        self.select(None);
        self.editing = None;
        self.drag = None;
        // Node ids of the old map mean nothing in the new one.
        if let Some(panel) = &mut self.details {
            *panel = DetailsPanel::default();
        }
        self.context_node = None;
        self.file_path = path;
        // Set again by `open_path` when the map came from a file just read.
        self.file_stamp = None;
        self.dirty = false;
        self.needs_fit = true;
        self.screen = Screen::Editor;
    }

    /// Move the selection with an arrow key and scroll the new node into view. With
    /// nothing selected, any arrow starts at the central idea.
    pub(super) fn navigate(&mut self, direction: Direction) {
        let target = match self.selected {
            Some(from) => navigation::neighbor(&self.map, from, direction),
            None => Some(self.map.root()),
        };
        if target.is_some() {
            self.select(target);
            self.reveal = target;
        }
    }

    pub(super) fn start_edit(&mut self, id: NodeId, buffer: String) {
        self.focus(id);
        self.editing = Some(EditSession {
            id,
            buffer,
            request_focus: true,
        });
    }

    /// Finish the current edit, keeping the old label if the new one is blank or unchanged.
    pub(super) fn commit_edit(&mut self) {
        let Some(session) = self.editing.take() else {
            return;
        };
        // Windows clipboards and keyboards may produce "\r\n"; store plain "\n".
        let buffer = session.buffer.replace("\r\n", "\n");
        let text = buffer.trim();
        let unchanged = self
            .map
            .node(session.id)
            .is_none_or(|node| node.text() == text);
        if !text.is_empty() && !unchanged {
            self.edit_map(|map| map.set_text(session.id, text));
        }
    }

    /// Select `id`, unfold any folded branches hiding it (one undo step), and scroll to it.
    pub(super) fn show_node(&mut self, id: NodeId) {
        let folded: Vec<NodeId> = self
            .map
            .ancestors(id)
            .filter(|&ancestor| self.map.node(ancestor).is_some_and(Node::is_collapsed))
            .collect();
        if !folded.is_empty() {
            self.edit_map(|map| {
                folded
                    .iter()
                    .try_for_each(|&ancestor| map.set_collapsed(ancestor, false))
            });
        }
        self.select(Some(id));
        self.reveal = Some(id);
    }

    /// Copy the selected branches (or the whole map) to the system clipboard as an
    /// outline, removing them from the map when `cut` is true.
    pub(super) fn copy_branch(&mut self, ctx: &egui::Context, cut: bool) {
        let root = self.map.root();
        let mut branches = self.selected_branches();
        if branches.is_empty() {
            branches.push(root);
        }
        if cut && branches.contains(&root) {
            "Select a branch to cut; the central idea cannot be removed"
                .clone_into(&mut self.status);
            return;
        }
        ctx.copy_text(clipboard::branches_outline(&self.map, &branches));
        if cut {
            self.perform(Action::Delete);
            "Cut to the clipboard".clone_into(&mut self.status);
        } else {
            "Copied to the clipboard".clone_into(&mut self.status);
        }
    }

    /// Paste an outline from the clipboard as new branches under the selected node.
    pub(super) fn paste_outline(&mut self, text: &str) {
        let items = clipboard::parse_outline(text);
        if items.is_empty() {
            "Nothing to paste: the clipboard holds no text".clone_into(&mut self.status);
            return;
        }
        let target = self.selected.unwrap_or_else(|| self.map.root());
        if let Some(branches) = self.edit_map(|map| clipboard::paste(map, target, &items)) {
            self.status = format!("Pasted {} nodes", items.len());
            if let Some(&first) = branches.first() {
                self.select(Some(first));
                self.reveal = Some(first);
            }
        }
    }
}
