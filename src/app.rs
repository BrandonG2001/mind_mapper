//! The egui application: start screen, toolbar, canvas and user interaction.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Event, Key, KeyboardShortcut, Modifiers, Ui, ViewportCommand};

use crate::background::Job;
use crate::details::DetailsPanel;
use crate::export::ExportError;
use crate::history::{History, HistoryLimits};
use crate::layout;
use crate::map_view::MapView;
use crate::model::{MindMap, Node, NodeId};
use crate::navigation::Direction;
use crate::recent::RecentFiles;
use crate::recovery::{Recovered, RecoveryStore};
use crate::search::{Search, SearchCommand};
use crate::settings::{Settings, SettingsFile};
use crate::start_screen::{StartContext, StartScreen};
use crate::storage::{FileStamp, StorageError};
use crate::theme::{self, ColorTheme};
use crate::user_templates::TemplateFolder;
use questions::{AfterDiscard, Question};

mod accessibility;
mod autosave;
mod canvas;
mod editing;
mod files;
mod focus;
mod links;
mod outline;
mod panels;
mod pictures;
mod questions;
mod selection;
mod updates;

/// Application name: window title, and the folder name for settings and recovery data.
pub const APP_NAME: &str = "Mind Mapper";
/// How long after the last change auto-save saves the map file, so a burst of edits
/// (typing, dragging) is saved once.
const AUTO_SAVE_DELAY: f64 = 2.0;
/// Minimum time between background copies of unsaved work.
const AUTOSAVE_SECONDS: f64 = 20.0;
/// Narrowest text box shown while editing, at 100% zoom.
const MIN_EDITOR_WIDTH: f32 = 140.0;
/// Space above and below the text inside the label editor, at 100% zoom.
const EDITOR_PADDING: f32 = 8.0;
/// Longest node label quoted in status messages and menus, in characters.
const MAX_QUOTED_LABEL: usize = 40;
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
/// File-dialog filters for files that can be imported (opened as a new, unsaved map).
const IMPORT_FILTERS: [(&str, &[&str]); 2] =
    [("OPML outline", &["opml"]), ("FreeMind map", &["mm"])];
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
    /// Swap the selected node with its sibling above (`Up`) or below (`Down`).
    Reorder(Direction),
    Export,
    /// Save the map as one of the user's templates on the start screen.
    SaveTemplate,
    Find,
    FindNext,
    FindPrevious,
    /// Open the find bar with its "Replace with" row.
    Replace,
    /// Esc: cancel linking, or close the find bar, or else leave focus mode.
    Dismiss,
    /// Start a cross-link from the selected node to the next node clicked.
    StartLink,
    ToggleDetails,
    /// Show only the selected branch, or the whole map again.
    ToggleFocus,
    /// Show or hide the outline panel.
    ToggleOutline,
}

/// A clipboard request, which egui reports as an event rather than a key press.
#[derive(Debug, Clone, PartialEq)]
enum ClipboardRequest {
    Copy,
    Cut,
    Paste(String),
    /// An image was pasted (the clipboard held a picture rather than text).
    PasteImage(std::sync::Arc<egui::ColorImage>),
}

/// Keyboard shortcuts. Variants with Shift come first because a shortcut without Shift
/// also matches when Shift is held.
const SHORTCUTS: [(KeyboardShortcut, Action); 30] = [
    (
        KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O),
        Action::ToggleOutline,
    ),
    // Before the plain arrows, which also match while Alt is held.
    (
        KeyboardShortcut::new(Modifiers::ALT, Key::ArrowUp),
        Action::Reorder(Direction::Up),
    ),
    (
        KeyboardShortcut::new(Modifiers::ALT, Key::ArrowDown),
        Action::Reorder(Direction::Down),
    ),
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
        KeyboardShortcut::new(Modifiers::COMMAND, Key::H),
        Action::Replace,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::Escape),
        Action::Dismiss,
    ),
    (
        KeyboardShortcut::new(Modifiers::NONE, Key::F6),
        Action::ToggleFocus,
    ),
    (
        KeyboardShortcut::new(Modifiers::COMMAND, Key::K),
        Action::StartLink,
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
    /// Edited in the outline panel's row rather than over the node on the canvas.
    in_outline: bool,
}

/// What the current mouse drag is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Drag {
    /// Moving the selected branches.
    Nodes {
        /// The selected branches being moved (see `selected_branches`).
        branches: Vec<NodeId>,
        /// Node under the cursor that the branches would be re-attached to if dropped now.
        drop_target: Option<NodeId>,
    },
    /// Drawing a selection box (Shift+drag on empty canvas) from this screen point.
    Select { origin: egui::Pos2 },
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
    /// Nodes selected besides the main one (`selected`), with Ctrl+click or a box.
    also_selected: HashSet<NodeId>,
    /// The check for a newer release: its setting, progress and result.
    updates: updates::Updates,
    /// Saving the map file itself shortly after each change (a setting).
    file_autosave: autosave::FileAutoSave,
    /// Where the user's own templates are kept; `None` if there is no data folder.
    template_folder: Option<TemplateFolder>,
    /// The node a cross-link is being drawn from, until the target is clicked.
    linking: Option<NodeId>,
    /// A cross-link label being edited in a pop-up.
    label_edit: Option<links::LabelEdit>,
    /// The outline panel, while it is open.
    outline: Option<outline::Outline>,
    /// A question shown over the window, waiting for the user's answer.
    question: Option<Question>,
    /// The map changed since the last recovery copy was written.
    autosave_pending: bool,
    /// When the last recovery copy was written, in seconds since the app started.
    last_autosave: f64,
    /// Recently opened or saved files, shown on the start screen.
    recent: RecentFiles,
    /// Fingerprint of `file_path` as this window last read or wrote it; `None` if it has
    /// not done either this session (e.g. work restored after a crash).
    file_stamp: Option<FileStamp>,
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
                    ..Settings::default()
                };
                app.update_settings(|settings| *settings = carried_over.clone());
                carried_over
            }
        };
        app.theme = settings.theme;
        app.ui_zoom = settings.ui_zoom;
        app.recent = settings.recent;
        app.updates.enabled = settings.check_updates;
        app.file_autosave.enabled = settings.auto_save;
        app.check_for_update_if_due(settings.last_update_check);
        // Set the option directly (not `set_zoom_factor`, which waits a frame) so the
        // first frame already uses the saved size.
        cc.egui_ctx
            .options_mut(|options| options.zoom_factor = app.ui_zoom);
        if let Some(dir) = data_dir {
            app.template_folder = Some(TemplateFolder::in_dir(&dir));
            app.reload_templates();
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
            question: None,
            linking: None,
            label_edit: None,
            outline: None,
            template_folder: None,
            updates: updates::Updates::default(),
            file_autosave: autosave::FileAutoSave::default(),
            also_selected: HashSet::new(),
            autosave_pending: false,
            last_autosave: 0.0,
            recent: RecentFiles::default(),
            file_stamp: None,
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

    /// Note that the map has unsaved changes (and needs a fresh recovery copy).
    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.autosave_pending = true;
        self.revision = self.revision.wrapping_add(1);
        self.file_autosave.changed_at = self.ctx.input(|input| input.time);
    }

    fn perform(&mut self, action: Action) {
        // Any toolbar or keyboard action first finishes in-progress edits.
        self.commit_edit();
        self.flush_details();
        let root = self.map.root();
        match action {
            Action::New => {
                // Pick up files and templates other windows saved since this one started.
                if let Some(file) = &self.settings_file {
                    self.recent = file.load().recent;
                }
                self.reload_templates();
                self.screen = Screen::Start { can_return: true };
            }
            Action::Open => self.discard_then(AfterDiscard::PickFile),
            Action::Save => match self.file_path.clone() {
                Some(path) => self.save_to(path),
                None => self.perform(Action::SaveAs),
            },
            Action::SaveAs => {
                if let Some(path) = files::file_dialog()
                    .set_file_name("mind_map.json")
                    .save_file()
                {
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
            Action::Delete => self.delete_selection(),
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
            Action::SaveTemplate => self.save_as_template(),
            Action::Navigate(direction) => self.navigate(direction),
            Action::Reorder(direction) => self.reorder(direction),
            Action::Find => match &mut self.search {
                Some(search) => search.request_focus(),
                None => self.search = Some(Search::new()),
            },
            Action::Replace => self.search.get_or_insert_with(Search::new).show_replace(),
            Action::FindNext => self.search_command(SearchCommand::Next),
            Action::FindPrevious => self.search_command(SearchCommand::Previous),
            Action::Dismiss => {
                if !self.cancel_link() && self.search.take().is_none() {
                    self.leave_focus();
                }
            }
            Action::StartLink => self.start_link(),
            Action::ToggleFocus => self.toggle_focus(),
            Action::ToggleOutline => self.toggle_outline(),
            Action::ToggleDetails => {
                if self.details.take().is_none() {
                    self.details = Some(DetailsPanel::default());
                }
            }
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
        if ctx.input(|input| input.viewport().close_requested()) && self.has_unsaved_work() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.question = Some(Question::Discard(AfterDiscard::Close));
        }
        let title = self.title();
        if title != self.window_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn handle_shortcuts(&mut self, ui: &Ui) {
        // While a text field has focus, keys (and Ctrl+C/X/V) belong to it; while a
        // question is open, they belong to the question.
        if self.editing.is_some()
            || self.question.is_some()
            || self.label_edit.is_some()
            || ui.ctx().egui_wants_keyboard_input()
        {
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
                Event::PasteImage(image) => Some(ClipboardRequest::PasteImage(Arc::clone(image))),
                _ => None,
            })
        });
        match request {
            Some(ClipboardRequest::Copy) => self.copy_branch(ctx, false),
            Some(ClipboardRequest::Cut) => self.copy_branch(ctx, true),
            Some(ClipboardRequest::Paste(text)) => self.paste_outline(&text),
            Some(ClipboardRequest::PasteImage(image)) => self.paste_picture(&image),
            None => {}
        }
    }
}

/// A node's label for status messages and menus: its first line, shortened if long.
fn node_label(map: &MindMap, id: NodeId) -> String {
    let text = map.node(id).map_or("", Node::text);
    let first_line = text.lines().next().unwrap_or_default();
    match first_line.char_indices().nth(MAX_QUOTED_LABEL) {
        Some((cut, _)) => format!("{}...", &first_line[..cut]),
        None => first_line.to_owned(),
    }
}

/// Short name for status messages: the file name, so long paths don't widen the toolbar.
fn file_label(path: &Path) -> std::borrow::Cow<'_, str> {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
}

impl eframe::App for MindMapApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.handle_window(ui);
        // Before the panels, so they already ignore clicks while a question is open.
        self.show_question(ui.ctx());
        self.show_label_editor(ui.ctx());
        self.poll_jobs();
        self.autosave(ui.ctx());
        self.handle_shortcuts(ui);
        if let Screen::Start { can_return } = self.screen {
            let recovery_label = self.recovered.as_ref().map(Recovered::description);
            let context = StartContext {
                can_return,
                recovery: recovery_label.as_deref(),
                recent: self.recent.paths(),
                update: self.updates.available.as_ref(),
            };
            let mut check_updates = self.updates.enabled;
            let choice = egui::CentralPanel::default()
                .show(ui, |ui| {
                    self.start
                        .show(ui, &context, &mut self.theme, &mut check_updates)
                })
                .inner;
            if check_updates != self.updates.enabled {
                self.set_check_updates(check_updates);
            }
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
        self.outline_panel(ui);
        self.details_panel(ui);
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
    }
}

#[cfg(test)]
mod ui_tests;

#[cfg(test)]
mod tests;
