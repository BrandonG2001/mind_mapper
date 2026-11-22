//! The egui application: start screen, toolbar, canvas and user interaction.

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{
    self, Align2, Event, FontId, Key, KeyboardShortcut, Modifiers, PointerButton, Rect, Response,
    Sense, Ui, Vec2, ViewportCommand,
};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};

use crate::background::{Job, JobState};
use crate::clipboard;
use crate::details::{self, DetailsEdit, DetailsPanel, color_palette};
use crate::export::{self, ExportError, ExportStyle};
use crate::history::{History, HistoryLimits};
use crate::layout;
use crate::map_view::{BASE_FONT_SIZE, LINE_HEIGHT, MapView};
use crate::model::{MapError, MindMap, Node, NodeId};
use crate::navigation::{self, Direction};
use crate::recent::RecentFiles;
use crate::recovery::{Recovered, RecoveryStore};
use crate::search::{Search, SearchCommand};
use crate::settings::{Settings, SettingsFile};
use crate::start_screen::{StartChoice, StartContext, StartScreen};
use crate::storage::{self, StorageError};
use crate::theme::{self, ColorTheme, theme_picker, ui_size_control};

/// Application name: window title, and the folder name for settings and recovery data.
pub const APP_NAME: &str = "Mind Mapper";
/// Minimum time between background copies of unsaved work.
const AUTOSAVE_SECONDS: f64 = 20.0;
/// Narrowest text box shown while editing, at 100% zoom.
const MIN_EDITOR_WIDTH: f32 = 140.0;
/// Space above and below the text inside the label editor, at 100% zoom.
const EDITOR_PADDING: f32 = 8.0;
/// Number of undo steps kept.
const HISTORY_LIMITS: HistoryLimits = HistoryLimits {
    max_steps: 200,
    // Each step is a full copy of the map; about 16 MB at 100,000 nodes.
    max_bytes: 256 * 1024 * 1024,
    min_steps: 10,
};
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
    Navigate(Direction),
    Export,
    Find,
    FindNext,
    FindPrevious,
    CloseFind,
    ToggleDetails,
}

/// A clipboard request, which egui reports as an event rather than a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ClipboardRequest {
    Copy,
    Cut,
    Paste(String),
}

/// Keyboard shortcuts. Variants with Shift come first because a shortcut without Shift
/// also matches when Shift is held.
const SHORTCUTS: [(KeyboardShortcut, Action); 24] = [
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
    (
        KeyboardShortcut::new(Modifiers::SHIFT, Key::F3),
        Action::FindPrevious,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::F3),
        Action::FindNext,
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::I),
        Action::ToggleDetails,
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::F),
        Action::Find,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::Escape),
        Action::CloseFind,
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::E),
        Action::Export,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::ArrowUp),
        Action::Navigate(Direction::Up),
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::ArrowDown),
        Action::Navigate(Direction::Down),
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::ArrowLeft),
        Action::Navigate(Direction::Left),
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::ArrowRight),
        Action::Navigate(Direction::Right),
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
    /// A node to scroll into view on the next frame (after keyboard navigation).
    reveal: Option<NodeId>,
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
    /// Where settings are saved; `None` if there is no data folder (nothing is saved).
    settings_file: Option<SettingsFile>,
    /// Interface size (egui's zoom factor), remembered between runs.
    ui_zoom: f32,
    /// The open find bar, if any.
    search: Option<Search>,
    /// The open notes/link/colour side panel, if any.
    details: Option<DetailsPanel>,
    /// The node the right-click menu was opened on.
    context_node: Option<NodeId>,
    /// egui context, to wake the window when background jobs finish.
    ctx: egui::Context,
    /// Exports running in the background, with the file name for status messages.
    exports: Vec<(String, Job<Result<(), ExportError>>)>,
    /// The recovery copy being written in the background, if any.
    recovery_job: Option<Job<Result<(), StorageError>>>,
    /// Increases with every edit, so cached results (search matches) know to refresh.
    revision: u64,
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
        app.ctx = cc.egui_ctx.clone();
        let data_dir = eframe::storage_dir(APP_NAME);
        app.settings_file = data_dir.as_deref().map(SettingsFile::in_dir);
        let settings = match &app.settings_file {
            Some(file) if file.exists() => file.load(),
            // First run of a version with the shared settings file: carry over what
            // older versions kept in eframe's own storage.
            _ => {
                let carried_over = Settings {
                    theme: ColorTheme::load(cc.storage),
                    ui_zoom: theme::load_ui_zoom(cc.storage).unwrap_or(1.0),
                    recent: RecentFiles::load(cc.storage),
                };
                app.update_settings(|settings| *settings = carried_over.clone());
                carried_over
            }
        };
        app.theme = settings.theme;
        app.ui_zoom = settings.ui_zoom;
        app.recent = settings.recent;
        // Set the option directly (not `set_zoom_factor`, which waits a frame) so the
        // first frame already uses the saved size.
        cc.egui_ctx
            .options_mut(|options| options.zoom_factor = app.ui_zoom);
        if let Some(dir) = data_dir {
            match RecoveryStore::for_this_window(&dir) {
                Ok(store) => {
                    app.recovery = Some(store);
                    app.find_orphan();
                }
                Err(err) => app.status = format!("Crash recovery is unavailable: {err}"),
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
            history: History::new(HISTORY_LIMITS),
            view: MapView::default(),
            selected: None,
            editing: None,
            drag: None,
            file_path: None,
            dirty: false,
            theme: ColorTheme::default(),
            applied_theme: None,
            needs_fit: true,
            reveal: None,
            window_title: String::new(),
            status: String::new(),
            recovery: None,
            recovered: None,
            deferred_open: None,
            autosave_pending: false,
            last_autosave: 0.0,
            recent: RecentFiles::default(),
            settings_file: None,
            ui_zoom: 1.0,
            search: None,
            details: None,
            context_node: None,
            ctx: egui::Context::default(),
            exports: Vec::new(),
            recovery_job: None,
            revision: 0,
        }
    }

    /// Apply `change` to the shared settings file (on top of other windows' changes) and
    /// adopt the merged recent-files list. Without a data folder, only memory changes.
    fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let Some(file) = &self.settings_file else {
            let mut settings = Settings {
                theme: self.theme,
                ui_zoom: self.ui_zoom,
                recent: std::mem::take(&mut self.recent),
            };
            change(&mut settings);
            self.recent = settings.recent;
            return;
        };
        match file.update(change) {
            Ok(latest) => self.recent = latest.recent,
            Err(err) => self.status = format!("Could not save settings: {err}"),
        }
    }

    /// Note that the map has unsaved changes (and needs a fresh recovery copy).
    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.autosave_pending = true;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Write a recovery copy if there are new changes and enough time has passed.
    fn autosave(&mut self, ctx: &egui::Context) {
        if !self.autosave_pending {
            return;
        }
        if self.recovery.is_none() {
            return;
        }
        let now = ctx.input(|input| input.time);
        let due = self.last_autosave + AUTOSAVE_SECONDS;
        if now < due {
            // egui only redraws on input, so ask for a wake-up when the save is due.
            ctx.request_repaint_after(Duration::from_secs_f64(due - now));
            return;
        }
        // One copy at a time: if the previous one is still being written, try again on
        // a later frame (changes stay pending).
        if self.recovery_job.is_some() {
            return;
        }
        let Some(store) = &self.recovery else {
            return;
        };
        // Copying the map is quick; turning it into JSON and writing it is the slow part
        // (about 40 ms at 100,000 nodes), so that happens on a background thread.
        let writer = store.writer();
        let map = self.map.clone();
        let source = self.file_path.clone();
        self.recovery_job = Some(Job::spawn(&self.ctx, "recovery-copy", move || {
            writer.save(&map, source.as_deref())
        }));
        self.autosave_pending = false;
        self.last_autosave = now;
    }

    /// Write this window's recovery copy now, on this thread.
    fn save_recovery_copy(&mut self) {
        // A background write of the same file must not land after this one.
        self.finish_recovery_job();
        if let Some(store) = &self.recovery
            && let Err(err) = store.save(&self.map, self.file_path.as_deref())
        {
            self.status = format!("Auto-save failed: {err}");
        }
        self.autosave_pending = false;
    }

    /// Wait for a background recovery copy still being written, and report any error.
    fn finish_recovery_job(&mut self) {
        if let Some(job) = self.recovery_job.take() {
            self.report_recovery_result(job.wait());
        }
    }

    fn report_recovery_result(&mut self, result: Option<Result<(), StorageError>>) {
        match result {
            Some(Ok(())) => {}
            Some(Err(err)) => self.status = format!("Auto-save failed: {err}"),
            None => "Auto-save failed unexpectedly".clone_into(&mut self.status),
        }
    }

    /// Collect the results of background jobs that have finished.
    fn poll_jobs(&mut self) {
        if let Some(job) = &self.recovery_job {
            match job.poll() {
                JobState::Running => {}
                JobState::Finished(result) => {
                    self.recovery_job = None;
                    self.report_recovery_result(Some(result));
                }
                JobState::Failed => {
                    self.recovery_job = None;
                    self.report_recovery_result(None);
                }
            }
        }
        let mut messages = Vec::new();
        self.exports.retain(|(name, job)| {
            let message = match job.poll() {
                JobState::Running => return true,
                JobState::Finished(Ok(())) => format!("Exported {name}"),
                JobState::Finished(Err(err)) => format!("Could not export {name}: {err}"),
                JobState::Failed => {
                    format!("Could not export {name}: the export stopped unexpectedly")
                }
            };
            messages.push(message);
            false
        });
        if let Some(last) = messages.pop() {
            self.status = last;
        }
    }

    /// Look for unsaved work left by a window that is no longer running.
    fn find_orphan(&mut self) {
        if let Some(store) = &self.recovery {
            match store.find_orphan() {
                Ok(found) => self.recovered = found,
                Err(err) => self.status = format!("Could not read recovered work: {err}"),
            }
        }
    }

    /// Delete the files of the recovered work on offer (it was restored or discarded).
    fn discard_orphan(&mut self) {
        if let Some(recovered) = self.recovered.take()
            && let Err(err) = recovered.discard()
        {
            self.status = format!("Could not remove recovered work: {err}");
        }
    }

    /// Delete the recovery copy (after a save, or once the user discards their work).
    fn clear_recovery(&mut self) {
        // A background write finishing after the delete would bring the file back.
        self.finish_recovery_job();
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
        // Any toolbar or keyboard action first finishes in-progress edits.
        self.commit_edit();
        self.flush_details();
        let root = self.map.root();
        match action {
            Action::New => {
                // Pick up files other windows opened since this one started.
                if let Some(file) = &self.settings_file {
                    self.recent = file.load().recent;
                }
                self.screen = Screen::Start { can_return: true };
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
            Action::Export => self.export_dialog(),
            Action::Navigate(direction) => self.navigate(direction),
            Action::Find => match &mut self.search {
                Some(search) => search.request_focus(),
                None => self.search = Some(Search::new()),
            },
            Action::FindNext => self.search_command(SearchCommand::Next),
            Action::FindPrevious => self.search_command(SearchCommand::Previous),
            Action::CloseFind => self.search = None,
            Action::ToggleDetails => {
                if self.details.take().is_none() {
                    self.details = Some(DetailsPanel::default());
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
        // Reload the side panel so it shows (and will not re-apply) the undone text.
        if let Some(panel) = &mut self.details {
            panel.follow(&self.map, panel.node());
        }
        self.mark_dirty();
    }

    fn replace_map(&mut self, map: MindMap, path: Option<PathBuf>) {
        self.map = map;
        self.history = History::new(HISTORY_LIMITS);
        self.selected = None;
        self.editing = None;
        self.drag = None;
        // Node ids of the old map mean nothing in the new one.
        if let Some(panel) = &mut self.details {
            *panel = DetailsPanel::default();
        }
        self.context_node = None;
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
                if let Some(path) = self.recent.paths().get(index).cloned()
                    && self.confirm_discard()
                {
                    self.open_path(&path);
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

    fn open_path(&mut self, path: &Path) {
        match storage::load_from_path(path) {
            Ok(map) => {
                self.status = format!("Opened {}", file_label(path));
                self.update_settings(|settings| settings.recent.add(path));
                self.replace_map(map, Some(path.to_owned()));
            }
            Err(err) => {
                self.status = format!("Could not open {}: {err}", path.display());
                // Drop dead entries (moved or deleted files) from the recent list.
                self.update_settings(|settings| settings.recent.remove(path));
            }
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        match storage::save_to_path(&self.map, &path) {
            Ok(()) => {
                self.status = format!("Saved {}", file_label(&path));
                self.update_settings(|settings| settings.recent.add(&path));
                self.dirty = false;
                self.autosave_pending = false;
                self.clear_recovery();
            }
            Err(err) => self.status = format!("Could not save {}: {err}", path.display()),
        }
        self.file_path = Some(path);
    }

    /// Ask where to export, then export there.
    fn export_dialog(&mut self) {
        let stem = self
            .file_path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(|| "mind_map".into(), |stem| stem.to_string_lossy());
        let dialog = FileDialog::new()
            .add_filter("PNG image", &["png"])
            .add_filter("SVG drawing", &["svg"])
            .add_filter("Markdown outline", &["md"])
            .set_file_name(format!("{stem}.png"));
        if let Some(path) = dialog.save_file() {
            self.export_to(&path);
        }
    }

    /// Move the selection with an arrow key and scroll the new node into view. With
    /// nothing selected, any arrow starts at the central idea.
    fn navigate(&mut self, direction: Direction) {
        let target = match self.selected {
            Some(from) => navigation::neighbor(&self.map, from, direction),
            None => Some(self.map.root()),
        };
        if target.is_some() {
            self.selected = target;
            self.reveal = target;
        }
    }

    /// Export the map to `path` (format from its extension) in the current theme's colours.
    ///
    /// Runs on a background thread (a PNG of a huge map takes seconds); the status line
    /// reports the result when it is done.
    fn export_to(&mut self, path: &Path) {
        let style = ExportStyle::from_visuals(&self.theme.visuals());
        let name = file_label(path).into_owned();
        self.status = format!("Exporting {name}...");
        let map = self.map.clone();
        let path = path.to_owned();
        let job = Job::spawn(&self.ctx, "export", move || {
            export::export(&map, style, &path)
        });
        self.exports.push((name, job));
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
            self.discard_orphan();
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
        // Windows clipboards and keyboards may produce "\r\n"; store plain "\n".
        let buffer = session.buffer.replace("\r\n", "\n");
        let text = buffer.trim();
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
            ui.label("Theme");
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

    fn canvas(&mut self, ui: &mut Ui) {
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
    fn node_menu(&mut self, ui: &mut Ui, id: NodeId) {
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

    /// Apply side-panel or menu edits to node `id`, as one undo step.
    fn apply_details(&mut self, id: NodeId, edits: &[DetailsEdit]) {
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
    fn flush_details(&mut self) {
        let pending = self
            .details
            .as_ref()
            .and_then(|panel| Some((panel.node()?, panel.pending(&self.map))));
        if let Some((id, edits)) = pending {
            self.apply_details(id, &edits);
        }
    }

    /// Show the details side panel for the selected node.
    fn details_panel(&mut self, ui: &mut Ui) {
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
                self.apply_details(id, &[DetailsEdit::Color(color)]);
            }
        }
        if let Some(address) = outcome.open_link {
            ui.ctx().open_url(egui::OpenUrl::new_tab(address));
        }
        self.details = Some(panel);
    }

    /// Act on a find-bar command: jump to the first, next or previous match, or close.
    fn search_command(&mut self, command: SearchCommand) {
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

    /// Select `id`, unfold any folded branches hiding it (one undo step), and scroll to it.
    fn show_node(&mut self, id: NodeId) {
        let folded: Vec<NodeId> = self
            .map
            .ancestors(id)
            .filter(|&ancestor| self.map.node(ancestor).is_some_and(Node::is_collapsed))
            .collect();
        if !folded.is_empty() {
            self.edit_map(|map| {
                folded
                    .iter()
                    .try_for_each(|&ancestor| map.set_collapsed(ancestor, false))
            });
        }
        self.selected = Some(id);
        self.reveal = Some(id);
    }

    /// Copy the selected branch (or the whole map) to the system clipboard as an outline,
    /// removing it from the map when `cut` is true.
    fn copy_branch(&mut self, ctx: &egui::Context, cut: bool) {
        let root = self.map.root();
        let branch = self.selected.unwrap_or(root);
        if cut && branch == root {
            "Select a branch to cut; the central idea cannot be removed"
                .clone_into(&mut self.status);
            return;
        }
        ctx.copy_text(clipboard::branch_outline(&self.map, branch));
        if cut {
            self.selected = Some(branch);
            self.perform(Action::Delete);
            "Cut to the clipboard".clone_into(&mut self.status);
        } else {
            "Copied to the clipboard".clone_into(&mut self.status);
        }
    }

    /// Paste an outline from the clipboard as new branches under the selected node.
    fn paste_outline(&mut self, text: &str) {
        let items = clipboard::parse_outline(text);
        if items.is_empty() {
            "Nothing to paste: the clipboard holds no text".clone_into(&mut self.status);
            return;
        }
        let target = self.selected.unwrap_or_else(|| self.map.root());
        if let Some(branches) = self.edit_map(|map| clipboard::paste(map, target, &items)) {
            self.status = format!("Pasted {} nodes", items.len());
            if let Some(&first) = branches.first() {
                self.selected = Some(first);
                self.reveal = Some(first);
            }
        }
    }

    /// Pan (without zooming) so `id` is comfortably inside `canvas`, if it isn't already.
    fn scroll_into_view(&mut self, id: NodeId, canvas: Rect) {
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

    /// Install the chosen theme in egui if it changed since the last frame.
    fn sync_theme(&mut self, ctx: &egui::Context) {
        if self.applied_theme != Some(self.theme) {
            // `None` is the first frame, applying the saved theme: nothing new to save.
            if self.applied_theme.is_some() {
                let theme = self.theme;
                self.update_settings(|settings| settings.theme = theme);
            }
            self.theme.apply(ctx);
            self.applied_theme = Some(self.theme);
        }
    }

    fn handle_window(&mut self, ui: &Ui) {
        let ctx = ui.ctx();
        self.sync_theme(ctx);
        let zoom = ctx.zoom_factor();
        if (zoom - self.ui_zoom).abs() > f32::EPSILON {
            self.ui_zoom = zoom;
            self.update_settings(|settings| settings.ui_zoom = zoom);
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
        // While a text field has focus, keys (and Ctrl+C/X/V) belong to it.
        if self.editing.is_some() || ui.ctx().egui_wants_keyboard_input() {
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
        if on_start {
            return;
        }
        let request = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                Event::Copy => Some(ClipboardRequest::Copy),
                Event::Cut => Some(ClipboardRequest::Cut),
                Event::Paste(text) => Some(ClipboardRequest::Paste(text.clone())),
                _ => None,
            })
        });
        match request {
            Some(ClipboardRequest::Copy) => self.copy_branch(ctx, false),
            Some(ClipboardRequest::Cut) => self.copy_branch(ctx, true),
            Some(ClipboardRequest::Paste(text)) => self.paste_outline(&text),
            None => {}
        }
    }
}

/// Remove plain Enter presses (without Shift) from `events`.
///
/// # Returns
///
/// Whether any were removed.
fn take_plain_enter(events: &mut Vec<Event>) -> bool {
    let before = events.len();
    events.retain(|event| {
        !matches!(
            event,
            Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift
        )
    });
    events.len() != before
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
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.handle_window(ui);
        self.poll_jobs();
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
        if let Some(search) = &mut self.search {
            search.refresh(&self.map, self.revision);
            let command = egui::Panel::top("find_bar")
                .show(ui, |ui| search.show(ui))
                .inner;
            if let Some(command) = command {
                self.search_command(command);
            }
        }
        self.details_panel(ui);
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
    fn multi_line_labels_keep_inner_line_breaks() {
        let (mut app, child) = app_with_child();
        app.start_edit(child, " First line\r\nsecond line \n".to_owned());

        app.commit_edit();

        assert_eq!(
            app.map.node(child).expect("child").text(),
            "First line\nsecond line"
        );
    }

    #[test]
    fn only_plain_enter_is_taken() {
        let key = |modifiers| Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let mut events = vec![key(Modifiers::SHIFT), Event::Text("x".to_owned())];
        assert!(!take_plain_enter(&mut events));
        assert_eq!(events.len(), 2);

        events.push(key(Modifiers::NONE));
        assert!(take_plain_enter(&mut events));

        assert_eq!(events.len(), 2);
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
    fn arrows_start_at_root_then_follow_the_map() {
        let (mut app, child) = app_with_child();
        app.selected = None;

        app.perform(Action::Navigate(Direction::Down));
        assert_eq!(app.selected, Some(app.map.root()));
        app.perform(Action::Navigate(Direction::Right));
        assert_eq!(app.selected, Some(child));
        assert_eq!(app.reveal, Some(child));
        // Nothing further out: the selection stays put.
        app.perform(Action::Navigate(Direction::Right));

        assert_eq!(app.selected, Some(child));
    }

    #[test]
    fn scroll_into_view_pans_only_when_needed() {
        let (mut app, child) = app_with_child();
        let canvas = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0));
        let position = app.map.node(child).expect("child").position();
        app.view.camera_mut().look_at(position, canvas.center());
        let centred = *app.view.camera();

        app.scroll_into_view(child, canvas);
        assert_eq!(*app.view.camera(), centred);
        app.view.camera_mut().pan(Vec2::new(5000.0, 0.0));
        app.scroll_into_view(child, canvas);

        let on_screen = app.view.camera().world_to_screen(position);
        assert!(canvas.contains(on_screen));
    }

    #[test]
    fn export_reports_success_and_bad_extensions() {
        let dir = TempDir::new("app_export");
        let (mut app, _) = app_with_child();

        let finish = |app: &mut MindMapApp| {
            while !app.exports.is_empty() {
                app.poll_jobs();
                std::thread::yield_now();
            }
        };

        app.export_to(&dir.path().join("map.md"));
        assert_eq!(app.status, "Exporting map.md...");
        finish(&mut app);
        assert_eq!(app.status, "Exported map.md");
        assert!(dir.path().join("map.md").exists());
        app.export_to(&dir.path().join("map.txt"));
        finish(&mut app);

        assert!(app.status.starts_with("Could not export map.txt"));
    }

    #[test]
    fn search_jumps_to_matches_and_unfolds_hidden_ones() {
        let (mut app, child) = app_with_child();
        app.edit_map(|map| map.set_text(child, "Needle"))
            .expect("child");
        let hidden = app
            .edit_map(|map| map.add_child(child, "another needle"))
            .expect("child");
        app.toggle_collapse(child);
        assert!(app.map.node(child).expect("child").is_collapsed());

        app.perform(Action::Find);
        app.search
            .as_mut()
            .expect("find bar open")
            .set_query("needle");
        app.search_command(SearchCommand::QueryChanged);
        assert_eq!(app.selected, Some(child));
        app.perform(Action::FindNext);

        assert_eq!(app.selected, Some(hidden));
        assert!(!app.map.node(child).expect("child").is_collapsed());
        assert_eq!(app.reveal, Some(hidden));
        app.perform(Action::CloseFind);
        assert!(app.search.is_none());
    }

    #[test]
    fn copy_cut_and_paste_branches() {
        let ctx = egui::Context::default();
        let (mut app, child) = app_with_child();
        app.edit_map(|map| map.add_child(child, "grandchild"))
            .expect("child");
        let outline = clipboard::branch_outline(&app.map, child);
        app.selected = Some(child);

        app.copy_branch(&ctx, true);
        assert!(app.map.node(child).is_none());
        app.selected = Some(app.map.root());
        app.paste_outline(&outline);

        let pasted = app.selected.expect("pasted branch selected");
        assert_eq!(clipboard::branch_outline(&app.map, pasted), outline);
        assert_eq!(app.status, "Pasted 2 nodes");
        app.selected = Some(app.map.root());
        app.copy_branch(&ctx, true);
        assert!(app.status.starts_with("Select a branch to cut"));
        app.paste_outline("   ");
        assert!(app.status.starts_with("Nothing to paste"));
    }

    #[test]
    fn details_edits_apply_as_one_undo_step_and_follow_undo() {
        let (mut app, child) = app_with_child();
        app.selected = Some(child);
        let mut panel = DetailsPanel::default();
        panel.follow(&app.map, Some(child));
        app.details = Some(panel);
        let red = details::PALETTE[3];

        app.apply_details(
            child,
            &[
                DetailsEdit::Note("first".to_owned()),
                DetailsEdit::Link(Some("https://example.com".to_owned())),
                DetailsEdit::Color(Some(red)),
            ],
        );
        let node = app.map.node(child).expect("child");
        assert_eq!(
            (node.note(), node.link(), node.color()),
            ("first", Some("https://example.com"), Some(red))
        );
        app.perform(Action::Undo);

        let node = app.map.node(child).expect("child");
        assert_eq!((node.note(), node.link(), node.color()), ("", None, None));
        // The panel reloaded, so it has nothing stale to re-apply.
        assert_eq!(
            app.details.as_ref().expect("panel").pending(&app.map),
            Vec::new()
        );
    }

    #[test]
    fn toggling_details_opens_and_closes_the_panel() {
        let mut app = MindMapApp::with_map(MindMap::new("root"));

        app.perform(Action::ToggleDetails);
        assert!(app.details.is_some());
        app.perform(Action::ToggleDetails);

        assert!(app.details.is_none());
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
        let dir = TempDir::new("app_restore");
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        app.recovery = Some(RecoveryStore::for_this_window(dir.path()).expect("store"));
        let mut map = MindMap::new("recovered");
        map.add_child(map.root(), "idea").expect("root exists");
        app.recovered = Some(Recovered {
            map: map.clone(),
            source: Some(PathBuf::from("ideas.json")),
            files: None,
        });
        app.deferred_open = Some(PathBuf::from("other.json"));

        app.choose(StartChoice::Restore);

        assert_eq!(app.screen, Screen::Editor);
        assert_eq!(app.map, map);
        assert_eq!(app.file_path, Some(PathBuf::from("ideas.json")));
        assert!(app.dirty);
        // Copied into this window's own recovery file straight away.
        let copy = app
            .recovery
            .as_ref()
            .expect("store")
            .load()
            .expect("readable");
        assert_eq!(copy.expect("copy written").map, map);
        assert!(app.recovered.is_none());
        assert!(app.deferred_open.is_none());
    }

    #[test]
    fn discard_recovery_deletes_orphan_and_opens_deferred_file() {
        let dir = TempDir::new("app_discard");
        // A window that crashed with unsaved work.
        let crashed = RecoveryStore::for_this_window(dir.path()).expect("store");
        crashed
            .save(&MindMap::new("recovered"), None)
            .expect("save succeeds");
        crashed.simulate_crash();
        let deferred = dir.path().join("deferred.json");
        storage::save_to_path(&MindMap::new("deferred"), &deferred).expect("save succeeds");
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        app.recovery = Some(RecoveryStore::for_this_window(dir.path()).expect("store"));
        app.find_orphan();
        assert!(app.recovered.is_some());
        app.deferred_open = Some(deferred.clone());

        app.choose(StartChoice::DiscardRecovery);

        assert!(app.recovered.is_none());
        let recovery_files = std::fs::read_dir(dir.path())
            .expect("readable")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("recovery"))
            .count();
        assert_eq!(recovery_files, 0);
        assert_eq!(app.file_path, Some(deferred));
        assert_eq!(app.screen, Screen::Editor);
    }

    #[test]
    fn autosave_waits_for_interval_then_writes_recovery_copy() {
        let dir = TempDir::new("app_autosave");
        let ctx = egui::Context::default();
        let (mut app, _) = app_with_child();
        app.recovery = Some(RecoveryStore::for_this_window(dir.path()).expect("store"));
        let saved_copy = |app: &MindMapApp| {
            app.recovery
                .as_ref()
                .expect("store set")
                .load()
                .expect("load succeeds")
        };
        // The app has just started (time 0), so the first interval has not elapsed.
        app.autosave(&ctx);
        assert!(saved_copy(&app).is_none());

        app.last_autosave = -AUTOSAVE_SECONDS;
        app.autosave(&ctx);
        assert!(app.recovery_job.is_some(), "written in the background");
        app.finish_recovery_job();

        assert_eq!(saved_copy(&app).expect("copy written").map, app.map);
        assert!(!app.autosave_pending);
    }

    #[test]
    fn clearing_waits_for_a_background_copy_so_none_is_left() {
        let dir = TempDir::new("app_clear_waits");
        let ctx = egui::Context::default();
        let (mut app, _) = app_with_child();
        app.recovery = Some(RecoveryStore::for_this_window(dir.path()).expect("store"));
        app.last_autosave = -AUTOSAVE_SECONDS;
        app.autosave(&ctx);

        app.clear_recovery();

        assert!(app.recovery_job.is_none());
        let store = app.recovery.as_ref().expect("store");
        assert!(store.load().expect("readable").is_none());
    }

    #[test]
    fn huge_maps_keep_fewer_undo_steps() {
        let mut map = MindMap::new("root");
        // About 8 MB of notes: the 256 MB budget then fits about 30 snapshots, not 200.
        let root = map.root();
        map.set_note(root, &"x".repeat(8 * 1024 * 1024))
            .expect("root");
        let mut app = MindMapApp::with_map(map);

        for step in 0..60 {
            app.edit_map(|map| map.set_text(root, &format!("step {step}")));
        }

        let kept = app.history.undo_steps();
        assert!((10..60).contains(&kept), "kept {kept} steps");
    }

    #[test]
    fn saving_removes_recovery_copy() {
        let dir = TempDir::new("app_save_clears");
        let (mut app, _) = app_with_child();
        let store = RecoveryStore::for_this_window(dir.path()).expect("store");
        store.save(&app.map, None).expect("save succeeds");
        app.recovery = Some(store);

        app.save_to(dir.path().join("map.json"));

        assert!(!app.dirty);
        let store = app.recovery.as_ref().expect("store set");
        assert!(store.load().expect("load succeeds").is_none());
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
    fn settings_changes_are_saved_to_the_shared_file() {
        let dir = TempDir::new("app_settings");
        let ctx = egui::Context::default();
        let mut app = MindMapApp::with_map(MindMap::new("root"));
        app.settings_file = Some(SettingsFile::in_dir(dir.path()));
        // Another window adds a recent file meanwhile.
        SettingsFile::in_dir(dir.path())
            .update(|settings| settings.recent.add(Path::new("other.json")))
            .expect("update succeeds");

        app.sync_theme(&ctx);
        app.theme = ColorTheme::LightGrey;
        app.sync_theme(&ctx);
        app.save_to(dir.path().join("mine.json"));

        let saved = SettingsFile::in_dir(dir.path()).load();
        assert_eq!(saved.theme, ColorTheme::LightGrey);
        assert_eq!(saved.recent.paths().len(), 2, "both windows' files kept");
        assert_eq!(app.recent, saved.recent);
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
