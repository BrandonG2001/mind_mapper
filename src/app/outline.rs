//! The outline view: the map as an indented list in a side panel (Ctrl+Shift+O), kept in
//! step with the canvas. Clicking a row selects its node (and shows it on the canvas),
//! double-clicking renames it in place, and the +/- buttons fold branches.

use eframe::egui::{self, Key, ScrollArea, TextEdit, Ui};

use super::canvas::take_plain_enter;
use super::{EditSession, MindMapApp};
use crate::model::{MindMap, NodeId};

/// Indentation per level of the tree, in points.
const INDENT: f32 = 16.0;

/// The outline panel's state while it is open.
#[derive(Debug, Default)]
pub(super) struct Outline {
    /// The rows in order: every node shown (folded branches hide theirs) and its depth.
    rows: Vec<(NodeId, u16)>,
    /// What the rows were built from: the map's revision and the focused branch.
    built_from: Option<(u64, Option<NodeId>)>,
    /// The selection the list last scrolled to, so it follows selection made elsewhere.
    followed: Option<NodeId>,
}

impl Outline {
    /// Rebuild the rows if the map, or the branch shown in focus mode, changed since they
    /// were built. Rows are only rebuilt on change, so a big map costs nothing per frame.
    fn refresh(&mut self, map: &MindMap, revision: u64, focus: Option<NodeId>) {
        let key = (revision, focus);
        if self.built_from == Some(key) {
            return;
        }
        self.built_from = Some(key);
        let start = focus
            .filter(|&id| map.node(id).is_some())
            .unwrap_or_else(|| map.root());
        // Depth of each node by slot; parents are visited before their children.
        let mut depth = vec![0_u16; map.slot_count()];
        self.rows.clear();
        for visit in map.visible_subtree_preorder(start) {
            let level = match visit.node.parent() {
                Some(parent) if visit.id != start => depth[parent.index()].saturating_add(1),
                _ => 0,
            };
            depth[visit.id.index()] = level;
            self.rows.push((visit.id, level));
        }
    }

    /// The row showing `id`, if it is in the list.
    fn row_of(&self, id: NodeId) -> Option<usize> {
        self.rows.iter().position(|&(row, _)| row == id)
    }
}

/// Something done in the outline, carried out once the panel has been drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutlineEvent {
    /// A row was clicked; `add` when Ctrl was held (add to or remove from the selection).
    Select { id: NodeId, add: bool },
    /// A row was double-clicked.
    Rename(NodeId),
    /// A row's +/- button was clicked.
    Fold(NodeId),
    /// The label being edited in the list was finished: kept, or abandoned with Esc.
    FinishEdit { keep: bool },
}

impl MindMapApp {
    /// Show or hide the outline panel.
    pub(super) fn toggle_outline(&mut self) {
        if self.outline.take().is_none() {
            self.outline = Some(Outline::default());
        }
    }

    /// Draw the outline panel, if open, and act on what was done in it.
    pub(super) fn outline_panel(&mut self, ui: &mut Ui) {
        // Taken out so `self` can be used freely below; put back at the end.
        let Some(mut outline) = self.outline.take() else {
            return;
        };
        outline.refresh(&self.map, self.revision, self.view.focus());
        // Scroll to the selected node when it was selected somewhere else.
        let follow = if self.selected == outline.followed {
            None
        } else {
            self.selected.and_then(|id| outline.row_of(id))
        };
        outline.followed = self.selected;
        let mut events = Vec::new();
        egui::Panel::left("outline")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                ui.strong("Outline");
                let row_height = ui.spacing().interact_size.y;
                let mut area = ScrollArea::both().auto_shrink(false);
                if let Some(index) = follow {
                    let pitch = row_height + ui.spacing().item_spacing.y;
                    let offset = row_offset(index, pitch) - ui.available_height() * 0.5;
                    area = area.vertical_scroll_offset(offset.max(0.0));
                }
                area.show_rows(ui, row_height, outline.rows.len(), |ui, range| {
                    for &(id, depth) in &outline.rows[range] {
                        ui.horizontal(|ui| {
                            ui.add_space(f32::from(depth) * INDENT);
                            self.outline_row(ui, id, &mut events);
                        });
                    }
                });
            });
        self.outline = Some(outline);
        for event in events {
            self.apply_outline_event(event);
        }
    }

    /// One row of the outline: fold button, then the label or its editor.
    fn outline_row(&mut self, ui: &mut Ui, id: NodeId, events: &mut Vec<OutlineEvent>) {
        let Some(node) = self.map.node(id) else {
            return;
        };
        let fold_width = ui.spacing().interact_size.y;
        if node.children().is_empty() {
            ui.add_space(fold_width);
        } else {
            let (mark, hint) = if node.is_collapsed() {
                ("+", "Expand")
            } else {
                ("-", "Collapse")
            };
            if ui.small_button(mark).on_hover_text(hint).clicked() {
                events.push(OutlineEvent::Fold(id));
            }
        }
        let renaming_here = self
            .editing
            .as_ref()
            .is_some_and(|session| session.in_outline && session.id == id);
        if renaming_here {
            if let Some(session) = &mut self.editing {
                outline_editor(ui, session, events);
            }
            return;
        }
        let selected = self.is_selected(id);
        let text = node.text();
        let mut lines = text.lines();
        let first = lines.next().unwrap_or_default();
        let label = if lines.next().is_some() {
            format!("{first} ...")
        } else {
            first.to_owned()
        };
        let response = ui.selectable_label(selected, label);
        if response.double_clicked() {
            events.push(OutlineEvent::Rename(id));
        } else if response.clicked() {
            let add = ui.input(|input| input.modifiers.command);
            events.push(OutlineEvent::Select { id, add });
        }
    }

    /// Carry out something done in the outline.
    pub(super) fn apply_outline_event(&mut self, event: OutlineEvent) {
        match event {
            OutlineEvent::Select { id, add } => {
                self.commit_edit();
                if add {
                    self.toggle_selected(id);
                } else {
                    self.select(Some(id));
                }
                self.reveal = Some(id);
                // Clicked in the list itself: no need to scroll the list to it.
                if let Some(outline) = &mut self.outline {
                    outline.followed = self.selected;
                }
            }
            OutlineEvent::Rename(id) => {
                self.commit_edit();
                let text = self.map.node(id).map(|node| node.text().to_owned());
                if let Some(text) = text {
                    self.select(Some(id));
                    self.start_edit(id, text);
                    if let Some(session) = &mut self.editing {
                        session.in_outline = true;
                    }
                }
            }
            OutlineEvent::Fold(id) => self.toggle_collapse(id),
            OutlineEvent::FinishEdit { keep: true } => self.commit_edit(),
            OutlineEvent::FinishEdit { keep: false } => self.editing = None,
        }
    }
}

/// The label editor in the row of the node being renamed in the outline. Enter (without
/// Shift) keeps the new label, Shift+Enter starts a new line, and Esc abandons it.
fn outline_editor(ui: &mut Ui, session: &mut EditSession, events: &mut Vec<OutlineEvent>) {
    let editor_id = ui.make_persistent_id("outline_label_editor");
    if ui.memory(|memory| memory.has_focus(editor_id))
        && ui.input_mut(|input| take_plain_enter(&mut input.events))
    {
        events.push(OutlineEvent::FinishEdit { keep: true });
    }
    let rows = session.buffer.lines().count().max(1);
    let response = ui.add(
        TextEdit::multiline(&mut session.buffer)
            .id(editor_id)
            // Also the field's name for screen readers.
            .hint_text("Node label")
            .desired_rows(rows)
            .desired_width(f32::INFINITY),
    );
    if session.request_focus {
        response.request_focus();
        session.request_focus = false;
    }
    if response.lost_focus() {
        let keep = !ui.input(|input| input.key_pressed(Key::Escape));
        events.push(OutlineEvent::FinishEdit { keep });
    }
}

/// Distance from the top of the list to row `index`, for rows `pitch` points apart.
#[allow(clippy::cast_precision_loss)]
// Only used to scroll; being off by a fraction of a row on lists of millions is fine.
fn row_offset(index: usize, pitch: f32) -> f32 {
    index as f32 * pitch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_list_shown_nodes_with_their_depth_and_rebuild_on_change() {
        let mut map = MindMap::new("root");
        let root = map.root();
        let a = map.add_child(root, "a").expect("root");
        let a1 = map.add_child(a, "a1").expect("a");
        let b = map.add_child(root, "b").expect("root");
        let mut outline = Outline::default();

        outline.refresh(&map, 1, None);
        assert_eq!(outline.rows, [(root, 0), (a, 1), (a1, 2), (b, 1)]);
        assert_eq!(outline.row_of(b), Some(3));

        // Same revision: the rows are kept as they are, without walking the map.
        map.set_collapsed(a, true).expect("a");
        outline.refresh(&map, 1, None);
        assert_eq!(outline.rows.len(), 4);
        // A new revision: folded children leave the list.
        outline.refresh(&map, 2, None);
        assert_eq!(outline.rows, [(root, 0), (a, 1), (b, 1)]);
        // Focus mode lists just the focused branch, starting at depth 0.
        map.set_collapsed(a, false).expect("a");
        outline.refresh(&map, 3, Some(a));
        assert_eq!(outline.rows, [(a, 0), (a1, 1)]);
    }
}
