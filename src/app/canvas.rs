//! The map canvas: drawing, mouse interaction, the inline label editor and the
//! right-click menu.

use eframe::egui::{
    self, Align2, Event, FontId, Key, PointerButton, Rect, Response, Sense, Ui, Vec2,
};

use super::{
    Action, Drag, EDITOR_PADDING, MIN_EDITOR_WIDTH, MindMapApp, NEW_NODE_TEXT, SCROLL_ZOOM_SPEED,
};
use crate::details::{self, DetailsEdit, DetailsPanel, color_palette};
use crate::map_view::{BASE_FONT_SIZE, LINE_HEIGHT};
use crate::model::{Node, NodeId};

impl MindMapApp {
    pub(super) fn handle_zoom(&mut self, ui: &Ui, response: &Response) {
        if !response.hovered() {
            return;
        }
        let (scroll, pinch, pointer) = ui.input(|input| {
            (
                input.smooth_scroll_delta().y,
                input.zoom_delta(),
                input.pointer.hover_pos(),
            )
        });
        let factor = pinch * (scroll * SCROLL_ZOOM_SPEED).exp();
        if (factor - 1.0).abs() > f32::EPSILON
            && let Some(anchor) = pointer
        {
            self.view.camera_mut().zoom_at(anchor, factor);
        }
    }

    /// Selection, node dragging and panning. Uses the previous frame's layout for hit
    /// testing, so the node under the cursor is the one the user saw.
    pub(super) fn handle_pointer(&mut self, ui: &Ui, response: &Response) {
        if response.drag_started() {
            let target = ui
                .input(|input| input.pointer.press_origin())
                .filter(|_| response.dragged_by(PointerButton::Primary))
                .and_then(|origin| self.view.hit_test(origin));
            self.drag = Some(match target {
                Some(id) => {
                    // One undo step covers both the move and any re-attachment on drop.
                    self.history.record(self.map.clone());
                    self.selected = Some(id);
                    Drag::Node {
                        id,
                        drop_target: None,
                    }
                }
                None => Drag::Pan,
            });
        }
        if response.dragged() {
            let delta = response.drag_delta();
            match self.drag {
                Some(Drag::Node { id, .. }) => {
                    let world_delta = delta / self.view.camera().zoom();
                    if self
                        .map
                        .translate_subtree(id, world_delta.x, world_delta.y)
                        .is_ok()
                    {
                        self.mark_dirty();
                    }
                    let drop_target = response
                        .interact_pointer_pos()
                        .and_then(|pos| self.find_drop_target(id, pos));
                    self.drag = Some(Drag::Node { id, drop_target });
                }
                Some(Drag::Pan) => self.view.camera_mut().pan(delta),
                None => {}
            }
        }
        if response.drag_stopped() {
            if let Some(Drag::Node {
                id,
                drop_target: Some(new_parent),
            }) = self.drag
            {
                self.reparent(id, new_parent);
            }
            self.drag = None;
        }

        let pointer = response.interact_pointer_pos();
        if response.clicked()
            && let Some(id) = pointer.and_then(|pos| self.view.hit_test_handle(pos))
        {
            self.commit_edit();
            self.toggle_collapse(id);
            return;
        }
        let hit = pointer.and_then(|pos| self.view.hit_test(pos));
        if response.double_clicked() {
            self.perform_on(hit, Action::Rename);
        } else if response.clicked() {
            self.commit_edit();
            self.selected = hit;
        }
    }

    /// The node under `screen` that `dragged` could be dropped onto: anything except the
    /// dragged branch itself and its current parent (dropping there would change nothing).
    pub(super) fn find_drop_target(&self, dragged: NodeId, screen: egui::Pos2) -> Option<NodeId> {
        let current_parent = self.map.node(dragged).and_then(Node::parent);
        self.view.hit_test_where(screen, |candidate| {
            Some(candidate) != current_parent && !self.map.is_in_subtree(candidate, dragged)
        })
    }

    /// Re-attach `id` under `new_parent`. The undo point was recorded when the drag began.
    pub(super) fn reparent(&mut self, id: NodeId, new_parent: NodeId) {
        // Unfold the target so the dropped branch stays visible.
        if self.map.node(new_parent).is_some_and(Node::is_collapsed)
            && let Err(err) = self.map.set_collapsed(new_parent, false)
        {
            self.status = err.to_string();
            return;
        }
        match self.map.move_subtree(id, new_parent) {
            Ok(()) => self.mark_dirty(),
            Err(err) => self.status = err.to_string(),
        }
    }

    /// Select `target` (if any) and then run `action` on it.
    pub(super) fn perform_on(&mut self, target: Option<NodeId>, action: Action) {
        if target.is_some() {
            self.selected = target;
            self.perform(action);
        }
    }

    /// Show the in-place text box for the node being edited.
    pub(super) fn show_editor(&mut self, ui: &mut Ui) {
        let Some(session) = self.editing.as_mut() else {
            return;
        };
        let Some(node_rect) = self.view.node_rect(session.id) else {
            // The node scrolled out of view (or was deleted): finish the edit.
            self.commit_edit();
            return;
        };
        // Enter saves the label; Shift+Enter falls through to the text box as a new line.
        let editor_id = ui.make_persistent_id("node_label_editor");
        if ui.memory(|memory| memory.has_focus(editor_id))
            && ui.input_mut(|input| take_plain_enter(&mut input.events))
        {
            self.commit_edit();
            return;
        }
        let zoom = self.view.camera().zoom();
        // Grow with the text: one row per line, plus one while a new line is started.
        let rows =
            session.buffer.lines().count().max(1) + usize::from(session.buffer.ends_with('\n'));
        let rows = f32::from(u16::try_from(rows).unwrap_or(u16::MAX));
        let size = Vec2::new(
            node_rect.width().max(MIN_EDITOR_WIDTH * zoom),
            node_rect
                .height()
                .max((rows * LINE_HEIGHT + 2.0 * EDITOR_PADDING) * zoom),
        );
        let editor = egui::TextEdit::multiline(&mut session.buffer)
            .id(editor_id)
            .font(FontId::proportional(BASE_FONT_SIZE * zoom))
            .hint_text(NEW_NODE_TEXT)
            .align(Align2::CENTER_CENTER);
        let response = ui.put(Rect::from_center_size(node_rect.center(), size), editor);
        if session.request_focus {
            response.request_focus();
            session.request_focus = false;
        }
        if response.lost_focus() {
            if ui.input(|input| input.key_pressed(Key::Escape)) {
                self.editing = None;
            } else {
                self.commit_edit();
            }
        }
    }

    pub(super) fn canvas(&mut self, ui: &mut Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        if self.needs_fit {
            self.view.fit(&self.map, response.rect);
            self.needs_fit = false;
        }
        if let Some(id) = self.reveal.take() {
            self.scroll_into_view(id, response.rect);
        }
        painter.rect_filled(response.rect, 0.0, ui.visuals().extreme_bg_color);
        self.handle_zoom(ui, &response);
        self.handle_pointer(ui, &response);
        self.view
            .paint(&painter, ui.visuals(), &self.map, self.selected);
        if let Some(search) = &self.search {
            self.view.outline_where(
                &painter,
                |id| search.is_match(id),
                ui.visuals().warn_fg_color,
            );
        }
        if let Some(Drag::Node {
            drop_target: Some(target),
            ..
        }) = self.drag
        {
            self.view
                .outline(&painter, target, ui.visuals().selection.stroke.color);
        }
        if response.secondary_clicked() {
            self.context_node = response
                .interact_pointer_pos()
                .and_then(|pos| self.view.hit_test(pos));
            if self.context_node.is_some() {
                self.selected = self.context_node;
            }
        }
        if let Some(id) = self.context_node {
            response.context_menu(|ui| self.node_menu(ui, id));
        }
        let hovered_details = response
            .hover_pos()
            .filter(|_| self.drag.is_none())
            .and_then(|pos| self.view.hit_test(pos))
            .and_then(|id| details::hover_text(&self.map, id));
        if let Some(text) = hovered_details {
            response.on_hover_text_at_pointer(text);
        }
        self.show_editor(ui);
    }

    /// The right-click menu of node `id`: colour, notes, rename and delete.
    pub(super) fn node_menu(&mut self, ui: &mut Ui, id: NodeId) {
        let Some(current) = self.map.node(id).map(Node::color) else {
            return;
        };
        ui.label(egui::RichText::new("Colour").strong());
        if let Some(color) = color_palette(ui, current) {
            self.apply_details(id, &[DetailsEdit::Color(color)]);
            ui.close();
        }
        ui.separator();
        if ui.button("Notes and link...").clicked() {
            self.details.get_or_insert_with(DetailsPanel::default);
            ui.close();
        }
        if ui.button("Rename").clicked() {
            self.selected = Some(id);
            self.perform(Action::Rename);
            ui.close();
        }
        let deletable = id != self.map.root();
        if ui
            .add_enabled(deletable, egui::Button::new("Delete"))
            .clicked()
        {
            self.selected = Some(id);
            self.perform(Action::Delete);
            ui.close();
        }
    }

    /// Pan (without zooming) so `id` is comfortably inside `canvas`, if it isn't already.
    pub(super) fn scroll_into_view(&mut self, id: NodeId, canvas: Rect) {
        let Some(position) = self.map.node(id).map(Node::position) else {
            return;
        };
        let on_screen = self.view.camera().world_to_screen(position);
        // Keep a margin so the whole node box, not just its centre, stays visible.
        let comfortable = canvas.shrink2(canvas.size() * 0.15);
        if !comfortable.contains(on_screen) {
            self.view
                .camera_mut()
                .look_at(position, comfortable.clamp(on_screen));
        }
    }
}

/// Remove plain Enter presses (without Shift) from `events`.
///
/// # Returns
///
/// Whether any were removed.
pub(super) fn take_plain_enter(events: &mut Vec<Event>) -> bool {
    let before = events.len();
    events.retain(|event| {
        !matches!(
            event,
            Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift
        )
    });
    events.len() != before
}
