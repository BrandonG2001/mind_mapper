//! The details side panel: colour, notes and web link of the selected node.

use eframe::egui::{self, RichText, Ui, Vec2};

use crate::map_view::to_color32;
use crate::model::{MindMap, NodeColor, NodeId};

/// Colours offered for nodes: the ten Tableau 10 colours (the first six are also the
/// automatic branch colours).
pub const PALETTE: [NodeColor; 10] = [
    NodeColor::rgb(0x4e, 0x79, 0xa7),
    NodeColor::rgb(0xf2, 0x8e, 0x2b),
    NodeColor::rgb(0x59, 0xa1, 0x4f),
    NodeColor::rgb(0xe1, 0x57, 0x59),
    NodeColor::rgb(0xb0, 0x7a, 0xa1),
    NodeColor::rgb(0x76, 0xb7, 0xb2),
    NodeColor::rgb(0xed, 0xc9, 0x48),
    NodeColor::rgb(0xff, 0x9d, 0xa7),
    NodeColor::rgb(0x9c, 0x75, 0x5f),
    NodeColor::rgb(0xba, 0xb0, 0xac),
];
/// Side length of a colour swatch button.
const SWATCH_SIZE: f32 = 20.0;
/// Visible rows of the notes field.
const NOTE_ROWS: usize = 10;

/// A change made in the details panel, to apply to the node it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailsEdit {
    /// New notes text.
    Note(String),
    /// New web link (`None` removes it).
    Link(Option<String>),
    /// New colour (`None` = automatic).
    Color(Option<NodeColor>),
}

/// What the user did in the panel this frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailsOutcome {
    /// A text field was left, so its edits should be applied now.
    pub commit: bool,
    /// A colour was picked (`Some(None)` = back to automatic).
    pub color: Option<Option<NodeColor>>,
    /// "Open link" was clicked for this address.
    pub open_link: Option<String>,
}

/// Row of colour swatches plus "Auto".
///
/// # Arguments
///
/// * `ui` - Where to draw
/// * `current` - The colour currently chosen, which is outlined
///
/// # Returns
///
/// The colour clicked (`Some(None)` for "Auto"), if any.
pub fn color_palette(ui: &mut Ui, current: Option<NodeColor>) -> Option<Option<NodeColor>> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        if ui
            .selectable_label(current.is_none(), "Auto")
            .on_hover_text("Use the branch colour")
            .clicked()
        {
            picked = Some(None);
        }
        for color in PALETTE {
            let mut swatch = egui::Button::new("")
                .fill(to_color32(color))
                .min_size(Vec2::splat(SWATCH_SIZE));
            if current == Some(color) {
                swatch = swatch.stroke(egui::Stroke::new(2.0, ui.visuals().strong_text_color()));
            }
            let response = ui.add(swatch);
            // A swatch has no text, so name it by its colour for screen readers.
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Button,
                    true,
                    format!("Colour {}", String::from(color)),
                )
            });
            if response.clicked() {
                picked = Some(Some(color));
            }
        }
    });
    picked
}

/// Editing state of the details panel: the node shown and the text being edited.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailsPanel {
    node: Option<NodeId>,
    note: String,
    link: String,
}

impl DetailsPanel {
    /// The node whose details are shown.
    #[must_use]
    pub const fn node(&self) -> Option<NodeId> {
        self.node
    }

    /// Show `selected` from now on, loading its notes and link into the text fields.
    /// Apply [`DetailsPanel::pending`] first, or unsaved typing is lost.
    pub fn follow(&mut self, map: &MindMap, selected: Option<NodeId>) {
        self.node = selected.filter(|&id| map.node(id).is_some());
        let node = self.node.and_then(|id| map.node(id));
        self.note.clear();
        self.note.push_str(node.map_or("", |node| node.note()));
        self.link.clear();
        self.link
            .push_str(node.and_then(|node| node.link()).unwrap_or(""));
    }

    /// Text-field edits that differ from what the node currently holds.
    #[must_use]
    pub fn pending(&self, map: &MindMap) -> Vec<DetailsEdit> {
        let Some(node) = self.node.and_then(|id| map.node(id)) else {
            return Vec::new();
        };
        let mut edits = Vec::new();
        if self.note.trim_end() != node.note() {
            edits.push(DetailsEdit::Note(self.note.clone()));
        }
        let link = Some(self.link.trim()).filter(|link| !link.is_empty());
        if link != node.link() {
            edits.push(DetailsEdit::Link(link.map(str::to_owned)));
        }
        edits
    }

    /// Draw the panel's contents.
    pub fn show(&mut self, ui: &mut Ui, map: &MindMap) -> DetailsOutcome {
        let mut outcome = DetailsOutcome::default();
        let Some(node) = self.node.and_then(|id| map.node(id)) else {
            ui.weak("Select a node to see and edit its colour, notes and link.");
            return outcome;
        };
        ui.label(RichText::new(node.text()).heading());
        ui.add_space(8.0);

        ui.label(RichText::new("Colour").strong());
        outcome.color = color_palette(ui, node.color());
        ui.add_space(8.0);

        ui.label(RichText::new("Notes").strong());
        let note_field = ui.add(
            egui::TextEdit::multiline(&mut self.note)
                .hint_text("Anything worth remembering about this idea")
                .desired_rows(NOTE_ROWS)
                .desired_width(f32::INFINITY),
        );
        ui.add_space(8.0);

        ui.label(RichText::new("Link").strong());
        let link_field = ui.add(
            egui::TextEdit::singleline(&mut self.link)
                .hint_text("https://")
                .desired_width(f32::INFINITY),
        );
        let address = self.link.trim();
        let openable = address.starts_with("https://") || address.starts_with("http://");
        if ui
            .add_enabled(openable, egui::Button::new("Open link"))
            .on_disabled_hover_text("Enter an address starting with http:// or https://")
            .clicked()
        {
            outcome.open_link = Some(address.to_owned());
        }
        outcome.commit = note_field.lost_focus() || link_field.lost_focus();
        outcome
    }
}

/// Text shown when hovering a node that has notes or a link.
#[must_use]
pub fn hover_text(map: &MindMap, id: NodeId) -> Option<String> {
    let node = map.node(id)?;
    match (node.note().is_empty(), node.link()) {
        (true, None) => None,
        (false, None) => Some(node.note().to_owned()),
        (true, Some(link)) => Some(link.to_owned()),
        (false, Some(link)) => Some(format!("{}\n\n{link}", node.note())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_with_details() -> (MindMap, NodeId) {
        let mut map = MindMap::new("root");
        let id = map.add_child(map.root(), "idea").expect("root");
        map.set_note(id, "a note").expect("idea");
        map.set_link(id, Some("https://example.com")).expect("idea");
        (map, id)
    }

    #[test]
    fn follow_loads_and_pending_reports_changes() {
        let (map, id) = map_with_details();
        let mut panel = DetailsPanel::default();

        panel.follow(&map, Some(id));
        assert_eq!(panel.node(), Some(id));
        assert_eq!(panel.pending(&map), Vec::new());
        "changed".clone_into(&mut panel.note);
        "  ".clone_into(&mut panel.link);

        assert_eq!(
            panel.pending(&map),
            vec![
                DetailsEdit::Note("changed".to_owned()),
                DetailsEdit::Link(None)
            ]
        );
    }

    #[test]
    fn follow_ignores_missing_nodes() {
        let (mut map, id) = map_with_details();
        map.remove_subtree(id).expect("idea");
        let mut panel = DetailsPanel::default();

        panel.follow(&map, Some(id));

        assert_eq!(panel.node(), None);
        assert_eq!(panel.pending(&map), Vec::new());
    }

    #[test]
    fn hover_text_combines_note_and_link() {
        let (mut map, id) = map_with_details();
        assert_eq!(
            hover_text(&map, id).as_deref(),
            Some("a note\n\nhttps://example.com")
        );
        map.set_link(id, None).expect("idea");
        assert_eq!(hover_text(&map, id).as_deref(), Some("a note"));
        map.set_note(id, "").expect("idea");

        assert_eq!(hover_text(&map, id), None);
    }

    #[test]
    fn palette_colours_are_distinct() {
        for (index, color) in PALETTE.iter().enumerate() {
            assert!(!PALETTE[index + 1..].contains(color));
        }
    }
}
