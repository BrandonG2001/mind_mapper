//! The map's nodes in the accessibility tree. Nodes are painted shapes, which screen
//! readers cannot see on their own; this describes each drawn node as an item of a
//! "Mind map" tree, nested like the map. (UI tests find nodes the same way.)

use std::collections::HashMap;
// `write!` into a `String` needs this trait in scope.
use std::fmt::Write as _;

use eframe::egui::{Response, Ui, UiBuilder, accesskit};

use super::{MindMapApp, node_label};
use crate::model::NodeId;

impl MindMapApp {
    /// Describe every node drawn this frame: its label, notes, level, folded state and
    /// whether it is selected. Activating one from a screen reader selects it.
    pub(super) fn describe_nodes(&mut self, ui: &mut Ui, canvas: &Response) {
        let ctx = ui.ctx().clone();
        let tree = canvas.id;
        // `None` when accessibility support is off (no screen reader): nothing to do.
        let enabled = ctx.accesskit_node_builder(tree, |node| {
            node.set_role(accesskit::Role::Tree);
            node.set_label("Mind map");
        });
        if enabled.is_none() {
            return;
        }
        let item_id = |id: NodeId| tree.with(("map node", id.index()));
        // Level of each drawn node; parents are drawn (and so listed) before children.
        let mut levels: HashMap<NodeId, usize> = HashMap::new();
        let mut activated = None;
        for (id, rect) in self.view.drawn() {
            let Some(node) = self.map.node(id) else {
                continue;
            };
            let parent = node.parent().filter(|parent| levels.contains_key(parent));
            let level = parent.map_or(1, |parent| levels[&parent] + 1);
            levels.insert(id, level);
            let item = item_id(id);
            // A child `Ui` places the item in the tree (under its parent's item); it draws
            // nothing and takes no space, so mouse handling is unaffected.
            drop(
                ui.new_child(
                    UiBuilder::new()
                        .scope_id(item)
                        .max_rect(rect)
                        .accessibility_parent(parent.map_or(tree, item_id))
                        .accessibility_role(accesskit::Role::TreeItem)
                        .accessibility_label(node.text()),
                ),
            );
            let selected = self.is_selected(id);
            let description = self.node_description(id);
            ctx.accesskit_node_builder(item, |builder| {
                builder.set_bounds(accesskit::Rect {
                    x0: rect.min.x.into(),
                    y0: rect.min.y.into(),
                    x1: rect.max.x.into(),
                    y1: rect.max.y.into(),
                });
                builder.set_level(level);
                builder.set_selected(selected);
                if !node.children().is_empty() {
                    builder.set_expanded(!node.is_collapsed());
                }
                if !description.is_empty() {
                    builder.set_description(description);
                }
                builder.add_action(accesskit::Action::Click);
            });
            if ui.input(|input| input.has_accesskit_action_request(item, accesskit::Action::Click))
            {
                activated = Some(id);
            }
        }
        if activated.is_some() {
            self.select(activated);
        }
    }

    /// What a screen reader says about `id` after its label: its notes, then its links.
    fn node_description(&self, id: NodeId) -> String {
        let mut description = self
            .map
            .node(id)
            .map_or_else(String::new, |node| node.note().to_owned());
        for link in self
            .map
            .cross_links()
            .iter()
            .filter(|link| link.touches(id))
        {
            if !description.is_empty() {
                description.push('\n');
            }
            let (direction, other) = if link.from() == id {
                ("Links to", link.to())
            } else {
                ("Linked from", link.from())
            };
            write!(description, "{direction} {}", node_label(&self.map, other))
                .expect("writing into a String cannot fail");
            if !link.label().is_empty() {
                write!(description, " ({})", link.label())
                    .expect("writing into a String cannot fail");
            }
        }
        description
    }
}
