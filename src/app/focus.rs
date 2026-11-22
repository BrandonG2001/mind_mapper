//! Focus mode: one branch shown on its own (F6, the toolbar or the right-click menu), with
//! the rest of the map hidden until Esc or F6.

use super::MindMapApp;
use crate::model::{Node, NodeId};

impl MindMapApp {
    /// Show only the selected branch, or the whole map again if focus mode is on.
    pub(super) fn toggle_focus(&mut self) {
        if self.view.focus().is_some() {
            self.leave_focus();
            return;
        }
        let root = self.map.root();
        match self.selected.filter(|&id| id != root) {
            Some(id) => self.enter_focus(id),
            None => "Select a branch to focus on; the central idea already shows everything"
                .clone_into(&mut self.status),
        }
    }

    /// Show only `id` and its branch, fitted to the window.
    pub(super) fn enter_focus(&mut self, id: NodeId) {
        self.view.set_focus(Some(id));
        self.needs_fit = true;
        // Hidden nodes cannot stay selected.
        let map = &self.map;
        self.also_selected
            .retain(|&selected| map.is_in_subtree(selected, id));
        if !self
            .selected
            .is_some_and(|selected| map.is_in_subtree(selected, id))
        {
            self.select(Some(id));
        }
        let label = self.map.node(id).map_or("", Node::text);
        self.status = format!(
            "Focus: showing \"{}\" only; Esc or F6 shows the whole map",
            label.lines().next().unwrap_or_default()
        );
    }

    /// Show the whole map again, fitted to the window.
    pub(super) fn leave_focus(&mut self) {
        if self.view.focus().is_some() {
            self.view.set_focus(None);
            self.needs_fit = true;
            "Showing the whole map".clone_into(&mut self.status);
        }
    }

    /// Whether `id` is on screen as far as focus mode goes: inside the focused branch, or
    /// anywhere when focus mode is off.
    pub(super) fn is_in_focus(&self, id: NodeId) -> bool {
        self.view
            .focus()
            .is_none_or(|branch| self.map.is_in_subtree(id, branch))
    }

    /// Leave focus mode if `id` is outside the focused branch, so it can be seen.
    pub(super) fn bring_into_focus(&mut self, id: NodeId) {
        if !self.is_in_focus(id) {
            self.leave_focus();
        }
    }

    /// Leave focus mode once the focused branch no longer exists (deleted, cut or undone).
    pub(super) fn check_focus(&mut self) {
        if self
            .view
            .focus()
            .is_some_and(|branch| self.map.node(branch).is_none())
        {
            self.leave_focus();
        }
    }
}
