//! The egui application: start screen, toolbar, canvas and user interaction.

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{
    self, Align2, FontId, Key, KeyboardShortcut, Modifiers, PointerButton, Rect, Response, Sense,
    Ui, Vec2, ViewportCommand,
};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};

use crate::history::History;
use crate::layout;
use crate::map_view::{BASE_FONT_SIZE, MapView};
use crate::model::{MapError, MindMap, Node, NodeId};
use crate::recent::RecentFiles;
use crate::recovery::{Recovered, RecoveryStore};
use crate::start_screen::{StartChoice, StartContext, StartScreen};
use crate::storage;
use crate::theme::{ColorTheme, theme_picker};

/// Application name: window title, and the folder name for settings and recovery data.
pub const APP_NAME: &str = "Mind Mapper";
/// Minimum time between background copies of unsaved work.
const AUTOSAVE_SECONDS: f64 = 20.0;
/// Narrowest text box shown while editing, at 100% zoom.
const MIN_EDITOR_WIDTH: f32 = 140.0;
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
    Tidy,
    ToggleCollapse,
}

/// Keyboard shortcuts. Variants with Shift come first because a shortcut without Shift
/// also matches when Shift is held.
const SHORTCUTS: [(KeyboardShortcut, Action); 14] = [
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
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::L),
        Action::Tidy,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::Space),
        Action::ToggleCollapse,
    ),
];

/// Which page the window is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    /// The template gallery. `can_return` is true when a map was already open.
    Start { can_return: bool },
    /// The mind-map editor.
    Editor,
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
    Node {
        /// The node being dragged.
        id: NodeId,
        /// Node under the cursor that the branch would be re-attached to if dropped now.
        drop_target: Option<NodeId>,
    },
    /// Moving the view.
    Pan,
}

/// The mind-mapping application state.
pub struct MindMapApp {
    screen: Screen,
    start: StartScreen,
    map: MindMap,
    history: History<MindMap>,
    view: MapView,
    selected: Option<NodeId>,
    editing: Option<EditSession>,
    drag: Option<Drag>,
    file_path: Option<PathBuf>,
    dirty: bool,
    theme: ColorTheme,
    /// The theme last installed in egui; `None` until the first frame.
    applied_theme: Option<ColorTheme>,
    /// The whole map is fitted into view once the canvas size is known.
    needs_fit: bool,
    window_title: String,
    status: String,
    /// Where unsaved work is copied for crash recovery; `None` if there is no data folder.
    recovery: Option<RecoveryStore>,
    /// Work recovered from a crashed session, waiting for Restore or Discard.
    recovered: Option<Recovered>,
    /// A command-line file held back until the recovered work has been dealt with.
    deferred_open: Option<PathBuf>,
    /// The map changed since the last recovery copy was written.
    autosave_pending: bool,
    /// When the last recovery copy was written, in seconds since the app started.
    last_autosave: f64,
    /// Recently opened or saved files, shown on the start screen.
    recent: RecentFiles,
}

impl MindMapApp {
    /// Create the app on the start screen, or straight in the editor if `initial_path`
    /// opens successfully.
    ///
    /// If unsaved work survived a crash, the start screen offers it first and
    /// `initial_path` is opened only after that work is discarded.
    ///
    /// # Arguments
    ///
    /// * `cc` - eframe creation context; its storage provides the saved theme and
    ///   recent files
    /// * `initial_path` - File passed on the command line, if any. If it fails to load,
    ///   the app stays on the start screen.
    #[must_use]
    pub fn new(cc: &eframe::CreationContext<'_>, initial_path: Option<&Path>) -> Self {
        let mut app = Self::with_map(MindMap::new(DEFAULT_ROOT_TEXT));
        app.theme = ColorTheme::load(cc.storage);
        app.recent = RecentFiles::load(cc.storage);
        app.recovery = eframe::storage_dir(APP_NAME).map(|dir| RecoveryStore::in_dir(&dir));
        if let Some(store) = &app.recovery {
            match store.load() {
                Ok(found) => app.recovered = found,
                Err(err) => app.status = format!("Could not read recovered work: {err}"),
            }
        }
        if let Some(path) = initial_path {
            if app.recovered.is_some() {
                app.deferred_open = Some(path.to_owned());
            } else {
                app.open_path(path);
            }
        }
        app
    }

    fn with_map(map: MindMap) -> Self {
        Self {
            screen: Screen::Start { can_return: false },
            start: StartScreen::default(),
            map,
            history: History::new(HISTORY_LIMIT),
            view: MapView::default(),
            selected: None,
            editing: None,
            drag: None,
            file_path: None,
            dirty: false,
            theme: ColorTheme::default(),
            applied_theme: None,
            needs_fit: true,
            window_title: String::new(),
            status: String::new(),
            recovery: None,
            recovered: None,
            deferred_open: None,
            autosave_pending: false,
            last_autosave: 0.0,
            recent: RecentFiles::default(),
        }
    }

    /// Note that the map has unsaved changes (and needs a fresh recovery copy).
    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.autosave_pending = true;
    }

    /// Write a recovery copy if there are new changes and enough time has passed.
    fn autosave(&mut self, ctx: &egui::Context) {
        if !self.autosave_pending {
            return;
        }
        let Some(store) = &self.recovery else {
            return;
        };
        let now = ctx.input(|input| input.time);
        let due = self.last_autosave + AUTOSAVE_SECONDS;
        if now < due {
            // egui only redraws on input, so ask for a wake-up when the save is due.
            ctx.request_repaint_after(Duration::from_secs_f64(due - now));
            return;
        }
        if let Err(err) = store.save(&self.map, self.file_path.as_deref()) {
            self.status = format!("Auto-save failed: {err}");
        }
        self.last_autosave = now;
        self.autosave_pending = false;
    }

    /// Delete the recovery copy (after a save, or once the user discards their work).
    fn clear_recovery(&mut self) {
        if let Some(store) = &self.recovery
            && let Err(err) = store.clear()
        {
            self.status = format!("Could not remove the recovery file: {err}");
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
                self.mark_dirty();
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
            Action::New => self.screen = Screen::Start { can_return: true },
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
                let created = self.edit_map(|map| {
                    // A new child inside a folded branch would be invisible; unfold it.
                    if map.node(parent).is_some_and(Node::is_collapsed) {
                        map.set_collapsed(parent, false)?;
                    }
                    map.add_child(parent, NEW_NODE_TEXT)
                });
                if let Some(id) = created {
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
            Action::CenterView => self.needs_fit = true,
            Action::Tidy => {
                self.edit_map(layout::tidy);
            }
            Action::ToggleCollapse => {
                if let Some(id) = self.selected {
                    self.toggle_collapse(id);
                }
            }
        }
    }

    /// Fold or unfold `id`'s branch (nodes without children are left alone). If the
    /// selection disappears into a folded branch, the folded node becomes selected.
    fn toggle_collapse(&mut self, id: NodeId) {
        let Some(node) = self.map.node(id) else {
            return;
        };
        if node.children().is_empty() {
            return;
        }
        let collapse = !node.is_collapsed();
        if self
            .edit_map(|map| map.set_collapsed(id, collapse))
            .is_some()
            && collapse
            && self
                .selected
                .is_some_and(|selected| self.map.is_in_subtree(selected, id))
        {
            self.selected = Some(id);
        }
    }

    /// Whether the selected node can be folded or unfolded, and which label to show.
    fn fold_state(&self) -> (bool, &'static str) {
        let node = self
            .selected
            .filter(|&id| id != self.map.root())
            .and_then(|id| self.map.node(id));
        match node {
            Some(node) if node.is_collapsed() => (true, "Expand"),
            Some(node) => (!node.children().is_empty(), "Collapse"),
            None => (false, "Collapse"),
        }
    }

    fn after_history_change(&mut self) {
        let map = &self.map;
        self.selected = self.selected.filter(|&id| map.node(id).is_some());
        self.editing = None;
        self.mark_dirty();
    }

    fn replace_map(&mut self, map: MindMap, path: Option<PathBuf>) {
        self.map = map;
        self.history = History::new(HISTORY_LIMIT);
        self.selected = None;
        self.editing = None;
        self.drag = None;
        self.file_path = path;
        self.dirty = false;
        self.needs_fit = true;
        self.screen = Screen::Editor;
    }

    /// Act on a start-screen choice.
    fn choose(&mut self, choice: StartChoice) {
        match choice {
            StartChoice::Template(index) => {
                if let Some(map) = self.start.template_map(index)
                    && self.confirm_discard()
                {
                    self.replace_map(map, None);
                }
            }
            StartChoice::Open => self.perform(Action::Open),
            StartChoice::Back => self.screen = Screen::Editor,
            StartChoice::Restore => {
                if let Some(Recovered { map, source }) = self.recovered.take() {
                    self.deferred_open = None;
                    self.replace_map(map, source);
                    // Still unsaved; this also keeps the recovery copy up to date.
                    self.mark_dirty();
                    "Restored unsaved work from the last session".clone_into(&mut self.status);
                }
            }
            StartChoice::OpenRecent(index) => {
                if let Some(path) = self.recent.paths().get(index).cloned()
                    && self.confirm_discard()
                {
                    self.open_path(&path);
                }
            }
            StartChoice::DiscardRecovery => {
                self.recovered = None;
                self.clear_recovery();
                if let Some(path) = self.deferred_open.take() {
                    self.open_path(&path);
                }
            }
        }
    }

    fn open_path(&mut self, path: &Path) {
        match storage::load_from_path(path) {
            Ok(map) => {
                self.status = format!("Opened {}", file_label(path));
                self.recent.add(path);
                self.replace_map(map, Some(path.to_owned()));
            }
            Err(err) => {
                self.status = format!("Could not open {}: {err}", path.display());
                // Drop dead entries (moved or deleted files) from the recent list.
                self.recent.remove(path);
            }
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        match storage::save_to_path(&self.map, &path) {
            Ok(()) => {
                self.status = format!("Saved {}", file_label(&path));
                self.recent.add(&path);
                self.dirty = false;
                self.autosave_pending = false;
                self.clear_recovery();
            }
            Err(err) => self.status = format!("Could not save {}: {err}", path.display()),
        }
        self.file_path = Some(path);
    }

    /// Ask before throwing away unsaved (or recovered) work. If the user agrees, the
    /// recovery copy is deleted too.
    ///
    /// # Returns
    ///
    /// `true` if it is fine to continue.
    fn confirm_discard(&mut self) -> bool {
        let has_unsaved = self.dirty || self.recovered.is_some();
        if !has_unsaved {
            return true;
        }
        let discard = MessageDialog::new()
            .set_title("Unsaved changes")
            .set_description("Discard the unsaved changes to this mind map?")
            .set_buttons(MessageButtons::YesNo)
            .show()
            == MessageDialogResult::Yes;
        if discard {
            self.recovered = None;
            self.clear_recovery();
            // If the current map stays open (e.g. the file dialog is cancelled), keep
            // protecting it.
            self.autosave_pending = self.dirty;
        }
        discard
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

    fn title(&self) -> String {
        if matches!(self.screen, Screen::Start { .. }) {
            return APP_NAME.to_owned();
        }
        let name = self
            .file_path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| "Untitled".into(), |name| name.to_string_lossy());
        let marker = if self.dirty { "*" } else { "" };
        format!("{marker}{name} - {APP_NAME}")
    }

    fn toolbar(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut clicked = None;
        let root = self.map.root();
        let has_selection = self.selected.is_some();
        let can_delete = self.selected.is_some_and(|id| id != root);
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
            ui.separator();
            ui.label("Theme");
            theme_picker(ui, &mut self.theme);
            ui.separator();
            ui.label(format!("{:.0}%", self.view.camera().zoom() * 100.0));
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
            self.view.camera_mut().zoom_at(anchor, factor);
        }
    }

    /// Selection, node dragging and panning. Uses the previous frame's layout for hit
    /// testing, so the node under the cursor is the one the user saw.
    fn handle_pointer(&mut self, ui: &Ui, response: &Response) {
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
    fn find_drop_target(&self, dragged: NodeId, screen: egui::Pos2) -> Option<NodeId> {
        let current_parent = self.map.node(dragged).and_then(Node::parent);
        self.view.hit_test_where(screen, |candidate| {
            Some(candidate) != current_parent && !self.map.is_in_subtree(candidate, dragged)
        })
    }

    /// Re-attach `id` under `new_parent`. The undo point was recorded when the drag began.
    fn reparent(&mut self, id: NodeId, new_parent: NodeId) {
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
    fn perform_on(&mut self, target: Option<NodeId>, action: Action) {
        if target.is_some() {
            self.selected = target;
            self.perform(action);
        }
    }

    /// Show the in-place text box for the node being edited.
    fn show_editor(&mut self, ui: &mut Ui) {
        let Some(session) = self.editing.as_mut() else {
            return;
        };
        let Some(node_rect) = self.view.node_rect(session.id) else {
            // The node scrolled out of view (or was deleted): finish the edit.
            self.commit_edit();
            return;
        };
        let zoom = self.view.camera().zoom();
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
        if self.needs_fit {
            self.view.fit(&self.map, response.rect);
            self.needs_fit = false;
        }
        painter.rect_filled(response.rect, 0.0, ui.visuals().extreme_bg_color);
        self.handle_zoom(ui, &response);
        self.handle_pointer(ui, &response);
        self.view
            .paint(&painter, ui.visuals(), &self.map, self.selected);
        if let Some(Drag::Node {
            drop_target: Some(target),
            ..
        }) = self.drag
        {
            self.view
                .outline(&painter, target, ui.visuals().selection.stroke.color);
        }
        self.show_editor(ui);
    }

    /// Install the chosen theme in egui if it changed since the last frame.
    fn sync_theme(&mut self, ctx: &egui::Context) {
        if self.applied_theme != Some(self.theme) {
            self.theme.apply(ctx);
            self.applied_theme = Some(self.theme);
        }
    }

    fn handle_window(&mut self, ui: &Ui) {
        let ctx = ui.ctx();
        self.sync_theme(ctx);
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
        // Only "Open" makes sense while the start screen is showing.
        let on_start = matches!(self.screen, Screen::Start { .. });
        let pressed = ctx.input_mut(|input| {
            SHORTCUTS
                .iter()
                .filter(|(_, action)| !on_start || *action == Action::Open)
                .find(|(shortcut, _)| input.consume_shortcut(shortcut))
                .map(|&(_, action)| action)
        });
        if let Some(action) = pressed {
            self.perform(action);
        }
    }
}

/// Short name for status messages: the file name, so long paths don't widen the toolbar.
fn file_label(path: &Path) -> std::borrow::Cow<'_, str> {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
}

/// A file dialog pre-configured for mind-map files.
fn file_dialog() -> FileDialog {
    FileDialog::new().add_filter(FILE_FILTER.0, FILE_FILTER.1)
}

impl eframe::App for MindMapApp {
    /// Called by eframe on exit and periodically, to remember settings between runs.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.theme.save(storage);
        self.recent.save(storage);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.handle_window(ui);
        self.autosave(ui.ctx());
        self.handle_shortcuts(ui);
        if let Screen::Start { can_return } = self.screen {
            let recovery_label = self.recovered.as_ref().map(Recovered::description);
            let context = StartContext {
                can_return,
                recovery: recovery_label.as_deref(),
                recent: self.recent.paths(),
            };
            let choice = egui::CentralPanel::default()
                .show(ui, |ui| self.start.show(ui, &context, &mut self.theme))
                .inner;
            if let Some(choice) = choice {
                self.choose(choice);
            }
            return;
        }
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
    use crate::test_support::TempDir;

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
    fn reparent_moves_branch_and_undo_restores_it() {
        let (mut app, child) = app_with_child();
        let root = app.map.root();
        let other = app
            .edit_map(|map| map.add_child(root, "other"))
            .expect("root exists");
        // Dragging records the undo point before anything moves.
        app.history.record(app.map.clone());

        app.reparent(child, other);

        assert_eq!(app.map.node(child).expect("child").parent(), Some(other));
        app.perform(Action::Undo);
        assert_eq!(app.map.node(child).expect("child").parent(), Some(root));
    }

    #[test]
    fn invalid_reparent_reports_error() {
        let (mut app, child) = app_with_child();
        let root = app.map.root();

        app.reparent(root, child);

        assert_eq!(app.map.node(root).expect("root").parent(), None);
        assert_eq!(app.status, MapError::RootNode.to_string());
    }

    #[test]
    fn tidy_is_one_undo_step() {
        let (mut app, child) = app_with_child();
        app.map
            .set_position(child, crate::model::Point::new(-999.0, 0.0))
            .expect("child exists");
        let before = app.map.clone();

        app.perform(Action::Tidy);
        assert_ne!(app.map, before);
        app.perform(Action::Undo);

        assert_eq!(app.map, before);
    }

    #[test]
    fn collapsing_moves_hidden_selection_to_folded_node() {
        let (mut app, child) = app_with_child();
        let grandchild = app
            .edit_map(|map| map.add_child(child, "deep"))
            .expect("child exists");
        app.selected = Some(grandchild);

        app.toggle_collapse(child);

        assert!(app.map.node(child).expect("child").is_collapsed());
        assert_eq!(app.selected, Some(child));
        assert_eq!(app.fold_state(), (true, "Expand"));
        app.perform(Action::ToggleCollapse);
        assert!(!app.map.node(child).expect("child").is_collapsed());
    }

    #[test]
    fn leaves_and_root_cannot_be_folded() {
        let (mut app, child) = app_with_child();
        app.selected = Some(child);
        assert_eq!(app.fold_state(), (false, "Collapse"));
        app.selected = Some(app.map.root());
        assert_eq!(app.fold_state(), (false, "Collapse"));
        let before = app.map.clone();

        app.toggle_collapse(child);
        app.toggle_collapse(app.map.root());

        assert_eq!(app.map, before);
    }

    #[test]
    fn adding_child_to_folded_node_unfolds_it() {
        let (mut app, child) = app_with_child();
        app.edit_map(|map| map.add_child(child, "deep"))
            .expect("child exists");
        app.toggle_collapse(child);
        app.selected = Some(child);

        app.perform(Action::AddChild);

        assert!(!app.map.node(child).expect("child").is_collapsed());
        assert_eq!(app.map.node(child).expect("child").children().len(), 2);
    }

    #[test]
    fn dropping_onto_folded_node_unfolds_it() {
        let (mut app, child) = app_with_child();
        let root = app.map.root();
        let target = app
            .edit_map(|map| map.add_child(root, "target"))
            .expect("root exists");
        app.edit_map(|map| map.add_child(target, "inside"))
            .expect("target exists");
        app.toggle_collapse(target);

        app.reparent(child, target);

        assert!(!app.map.node(target).expect("target").is_collapsed());
        assert_eq!(app.map.node(child).expect("child").parent(), Some(target));
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

    #[test]
    fn app_starts_on_start_screen() {
        let app = MindMapApp::with_map(MindMap::new("root"));

        assert_eq!(app.screen, Screen::Start { can_return: false });
        assert_eq!(app.title(), "Mind Mapper");
    }

    #[test]
    fn choosing_a_template_opens_it_in_the_editor() {
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        let expected = app.start.template_map(2).expect("template exists");

        app.choose(StartChoice::Template(2));

        assert_eq!(app.screen, Screen::Editor);
        assert_eq!(app.map, expected);
        assert!(!app.dirty);
        assert!(app.file_path.is_none());
        assert!(app.needs_fit);
    }

    #[test]
    fn new_shows_start_screen_and_back_returns_unchanged() {
        let (mut app, child) = app_with_child();
        app.screen = Screen::Editor;

        app.perform(Action::New);
        assert_eq!(app.screen, Screen::Start { can_return: true });
        app.choose(StartChoice::Back);

        assert_eq!(app.screen, Screen::Editor);
        assert!(app.map.node(child).is_some());
    }

    #[test]
    fn restore_opens_recovered_map_as_unsaved() {
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        let mut map = MindMap::new("recovered");
        map.add_child(map.root(), "idea").expect("root exists");
        app.recovered = Some(Recovered {
            map: map.clone(),
            source: Some(PathBuf::from("ideas.json")),
        });
        app.deferred_open = Some(PathBuf::from("other.json"));

        app.choose(StartChoice::Restore);

        assert_eq!(app.screen, Screen::Editor);
        assert_eq!(app.map, map);
        assert_eq!(app.file_path, Some(PathBuf::from("ideas.json")));
        assert!(app.dirty);
        assert!(app.autosave_pending);
        assert!(app.recovered.is_none());
        assert!(app.deferred_open.is_none());
    }

    #[test]
    fn discard_recovery_deletes_copy_and_opens_deferred_file() {
        let dir = TempDir::new("app_discard");
        let store = RecoveryStore::in_dir(dir.path());
        let recovered = MindMap::new("recovered");
        store.save(&recovered, None).expect("save succeeds");
        let deferred = dir.path().join("deferred.json");
        storage::save_to_path(&MindMap::new("deferred"), &deferred).expect("save succeeds");
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        app.recovery = Some(store.clone());
        app.recovered = Some(Recovered {
            map: recovered,
            source: None,
        });
        app.deferred_open = Some(deferred.clone());

        app.choose(StartChoice::DiscardRecovery);

        assert_eq!(store.load().expect("load succeeds"), None);
        assert!(app.recovered.is_none());
        assert_eq!(app.file_path, Some(deferred));
        assert_eq!(app.screen, Screen::Editor);
    }

    #[test]
    fn autosave_waits_for_interval_then_writes_recovery_copy() {
        let dir = TempDir::new("app_autosave");
        let store = RecoveryStore::in_dir(dir.path());
        let ctx = egui::Context::default();
        let (mut app, _) = app_with_child();
        app.recovery = Some(store.clone());
        // The app has just started (time 0), so the first interval has not elapsed.
        app.autosave(&ctx);
        assert_eq!(store.load().expect("load succeeds"), None);

        app.last_autosave = -AUTOSAVE_SECONDS;
        app.autosave(&ctx);

        let saved = store.load().expect("load succeeds").expect("copy written");
        assert_eq!(saved.map, app.map);
        assert!(!app.autosave_pending);
    }

    #[test]
    fn saving_removes_recovery_copy() {
        let dir = TempDir::new("app_save_clears");
        let store = RecoveryStore::in_dir(dir.path());
        let (mut app, _) = app_with_child();
        app.recovery = Some(store.clone());
        store.save(&app.map, None).expect("save succeeds");

        app.save_to(dir.path().join("map.json"));

        assert!(!app.dirty);
        assert_eq!(store.load().expect("load succeeds"), None);
    }

    #[test]
    fn saving_and_opening_update_recent_files() {
        let dir = TempDir::new("app_recent");
        let path = dir.path().join("ideas.json");
        let (mut app, _) = app_with_child();

        app.save_to(path.clone());
        assert_eq!(app.recent.paths(), std::slice::from_ref(&path));
        app.choose(StartChoice::OpenRecent(0));

        assert_eq!(app.file_path, Some(path));
        assert_eq!(app.screen, Screen::Editor);
    }

    #[test]
    fn file_label_is_just_the_file_name() {
        let path = Path::new("some").join("folder").join("ideas.json");

        assert_eq!(file_label(&path), "ideas.json");
    }

    #[test]
    fn missing_recent_file_is_removed() {
        let dir = TempDir::new("app_recent_missing");
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        app.recent.add(&dir.path().join("gone.json"));

        app.choose(StartChoice::OpenRecent(0));

        assert_eq!(app.recent.paths(), &[] as &[PathBuf]);
        assert_eq!(app.screen, Screen::Start { can_return: false });
    }

    #[test]
    fn out_of_range_template_is_ignored() {
        let mut app = MindMapApp::with_map(MindMap::new("root"));

        app.choose(StartChoice::Template(usize::MAX));

        assert_eq!(app.screen, Screen::Start { can_return: false });
    }

    #[test]
    fn title_reflects_file_and_dirty_state() {
        let (mut app, _) = app_with_child();
        app.screen = Screen::Editor;
        assert_eq!(app.title(), "*Untitled - Mind Mapper");

        app.file_path = Some(PathBuf::from("ideas.json"));
        app.dirty = false;

        assert_eq!(app.title(), "ideas.json - Mind Mapper");
    }

    #[test]
    fn sync_theme_installs_chosen_theme() {
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        let ctx = egui::Context::default();

        app.theme = ColorTheme::LightPurple;
        app.sync_theme(&ctx);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        app.theme = ColorTheme::DarkBlue;
        app.sync_theme(&ctx);

        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(app.applied_theme, Some(ColorTheme::DarkBlue));
    }
}
