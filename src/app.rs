//! The egui application: toolbar, canvas rendering and user interaction.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, FontId, Galley, Key, KeyboardShortcut, Modifiers, PointerButton, Pos2,
    Rect, Response, Sense, Stroke, StrokeKind, Theme, Ui, Vec2, ViewportCommand, Visuals,
    epaint::CubicBezierShape,
};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};

use crate::camera::Camera;
use crate::history::History;
use crate::model::{MapError, MindMap, Node, NodeId};
use crate::storage;

/// Font size of node labels at 100% zoom.
const BASE_FONT_SIZE: f32 = 16.0;
/// Space between a node's label and its border at 100% zoom.
const NODE_PADDING: Vec2 = Vec2::new(14.0, 8.0);
/// Labels wider than this (at 100% zoom) wrap onto several lines.
const MAX_LABEL_WIDTH: f32 = 240.0;
/// Narrowest text box shown while editing, at 100% zoom.
const MIN_EDITOR_WIDTH: f32 = 140.0;
/// Rounded-corner radius of node boxes at 100% zoom.
const CORNER_RADIUS: f32 = 8.0;
/// Number of undo steps kept.
const HISTORY_LIMIT: usize = 200;
/// How strongly one scroll-wheel pixel changes the zoom.
const SCROLL_ZOOM_SPEED: f32 = 0.0015;
/// Label of the root node in a new map.
const DEFAULT_ROOT_TEXT: &str = "Central idea";
/// Label given to nodes created from the toolbar or keyboard.
const NEW_NODE_TEXT: &str = "New idea";
/// File-dialog filter name and extension.
const FILE_FILTER: (&str, &[&str]) = ("Mind map", &["json"]);
/// Accent colours cycled through for the root's branches.
const BRANCH_COLORS: [Color32; 6] = [
    Color32::from_rgb(0x4e, 0x79, 0xa7),
    Color32::from_rgb(0xf2, 0x8e, 0x2b),
    Color32::from_rgb(0x59, 0xa1, 0x4f),
    Color32::from_rgb(0xe1, 0x57, 0x59),
    Color32::from_rgb(0xb0, 0x7a, 0xa1),
    Color32::from_rgb(0x76, 0xb7, 0xb2),
];

/// Something the user can trigger from the toolbar or the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    New,
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    AddChild,
    AddSibling,
    Rename,
    Delete,
    CenterView,
    ToggleTheme,
}

/// Keyboard shortcuts. Variants with Shift come first because a shortcut without Shift
/// also matches when Shift is held.
const SHORTCUTS: [(KeyboardShortcut, Action); 12] = [
    (
        KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S),
        Action::SaveAs,
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z),
        Action::Redo,
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::N),
        Action::New,
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::O),
        Action::Open,
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::S),
        Action::Save,
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Z),
        Action::Undo,
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Y),
        Action::Redo,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::Tab),
        Action::AddChild,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::Enter),
        Action::AddSibling,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::F2),
        Action::Rename,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::Delete),
        Action::Delete,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::Home),
        Action::CenterView,
    ),
];

/// A node's on-screen geometry for the current frame.
#[derive(Debug, Clone)]
struct LaidOutNode {
    id: NodeId,
    rect: Rect,
    galley: Arc<Galley>,
    accent: Color32,
}

/// A label that is being edited in place.
#[derive(Debug, Clone, PartialEq)]
struct EditSession {
    id: NodeId,
    buffer: String,
    request_focus: bool,
}

/// What the current mouse drag is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    /// Moving a node and its descendants.
    Node(NodeId),
    /// Moving the view.
    Pan,
}

/// The mind-mapping application state.
pub struct MindMapApp {
    map: MindMap,
    history: History<MindMap>,
    camera: Camera,
    selected: Option<NodeId>,
    editing: Option<EditSession>,
    drag: Option<Drag>,
    file_path: Option<PathBuf>,
    dirty: bool,
    dark_mode: bool,
    /// The view is centred on the root once the canvas size is known.
    needs_centering: bool,
    canvas_rect: Rect,
    /// Reused every frame to avoid reallocating.
    layout: Vec<LaidOutNode>,
    window_title: String,
    status: String,
}

impl MindMapApp {
    /// Create the app, optionally opening the map at `initial_path`.
    ///
    /// # Arguments
    ///
    /// * `cc` - eframe creation context, used to apply the dark theme
    /// * `initial_path` - File passed on the command line, if any. A load failure is shown
    ///   in the status bar and the app starts with an empty map instead.
    #[must_use]
    pub fn new(cc: &eframe::CreationContext<'_>, initial_path: Option<&Path>) -> Self {
        cc.egui_ctx.set_theme(Theme::Dark);
        let mut app = Self::with_map(MindMap::new(DEFAULT_ROOT_TEXT));
        if let Some(path) = initial_path {
            app.open_path(path);
        }
        app
    }

    fn with_map(map: MindMap) -> Self {
        Self {
            map,
            history: History::new(HISTORY_LIMIT),
            camera: Camera::default(),
            selected: None,
            editing: None,
            drag: None,
            file_path: None,
            dirty: false,
            dark_mode: true,
            needs_centering: true,
            canvas_rect: Rect::NOTHING,
            layout: Vec::new(),
            window_title: String::new(),
            status: String::new(),
        }
    }

    /// Apply `change` to the map, recording an undo point only if it succeeds.
    fn edit_map<R>(
        &mut self,
        change: impl FnOnce(&mut MindMap) -> Result<R, MapError>,
    ) -> Option<R> {
        let snapshot = self.map.clone();
        match change(&mut self.map) {
            Ok(value) => {
                self.history.record(snapshot);
                self.dirty = true;
                Some(value)
            }
            Err(err) => {
                self.status = err.to_string();
                None
            }
        }
    }

    fn perform(&mut self, action: Action) {
        // Any toolbar or keyboard action first finishes an in-progress edit.
        self.commit_edit();
        let root = self.map.root();
        match action {
            Action::New => {
                if self.confirm_discard() {
                    self.replace_map(MindMap::new(DEFAULT_ROOT_TEXT), None);
                }
            }
            Action::Open => {
                if self.confirm_discard()
                    && let Some(path) = file_dialog().pick_file()
                {
                    self.open_path(&path);
                }
            }
            Action::Save => match self.file_path.take() {
                Some(path) => self.save_to(path),
                None => self.perform(Action::SaveAs),
            },
            Action::SaveAs => {
                if let Some(path) = file_dialog().set_file_name("mind_map.json").save_file() {
                    self.save_to(path);
                }
            }
            Action::Undo => {
                if self.history.undo(&mut self.map) {
                    self.after_history_change();
                }
            }
            Action::Redo => {
                if self.history.redo(&mut self.map) {
                    self.after_history_change();
                }
            }
            Action::AddChild => {
                let parent = self.selected.unwrap_or(root);
                if let Some(id) = self.edit_map(|map| map.add_child(parent, NEW_NODE_TEXT)) {
                    self.start_edit(id, String::new());
                }
            }
            Action::AddSibling => {
                let created = match self.selected.filter(|&id| id != root) {
                    Some(sibling) => self.edit_map(|map| map.add_sibling(sibling, NEW_NODE_TEXT)),
                    None => self.edit_map(|map| map.add_child(root, NEW_NODE_TEXT)),
                };
                if let Some(id) = created {
                    self.start_edit(id, String::new());
                }
            }
            Action::Rename => {
                if let Some(id) = self.selected
                    && let Some(node) = self.map.node(id)
                {
                    let text = node.text().to_owned();
                    self.start_edit(id, text);
                }
            }
            Action::Delete => {
                if let Some(id) = self.selected.filter(|&id| id != root) {
                    let parent = self.map.node(id).and_then(Node::parent);
                    if self.edit_map(|map| map.remove_subtree(id)).is_some() {
                        self.selected = parent;
                    }
                }
            }
            Action::CenterView => self.needs_centering = true,
            Action::ToggleTheme => self.dark_mode = !self.dark_mode,
        }
    }

    fn after_history_change(&mut self) {
        let map = &self.map;
        self.selected = self.selected.filter(|&id| map.node(id).is_some());
        self.editing = None;
        self.dirty = true;
    }

    fn replace_map(&mut self, map: MindMap, path: Option<PathBuf>) {
        self.map = map;
        self.history = History::new(HISTORY_LIMIT);
        self.selected = None;
        self.editing = None;
        self.drag = None;
        self.file_path = path;
        self.dirty = false;
        self.needs_centering = true;
    }

    fn open_path(&mut self, path: &Path) {
        match storage::load_from_path(path) {
            Ok(map) => {
                self.status = format!("Opened {}", path.display());
                self.replace_map(map, Some(path.to_owned()));
            }
            Err(err) => self.status = format!("Could not open {}: {err}", path.display()),
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        match storage::save_to_path(&self.map, &path) {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.dirty = false;
            }
            Err(err) => self.status = format!("Could not save {}: {err}", path.display()),
        }
        self.file_path = Some(path);
    }

    /// Ask before throwing away unsaved changes.
    fn confirm_discard(&self) -> bool {
        !self.dirty
            || MessageDialog::new()
                .set_title("Unsaved changes")
                .set_description("Discard the unsaved changes to this mind map?")
                .set_buttons(MessageButtons::YesNo)
                .show()
                == MessageDialogResult::Yes
    }

    fn start_edit(&mut self, id: NodeId, buffer: String) {
        self.selected = Some(id);
        self.editing = Some(EditSession {
            id,
            buffer,
            request_focus: true,
        });
    }

    /// Finish the current edit, keeping the old label if the new one is blank or unchanged.
    fn commit_edit(&mut self) {
        let Some(session) = self.editing.take() else {
            return;
        };
        let text = session.buffer.trim();
        let unchanged = self
            .map
            .node(session.id)
            .is_none_or(|node| node.text() == text);
        if !text.is_empty() && !unchanged {
            self.edit_map(|map| map.set_text(session.id, text));
        }
    }

    fn hit_test(&self, screen: Pos2) -> Option<NodeId> {
        // Reverse order: nodes painted last are on top.
        self.layout
            .iter()
            .rev()
            .find(|node| node.rect.contains(screen))
            .map(|node| node.id)
    }

    fn title(&self) -> String {
        let name = self
            .file_path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| "Untitled".into(), |name| name.to_string_lossy());
        let marker = if self.dirty { "*" } else { "" };
        format!("{marker}{name} - Mind Mapper")
    }

    fn toolbar(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut clicked = None;
        let root = self.map.root();
        let has_selection = self.selected.is_some();
        let can_delete = self.selected.is_some_and(|id| id != root);
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
            ui.separator();
            let theme_label = if self.dark_mode {
                "Light mode"
            } else {
                "Dark mode"
            };
            if ui.button(theme_label).clicked() {
                clicked = Some(Action::ToggleTheme);
            }
            ui.separator();
            ui.label(format!("{:.0}%", self.camera.zoom() * 100.0));
            ui.label(&self.status);
        });
        clicked
    }

    fn handle_zoom(&mut self, ui: &Ui, response: &Response) {
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
            self.camera.zoom_at(anchor, factor);
        }
    }

    /// Selection, node dragging and panning. Uses the previous frame's layout for hit
    /// testing, so the node under the cursor is the one the user saw.
    fn handle_pointer(&mut self, ui: &Ui, response: &Response) {
        if response.drag_started() {
            let target = ui
                .input(|input| input.pointer.press_origin())
                .filter(|_| response.dragged_by(PointerButton::Primary))
                .and_then(|origin| self.hit_test(origin));
            self.drag = Some(match target {
                Some(id) => {
                    self.history.record(self.map.clone());
                    self.selected = Some(id);
                    Drag::Node(id)
                }
                None => Drag::Pan,
            });
        }
        if response.dragged() {
            let delta = response.drag_delta();
            match self.drag {
                Some(Drag::Node(id)) => {
                    let world_delta = delta / self.camera.zoom();
                    if self
                        .map
                        .translate_subtree(id, world_delta.x, world_delta.y)
                        .is_ok()
                    {
                        self.dirty = true;
                    }
                }
                Some(Drag::Pan) => self.camera.pan(delta),
                None => {}
            }
        }
        if response.drag_stopped() {
            self.drag = None;
        }

        let hit = response
            .interact_pointer_pos()
            .and_then(|pos| self.hit_test(pos));
        if response.double_clicked() {
            self.perform_on(hit, Action::Rename);
        } else if response.clicked() {
            self.commit_edit();
            self.selected = hit;
        }
    }

    /// Select `target` (if any) and then run `action` on it.
    fn perform_on(&mut self, target: Option<NodeId>, action: Action) {
        if target.is_some() {
            self.selected = target;
            self.perform(action);
        }
    }

    /// Lay out the visible nodes and paint edges and nodes.
    fn paint(&mut self, painter: &egui::Painter, visuals: &Visuals) {
        let zoom = self.camera.zoom();
        let font = FontId::proportional(BASE_FONT_SIZE * zoom);
        let padding = NODE_PADDING * zoom;
        let text_color = visuals.strong_text_color();
        let edge_width = (2.0 * zoom).max(1.0);
        let clip = painter.clip_rect();
        // Nodes are positioned by their centre, so widen the clip test by the largest
        // likely half-size to keep partly visible nodes.
        let cull_rect = clip.expand(MAX_LABEL_WIDTH * zoom);

        self.layout.clear();
        for visit in self.map.preorder() {
            let accent = visit.branch.map_or(text_color, |branch| {
                BRANCH_COLORS[branch % BRANCH_COLORS.len()]
            });
            let center = self.camera.world_to_screen(visit.node.position());

            if let Some(parent) = visit.node.parent().and_then(|id| self.map.node(id)) {
                let start = self.camera.world_to_screen(parent.position());
                if clip.intersects(Rect::from_two_pos(start, center)) {
                    let bend = Vec2::new((center.x - start.x) * 0.5, 0.0);
                    painter.add(CubicBezierShape::from_points_stroke(
                        [start, start + bend, center - bend, center],
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(edge_width, accent),
                    ));
                }
            }

            if cull_rect.contains(center) {
                let galley = painter.layout(
                    visit.node.text().to_owned(),
                    font.clone(),
                    text_color,
                    MAX_LABEL_WIDTH * zoom,
                );
                let rect = Rect::from_center_size(center, galley.size() + padding * 2.0);
                self.layout.push(LaidOutNode {
                    id: visit.id,
                    rect,
                    galley,
                    accent,
                });
            }
        }

        let radius = CORNER_RADIUS * zoom;
        let fill = visuals.window_fill();
        let selection = visuals.selection.stroke.color;
        for node in &self.layout {
            let is_selected = self.selected == Some(node.id);
            let (border, width) = if is_selected {
                (selection, edge_width * 2.0)
            } else {
                (node.accent, edge_width)
            };
            painter.rect_filled(node.rect, radius, fill);
            painter.rect_stroke(
                node.rect,
                radius,
                Stroke::new(width, border),
                StrokeKind::Inside,
            );
            painter.galley(
                node.rect.center() - node.galley.size() * 0.5,
                Arc::clone(&node.galley),
                text_color,
            );
        }
    }

    /// Show the in-place text box for the node being edited.
    fn show_editor(&mut self, ui: &mut Ui) {
        let Some(session) = self.editing.as_mut() else {
            return;
        };
        let Some(node_rect) = self
            .layout
            .iter()
            .find(|node| node.id == session.id)
            .map(|node| node.rect)
        else {
            // The node scrolled out of view (or was deleted): finish the edit.
            self.commit_edit();
            return;
        };
        let zoom = self.camera.zoom();
        let size = Vec2::new(
            node_rect.width().max(MIN_EDITOR_WIDTH * zoom),
            node_rect.height(),
        );
        let editor = egui::TextEdit::singleline(&mut session.buffer)
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

    fn canvas(&mut self, ui: &mut Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        self.canvas_rect = response.rect;
        if self.needs_centering {
            let root = self.map.node(self.map.root()).map(Node::position);
            if let Some(root) = root {
                self.camera.center_on(root, self.canvas_rect.center());
            }
            self.needs_centering = false;
        }
        painter.rect_filled(self.canvas_rect, 0.0, ui.visuals().extreme_bg_color);
        self.handle_zoom(ui, &response);
        self.handle_pointer(ui, &response);
        self.paint(&painter, ui.visuals());
        self.show_editor(ui);
    }

    fn handle_window(&mut self, ui: &Ui) {
        let ctx = ui.ctx();
        let theme = if self.dark_mode {
            Theme::Dark
        } else {
            Theme::Light
        };
        if ctx.theme() != theme {
            ctx.set_theme(theme);
        }
        if ctx.input(|input| input.viewport().close_requested()) && !self.confirm_discard() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }
        let title = self.title();
        if title != self.window_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn handle_shortcuts(&mut self, ui: &Ui) {
        if self.editing.is_some() {
            return;
        }
        let ctx = ui.ctx();
        // Keep keyboard focus off the toolbar so Tab and Enter reach the canvas
        // instead of moving focus between (or pressing) buttons.
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        let pressed = ctx.input_mut(|input| {
            SHORTCUTS
                .iter()
                .find(|(shortcut, _)| input.consume_shortcut(shortcut))
                .map(|&(_, action)| action)
        });
        if let Some(action) = pressed {
            self.perform(action);
        }
    }
}

/// A file dialog pre-configured for mind-map files.
fn file_dialog() -> FileDialog {
    FileDialog::new().add_filter(FILE_FILTER.0, FILE_FILTER.1)
}

impl eframe::App for MindMapApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.handle_window(ui);
        self.handle_shortcuts(ui);
        let clicked = egui::Panel::top("toolbar")
            .show(ui, |ui| self.toolbar(ui))
            .inner;
        if let Some(action) = clicked {
            self.perform(action);
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_child() -> (MindMapApp, NodeId) {
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        app.perform(Action::AddChild);
        let child = app.selected.expect("new child is selected");
        app.commit_edit();
        (app, child)
    }

    #[test]
    fn add_child_selects_and_edits_new_node() {
        let mut app = MindMapApp::with_map(MindMap::new("root"));

        app.perform(Action::AddChild);

        let id = app.selected.expect("new node is selected");
        assert_eq!(app.editing.as_ref().map(|session| session.id), Some(id));
        assert_eq!(app.map.len(), 2);
        assert!(app.dirty);
        assert!(app.history.can_undo());
    }

    #[test]
    fn blank_edit_keeps_default_label() {
        let (app, child) = app_with_child();

        assert_eq!(app.map.node(child).expect("child").text(), NEW_NODE_TEXT);
    }

    #[test]
    fn commit_edit_trims_and_applies_text() {
        let (mut app, child) = app_with_child();
        app.start_edit(child, "  Idea  ".to_owned());

        app.commit_edit();

        assert_eq!(app.map.node(child).expect("child").text(), "Idea");
        assert!(app.editing.is_none());
    }

    #[test]
    fn add_sibling_on_root_adds_child() {
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        app.selected = Some(app.map.root());

        app.perform(Action::AddSibling);

        let id = app.selected.expect("new node is selected");
        assert_eq!(
            app.map.node(id).expect("node").parent(),
            Some(app.map.root())
        );
    }

    #[test]
    fn delete_selects_parent_and_undo_restores() {
        let (mut app, child) = app_with_child();

        app.perform(Action::Delete);

        assert!(app.map.node(child).is_none());
        assert_eq!(app.selected, Some(app.map.root()));
        app.perform(Action::Undo);
        assert!(app.map.node(child).is_some());
        app.perform(Action::Redo);
        assert!(app.map.node(child).is_none());
    }

    #[test]
    fn root_cannot_be_deleted() {
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        app.selected = Some(app.map.root());

        app.perform(Action::Delete);

        assert_eq!(app.map.len(), 1);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn undo_drops_selection_of_removed_node() {
        let (mut app, child) = app_with_child();
        app.selected = Some(child);

        app.perform(Action::Undo);
        app.perform(Action::Undo);

        assert_eq!(app.selected, None);
        assert_eq!(app.map.len(), 1);
    }

    /// Lay out an empty label with egui's real font system (headless, no window).
    fn empty_galley() -> Arc<Galley> {
        let ctx = egui::Context::default();
        let mut galley = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            galley = Some(ui.painter().layout_no_wrap(
                String::new(),
                FontId::default(),
                Color32::WHITE,
            ));
        });
        // Nothing renders the frame, so discard its texture uploads explicitly.
        output.textures_delta.clear();
        galley.expect("run_ui calls the closure once")
    }

    #[test]
    fn hit_test_prefers_topmost_node() {
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        let root = app.map.root();
        let child = app.map.add_child(root, "x").expect("root exists");
        let galley = empty_galley();
        let make = |id, min: Pos2| LaidOutNode {
            id,
            rect: Rect::from_min_size(min, Vec2::splat(10.0)),
            galley: Arc::clone(&galley),
            accent: Color32::WHITE,
        };
        app.layout = vec![make(root, Pos2::ZERO), make(child, Pos2::new(5.0, 5.0))];

        assert_eq!(app.hit_test(Pos2::new(7.0, 7.0)), Some(child));
        assert_eq!(app.hit_test(Pos2::new(1.0, 1.0)), Some(root));
        assert_eq!(app.hit_test(Pos2::new(50.0, 50.0)), None);
    }

    #[test]
    fn title_reflects_file_and_dirty_state() {
        let (mut app, _) = app_with_child();
        assert_eq!(app.title(), "*Untitled - Mind Mapper");

        app.file_path = Some(PathBuf::from("ideas.json"));
        app.dirty = false;

        assert_eq!(app.title(), "ideas.json - Mind Mapper");
    }

    #[test]
    fn toggle_theme_flips_mode() {
        let mut app = MindMapApp::with_map(MindMap::new("root"));

        app.perform(Action::ToggleTheme);

        assert!(!app.dark_mode);
    }
}
