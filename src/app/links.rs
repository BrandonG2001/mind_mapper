//! Cross-links between nodes: starting one (Ctrl+K or the right-click menu), finishing
//! it with a click on the target, labelling and removing links.

use eframe::egui::{self, Key, Modifiers};

use super::{MindMapApp, node_label};
use crate::model::NodeId;

/// Widest the link label editor gets, in points.
const LABEL_EDITOR_WIDTH: f32 = 320.0;

/// A link label being edited in a pop-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LabelEdit {
    /// The link's two ends (in either order).
    ends: (NodeId, NodeId),
    /// The text being typed.
    buffer: String,
    /// Whether the text field still needs keyboard focus (on the first frame).
    request_focus: bool,
}

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

    /// Open the label editor for the link between `a` and `b`, starting from its label.
    pub(super) fn edit_link_label(&mut self, a: NodeId, b: NodeId) {
        let current = self
            .map
            .cross_links()
            .iter()
            .find(|link| link.joins(a, b))
            .map(|link| link.label().to_owned());
        if let Some(buffer) = current {
            self.label_edit = Some(LabelEdit {
                ends: (a, b),
                buffer,
                request_focus: true,
            });
        }
    }

    /// Draw the link label editor, if open, over the window. Enter or Save keeps the text
    /// (empty removes the label); Esc, Cancel or a click outside closes it unchanged.
    pub(super) fn show_label_editor(&mut self, ctx: &egui::Context) {
        let Some(edit) = &mut self.label_edit else {
            return;
        };
        let (a, b) = edit.ends;
        let title = format!(
            "Label for the link between \"{}\" and \"{}\"",
            node_label(&self.map, a),
            node_label(&self.map, b)
        );
        let response = egui::Modal::new(egui::Id::unique("link_label")).show(ctx, |ui| {
            ui.set_max_width(LABEL_EDITOR_WIDTH);
            ui.label(title);
            let field = ui.add(
                egui::TextEdit::singleline(&mut edit.buffer)
                    .hint_text("e.g. depends on")
                    .desired_width(LABEL_EDITOR_WIDTH),
            );
            if edit.request_focus {
                field.request_focus();
                edit.request_focus = false;
            }
            let entered = field.lost_focus()
                && ui.input(|input| {
                    input.key_pressed(Key::Enter) && input.modifiers == Modifiers::NONE
                });
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() || entered {
                    Some(true)
                } else if ui.button("Cancel").clicked() {
                    Some(false)
                } else {
                    None
                }
            })
            .inner
        });
        let save = response
            .inner
            .or_else(|| response.should_close().then_some(false));
        let Some(save) = save else {
            return;
        };
        if let Some(edit) = self.label_edit.take()
            && save
        {
            let (a, b) = edit.ends;
            if self
                .edit_map(|map| map.set_cross_link_label(a, b, &edit.buffer))
                .is_some()
            {
                "Link label saved".clone_into(&mut self.status);
            }
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
