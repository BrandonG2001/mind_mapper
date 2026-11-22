//! The shortcut overview (F1): every keyboard shortcut and mouse action, in a window.
//!
//! The keys come from the same [`SHORTCUTS`] table that handles them, so the overview
//! cannot fall out of date; each action's description is required by an exhaustive match.

use eframe::egui::{self, Grid, RichText, Ui};

use super::{Action, MindMapApp, SHORTCUTS};
use crate::navigation::Direction;

/// A group of entries in the overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Files,
    Editing,
    Moving,
    View,
    Find,
}

impl Section {
    /// Every section, in the order shown.
    const ALL: [Self; 5] = [
        Self::Files,
        Self::Editing,
        Self::Moving,
        Self::View,
        Self::Find,
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Editing => "Editing",
            Self::Moving => "Moving around",
            Self::View => "View",
            Self::Find => "Find",
        }
    }
}

/// Keys handled outside [`SHORTCUTS`] (by text fields, egui or the clipboard).
const OTHER_KEYS: [(Section, &str, &str); 3] = [
    (
        Section::Editing,
        "Ctrl+C / Ctrl+X / Ctrl+V",
        "Copy, cut or paste branches as outlines; paste a copied image onto the selected node",
    ),
    (
        Section::Editing,
        "Shift+Enter",
        "Start a new line while editing a label",
    ),
    (
        Section::View,
        "Ctrl + / Ctrl - / Ctrl 0",
        "Make the interface larger, smaller, or reset it to 100%",
    ),
];

/// What the mouse does.
const MOUSE: [(&str, &str); 10] = [
    ("Click a node", "Select it"),
    (
        "Ctrl+click a node",
        "Add it to the selection, or take it out",
    ),
    ("Double-click a node", "Rename it"),
    (
        "Drag a node",
        "Move it with its branch; drop it on another node to attach it there",
    ),
    ("Shift+drag on empty canvas", "Select the nodes in a box"),
    (
        "Right-click a node",
        "Colour, notes, links, pictures, focus, reordering and more",
    ),
    ("Drag empty canvas", "Move the view"),
    ("Scroll wheel", "Zoom in and out"),
    ("Drop a PNG or JPEG file on a node", "Add the picture to it"),
    (
        "Drag a row in the outline",
        "Move its branch: onto a row's middle to attach it, near an edge to place it beside",
    ),
];

impl Action {
    /// Where the overview lists this action, and what it does.
    const fn help(self) -> (Section, &'static str) {
        match self {
            Self::New => (Section::Files, "New map, from a template"),
            Self::Open => (
                Section::Files,
                "Open a map, or import an OPML or FreeMind file",
            ),
            Self::Save => (Section::Files, "Save"),
            Self::SaveAs => (Section::Files, "Save as a new file"),
            Self::Export => (
                Section::Files,
                "Export as PNG, SVG, PDF, Markdown, OPML or FreeMind",
            ),
            Self::ExportBranch => (Section::Files, "Export just the selected branch"),
            Self::SaveTemplate => (Section::Files, "Save the map as a template"),
            Self::Undo => (Section::Editing, "Undo"),
            Self::Redo => (Section::Editing, "Redo"),
            Self::AddChild => (Section::Editing, "Add a child to the selected node"),
            Self::AddSibling => (Section::Editing, "Add a sibling to the selected node"),
            Self::Rename => (Section::Editing, "Rename the selected node"),
            Self::Delete => (
                Section::Editing,
                "Delete the selected nodes and their branches",
            ),
            Self::Tidy => (
                Section::Editing,
                "Tidy up: arrange the map without overlaps",
            ),
            Self::StartLink => (
                Section::Editing,
                "Link the selected node to the next node clicked",
            ),
            Self::ToggleBold => (
                Section::Editing,
                "Make the selected nodes bold, or plain again",
            ),
            Self::Reorder(Direction::Up | Direction::Left) => {
                (Section::Editing, "Move the selected node above its sibling")
            }
            Self::Reorder(Direction::Down | Direction::Right) => {
                (Section::Editing, "Move the selected node below its sibling")
            }
            Self::Navigate(Direction::Up) => (Section::Moving, "Select the sibling above"),
            Self::Navigate(Direction::Down) => (Section::Moving, "Select the sibling below"),
            Self::Navigate(Direction::Left) => (
                Section::Moving,
                "Select leftwards: the parent on the right half, a child on the left",
            ),
            Self::Navigate(Direction::Right) => (
                Section::Moving,
                "Select rightwards: a child on the right half, the parent on the left",
            ),
            Self::CenterView => (Section::View, "Fit the whole map in the window"),
            Self::ToggleCollapse => (Section::View, "Fold or unfold the selected branch"),
            Self::ToggleFocus => (
                Section::View,
                "Show only the selected branch, or everything",
            ),
            Self::ToggleOutline => (Section::View, "Show or hide the outline"),
            Self::ToggleDetails => (
                Section::View,
                "Show or hide the colour, notes and link panel",
            ),
            Self::ToggleHelp => (Section::View, "Show or hide this overview"),
            Self::Find => (Section::Find, "Find text in labels and notes"),
            Self::Replace => (Section::Find, "Find and replace"),
            Self::FindNext => (Section::Find, "Next match"),
            Self::FindPrevious => (Section::Find, "Previous match"),
            Self::Dismiss => (
                Section::Find,
                "Cancel a link, close the find bar or this overview, or leave focus mode",
            ),
        }
    }
}

/// The overview window, while it is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Help;

impl MindMapApp {
    /// Show or hide the shortcut overview.
    pub(super) fn toggle_help(&mut self) {
        if self.help.take().is_none() {
            self.help = Some(Help);
        }
    }

    /// Draw the shortcut overview, if open.
    pub(super) fn show_help(&mut self, ctx: &egui::Context) {
        let mut open = self.help.is_some();
        egui::Window::new("Shortcuts and mouse")
            .open(&mut open)
            .default_width(560.0)
            .vscroll(true)
            .show(ctx, |ui| {
                for section in Section::ALL {
                    section_grid(ui, section);
                }
                ui.add_space(ui.spacing().item_spacing.y * 2.0);
                ui.label(RichText::new("Mouse").strong());
                Grid::new("help_mouse")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (gesture, effect) in MOUSE {
                            ui.label(gesture);
                            ui.label(effect);
                            ui.end_row();
                        }
                    });
            });
        // The window's close button clears `open`.
        if !open {
            self.help = None;
        }
    }
}

/// One section of the overview: its shortcuts, then its other keys.
fn section_grid(ui: &mut Ui, section: Section) {
    ui.label(RichText::new(section.title()).strong());
    Grid::new(("help", section.title()))
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for (shortcut, action) in &SHORTCUTS {
                let (listed_in, effect) = action.help();
                if listed_in == section {
                    ui.label(ui.ctx().format_shortcut(shortcut));
                    ui.label(effect);
                    ui.end_row();
                }
            }
            for (listed_in, keys, effect) in OTHER_KEYS {
                if listed_in == section {
                    ui.label(keys);
                    ui.label(effect);
                    ui.end_row();
                }
            }
        });
    ui.add_space(ui.spacing().item_spacing.y * 2.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_is_described_once_in_a_section() {
        for (shortcut, action) in &SHORTCUTS {
            let (section, effect) = action.help();
            assert!(Section::ALL.contains(&section));
            assert!(!effect.is_empty(), "{shortcut:?}");
        }
        // No two shortcuts are the same keys.
        for (index, (shortcut, _)) in SHORTCUTS.iter().enumerate() {
            assert!(
                SHORTCUTS[index + 1..]
                    .iter()
                    .all(|(other, _)| other != shortcut),
                "{shortcut:?} is listed twice"
            );
        }
    }
}
