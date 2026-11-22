//! The toolbar, the details side panel, the find bar and start-screen choices.

use eframe::egui::{self, Ui};

use super::questions::AfterDiscard;
use super::{Action, MindMapApp, Screen};
use crate::details::DetailsEdit;
use crate::model::NodeId;
use crate::recovery::Recovered;
use crate::search::SearchCommand;
use crate::start_screen::StartChoice;
use crate::theme::{theme_picker, ui_size_control};

impl MindMapApp {
    /// Act on a start-screen choice.
    pub(super) fn choose(&mut self, choice: StartChoice) {
        match choice {
            StartChoice::Template(index) => self.discard_then(AfterDiscard::Template(index)),
            StartChoice::Open => self.perform(Action::Open),
            StartChoice::Back => self.screen = Screen::Editor,
            StartChoice::Restore => {
                if let Some(recovered) = self.recovered.take() {
                    let Recovered { map, source, files } = recovered;
                    self.deferred_open = None;
                    self.replace_map(map, source);
                    // Still unsaved: copy it into this window's recovery file before the
                    // orphaned copy is deleted, so there is never a moment without one.
                    self.mark_dirty();
                    self.save_recovery_copy();
                    "Restored unsaved work from the last session".clone_into(&mut self.status);
                    if let Some(files) = files
                        && let Err(err) = files.discard()
                    {
                        self.status = format!("Could not remove recovered work: {err}");
                    }
                    // Other crashed windows' work is offered on the start screen.
                    self.find_orphan();
                }
            }
            StartChoice::OpenRecent(index) => {
                if let Some(path) = self.recent.paths().get(index).cloned() {
                    self.discard_then(AfterDiscard::OpenPath(path));
                }
            }
            StartChoice::DiscardRecovery => {
                self.discard_orphan();
                // Offer the next crashed window's work, if any, before opening a file.
                self.find_orphan();
                if self.recovered.is_none()
                    && let Some(path) = self.deferred_open.take()
                {
                    self.open_path(&path);
                }
            }
        }
    }

    pub(super) fn toolbar(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut clicked = None;
        let root = self.map.root();
        let has_selection = self.selected.is_some();
        let can_delete = self.selection().any(|id| id != root);
        let (can_fold, fold_label) = self.fold_state();
        let buttons = [
            ("New", Action::New, true, "Ctrl+N"),
            ("Open", Action::Open, true, "Ctrl+O"),
            ("Save", Action::Save, true, "Ctrl+S"),
            ("Save As", Action::SaveAs, true, "Ctrl+Shift+S"),
            ("Undo", Action::Undo, self.history.can_undo(), "Ctrl+Z"),
            ("Redo", Action::Redo, self.history.can_redo(), "Ctrl+Y"),
            ("Add child", Action::AddChild, true, "Tab"),
            ("Add sibling", Action::AddSibling, true, "Enter"),
            (
                "Rename",
                Action::Rename,
                has_selection,
                "F2 or double-click",
            ),
            ("Delete", Action::Delete, can_delete, "Delete"),
            ("Center", Action::CenterView, true, "Home"),
            (
                "Export",
                Action::Export,
                true,
                "Ctrl+E: PNG, SVG or Markdown",
            ),
            ("Find", Action::Find, true, "Ctrl+F"),
            (
                "Notes",
                Action::ToggleDetails,
                true,
                "Ctrl+I: colour, notes and link",
            ),
            (
                "Tidy up",
                Action::Tidy,
                true,
                "Ctrl+L: arrange the map without overlaps",
            ),
            (
                fold_label,
                Action::ToggleCollapse,
                can_fold,
                "Space or click the +/- handle",
            ),
        ];
        ui.horizontal_wrapped(|ui| {
            for (label, action, enabled, hint) in buttons {
                if ui
                    .add_enabled(enabled, egui::Button::new(label))
                    .on_hover_text(hint)
                    .clicked()
                {
                    clicked = Some(action);
                }
            }
        });
        // Second row: appearance settings, map zoom and the status message.
        ui.horizontal(|ui| {
            theme_picker(ui, &mut self.theme);
            ui.separator();
            ui_size_control(ui);
            ui.separator();
            ui.label(format!("Map {:.0}%", self.view.camera().zoom() * 100.0))
                .on_hover_text("Map zoom: scroll wheel");
            ui.separator();
            ui.label(&self.status);
        });
        clicked
    }

    /// Apply side-panel or menu edits to node `id`, as one undo step.
    pub(super) fn apply_details(&mut self, id: NodeId, edits: &[DetailsEdit]) {
        if edits.is_empty() {
            return;
        }
        self.edit_map(|map| {
            edits.iter().try_for_each(|edit| match edit {
                DetailsEdit::Note(note) => map.set_note(id, note),
                DetailsEdit::Link(link) => map.set_link(id, link.as_deref()),
                DetailsEdit::Color(color) => map.set_color(id, *color),
            })
        });
        // Keep the side panel in step, so it never holds text the map no longer has.
        if let Some(panel) = &mut self.details
            && panel.node() == Some(id)
        {
            panel.follow(&self.map, Some(id));
        }
    }

    /// Save any text typed in the details panel that has not been applied yet.
    pub(super) fn flush_details(&mut self) {
        let pending = self
            .details
            .as_ref()
            .and_then(|panel| Some((panel.node()?, panel.pending(&self.map))));
        if let Some((id, edits)) = pending {
            self.apply_details(id, &edits);
        }
    }

    /// Show the details side panel for the selected node.
    pub(super) fn details_panel(&mut self, ui: &mut Ui) {
        // Taken out so `self` can be used freely below; put back at the end.
        let Some(mut panel) = self.details.take() else {
            return;
        };
        if panel.node() != self.selected {
            if let Some(id) = panel.node() {
                self.apply_details(id, &panel.pending(&self.map));
            }
            panel.follow(&self.map, self.selected);
        }
        let outcome = egui::Panel::right("details")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| panel.show(ui, &self.map))
            .inner;
        if let Some(id) = panel.node() {
            if outcome.commit {
                self.apply_details(id, &panel.pending(&self.map));
            }
            if let Some(color) = outcome.color {
                self.color_selection(color);
            }
        }
        if let Some(address) = outcome.open_link {
            ui.ctx().open_url(egui::OpenUrl::new_tab(address));
        }
        self.details = Some(panel);
    }

    /// Act on a find-bar command: jump to the first, next or previous match, or close.
    pub(super) fn search_command(&mut self, command: SearchCommand) {
        let Some(search) = &mut self.search else {
            return;
        };
        search.refresh(&self.map, self.revision);
        let target = match command {
            SearchCommand::QueryChanged => search.first(),
            SearchCommand::Next => search.step(true),
            SearchCommand::Previous => search.step(false),
            SearchCommand::Close => {
                self.search = None;
                return;
            }
        };
        if let Some(id) = target {
            self.show_node(id);
        }
    }
}
