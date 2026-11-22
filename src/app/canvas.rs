//! The map canvas: drawing, mouse interaction, the inline label editor and the
//! right-click menu.

use eframe::egui::{
    self, Align2, Event, FontId, Key, PointerButton, Rect, Response, Sense, StrokeKind, Ui, Vec2,
};

use super::{
    Action, Drag, EDITOR_PADDING, MIN_EDITOR_WIDTH, MindMapApp, NEW_NODE_TEXT, SCROLL_ZOOM_SPEED,
};
use crate::details::{self, DetailsPanel, color_palette};
use crate::map_view::{BASE_FONT_SIZE, LINE_HEIGHT};
use crate::model::{Node, NodeId};
use crate::navigation::{self, Direction};

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

    /// Selection, node dragging, box selection and panning. Uses the previous frame's
    /// layout for hit testing, so the node under the cursor is the one the user saw.
    pub(super) fn handle_pointer(&mut self, ui: &Ui, response: &Response) {
        let modifiers = ui.input(|input| input.modifiers);
        if response.drag_started() {
            let origin = ui
                .input(|input| input.pointer.press_origin())
                .filter(|_| response.dragged_by(PointerButton::Primary));
            let target = origin.and_then(|origin| self.view.hit_test(origin));
            self.drag = Some(match (target, origin) {
                (Some(id), _) => {
                    // One undo step covers both the move and any re-attachment on drop.
                    self.history.record(self.map.clone());
                    // Dragging a selected node moves the whole selection with it.
                    self.focus(id);
                    Drag::Nodes {
                        branches: self.selected_branches(),
                        drop_target: None,
                    }
                }
                (None, Some(origin)) if modifiers.shift => Drag::Select { origin },
                (None, _) => Drag::Pan,
            });
        }
        // `take` moves the drag out, so `self` can be borrowed freely; it is put back.
        if response.dragged()
            && let Some(drag) = self.drag.take()
        {
            let delta = response.drag_delta();
            self.drag = Some(match drag {
                Drag::Nodes { branches, .. } => {
                    let world = delta / self.view.camera().zoom();
                    let moved = branches.iter().try_for_each(|&branch| {
                        self.map.translate_subtree(branch, world.x, world.y)
                    });
                    if moved.is_ok() {
                        self.mark_dirty();
                    }
                    let drop_target = response
                        .interact_pointer_pos()
                        .and_then(|pos| self.find_drop_target(&branches, pos));
                    Drag::Nodes {
                        branches,
                        drop_target,
                    }
                }
                Drag::Pan => {
                    self.view.camera_mut().pan(delta);
                    Drag::Pan
                }
                // `name @ pattern` keeps the whole matched value under `name`.
                select @ Drag::Select { .. } => select,
            });
        }
        if response.drag_stopped() {
            match self.drag.take() {
                Some(Drag::Nodes {
                    branches,
                    drop_target: Some(new_parent),
                }) => self.reparent(&branches, new_parent),
                Some(Drag::Select { origin }) => {
                    if let Some(end) = ui.input(|input| input.pointer.latest_pos()) {
                        let boxed: Vec<NodeId> = self
                            .view
                            .nodes_in(Rect::from_two_pos(origin, end))
                            .collect();
                        // Ctrl+Shift+drag adds to the selection.
                        self.select_many(boxed, modifiers.command);
                    }
                }
                Some(Drag::Nodes { .. } | Drag::Pan) | None => {}
            }
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
            match hit {
                Some(id) if modifiers.command => self.toggle_selected(id),
                // Ctrl+click on empty canvas keeps the selection.
                None if modifiers.command => {}
                _ => self.select(hit),
            }
        }
    }

    /// The node under `screen` that the `dragged` branches could be dropped onto: anything
    /// outside them, except their parent when they all share it (dropping there would
    /// change nothing).
    pub(super) fn find_drop_target(
        &self,
        dragged: &[NodeId],
        screen: egui::Pos2,
    ) -> Option<NodeId> {
        let parent_of = |id: NodeId| self.map.node(id).and_then(Node::parent);
        self.view.hit_test_where(screen, |candidate| {
            let changes_nothing = dragged
                .iter()
                .all(|&branch| parent_of(branch) == Some(candidate));
            let inside = dragged
                .iter()
                .any(|&branch| self.map.is_in_subtree(candidate, branch));
            !changes_nothing && !inside
        })
    }

    /// Re-attach the `branches` under `new_parent` (any already there stay put). The undo
    /// point was recorded when the drag began.
    pub(super) fn reparent(&mut self, branches: &[NodeId], new_parent: NodeId) {
        // Unfold the target so the dropped branches stay visible.
        if self.map.node(new_parent).is_some_and(Node::is_collapsed)
            && let Err(err) = self.map.set_collapsed(new_parent, false)
        {
            self.status = err.to_string();
            return;
        }
        let map = &mut self.map;
        let moved = branches.iter().try_for_each(|&branch| {
            if map.node(branch).and_then(Node::parent) == Some(new_parent) {
                Ok(())
            } else {
                map.move_subtree(branch, new_parent)
            }
        });
        match moved {
            Ok(()) => self.mark_dirty(),
            Err(err) => self.status = err.to_string(),
        }
    }

    /// Select only `target` (if any) and then run `action` on it.
    pub(super) fn perform_on(&mut self, target: Option<NodeId>, action: Action) {
        if target.is_some() {
            self.select(target);
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
        self.check_focus();
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
        // The closure borrows only these two fields, leaving `self.view` free to change.
        let (main, others) = (self.selected, &self.also_selected);
        self.view.paint(&painter, ui.visuals(), &self.map, |id| {
            main == Some(id) || others.contains(&id)
        });
        self.describe_nodes(ui, &response);
        if let Some(search) = &self.search {
            self.view.outline_where(
                &painter,
                |id| search.is_match(id),
                ui.visuals().warn_fg_color,
            );
        }
        match &self.drag {
            Some(Drag::Nodes {
                drop_target: Some(target),
                ..
            }) => self
                .view
                .outline(&painter, *target, ui.visuals().selection.stroke.color),
            Some(Drag::Select { origin }) => {
                if let Some(end) = ui.input(|input| input.pointer.latest_pos()) {
                    let selection = ui.visuals().selection;
                    let area = Rect::from_two_pos(*origin, end);
                    painter.rect_filled(area, 0.0, selection.bg_fill.gamma_multiply(0.25));
                    painter.rect_stroke(area, 0.0, selection.stroke, StrokeKind::Inside);
                }
            }
            Some(Drag::Nodes { .. } | Drag::Pan) | None => {}
        }
        if response.secondary_clicked() {
            self.context_node = response
                .interact_pointer_pos()
                .and_then(|pos| self.view.hit_test(pos));
            if let Some(id) = self.context_node {
                // Right-clicking a selected node keeps the others for the menu's actions.
                self.focus(id);
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

    /// The right-click menu of node `id`: colour, notes, rename, reorder and delete.
    /// Colour and delete apply to every selected node.
    pub(super) fn node_menu(&mut self, ui: &mut Ui, id: NodeId) {
        let Some(current) = self.map.node(id).map(Node::color) else {
            return;
        };
        ui.label(egui::RichText::new("Colour").strong());
        if let Some(color) = color_palette(ui, current) {
            self.focus(id);
            self.color_selection(color);
            ui.close();
        }
        ui.separator();
        if ui.button("Notes and link...").clicked() {
            self.details.get_or_insert_with(DetailsPanel::default);
            ui.close();
        }
        let root = self.map.root();
        let can_move = |direction| navigation::neighbor(&self.map, id, direction).is_some();
        let focus_label = if self.view.focus().is_some() {
            "Show whole map"
        } else {
            "Focus on this branch"
        };
        let menu_actions = [
            ("Rename", Action::Rename, true, "F2"),
            (
                focus_label,
                Action::ToggleFocus,
                id != root || self.view.focus().is_some(),
                "F6",
            ),
            (
                "Move up",
                Action::Reorder(Direction::Up),
                can_move(Direction::Up),
                "Alt+Up",
            ),
            (
                "Move down",
                Action::Reorder(Direction::Down),
                can_move(Direction::Down),
                "Alt+Down",
            ),
            (
                "Delete",
                Action::Delete,
                self.selection().any(|selected| selected != root),
                "Delete",
            ),
        ];
        for (label, action, enabled, shortcut) in menu_actions {
            let button = egui::Button::new(label).shortcut_text(shortcut);
            if ui.add_enabled(enabled, button).clicked() {
                self.focus(id);
                self.perform(action);
                ui.close();
            }
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
