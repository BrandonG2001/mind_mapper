//! Opening, saving and exporting maps, unsaved-change prompts and settings.

use std::path::{Path, PathBuf};

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};

use super::{FILE_FILTER, MindMapApp, file_label};
use crate::background::Job;
use crate::export::{self, ExportStyle};
use crate::settings::Settings;
use crate::storage;

impl MindMapApp {
    /// Apply `change` to the shared settings file (on top of other windows' changes) and
    /// adopt the merged recent-files list. Without a data folder, only memory changes.
    pub(super) fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
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

    pub(super) fn open_path(&mut self, path: &Path) {
        match storage::load_from_path(path) {
            Ok((map, stamp)) => {
                self.status = format!("Opened {}", file_label(path));
                self.update_settings(|settings| settings.recent.add(path));
                self.replace_map(map, Some(path.to_owned()));
                self.file_stamp = Some(stamp);
            }
            Err(err) => {
                self.status = format!("Could not open {}: {err}", path.display());
                // Drop dead entries (moved or deleted files) from the recent list.
                self.update_settings(|settings| settings.recent.remove(path));
            }
        }
    }

    /// Save the map to `path`, first asking if that would overwrite changes another window
    /// or program made to the file.
    pub(super) fn save_to(&mut self, path: PathBuf) {
        if self.would_overwrite_changes(&path) && !confirm_overwrite(&path) {
            self.status = format!(
                "Not saved: {} was changed elsewhere; use Save As to keep both versions",
                file_label(&path)
            );
            return;
        }
        match storage::save_to_path(&self.map, &path) {
            Ok(stamp) => {
                self.file_stamp = Some(stamp);
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

    /// Whether saving to `path` would replace contents this window has not seen: the
    /// map's own file changed on disk since it was opened or last saved here (or was never
    /// read in this session, as after restoring crash-recovered work).
    ///
    /// Saving over a different, existing file is already confirmed by the file dialog.
    pub(super) fn would_overwrite_changes(&self, path: &Path) -> bool {
        if self.file_path.as_deref() != Some(path) {
            return false;
        }
        // If the file cannot be read, let the save itself report the problem.
        storage::current_stamp(path)
            .ok()
            .flatten()
            .is_some_and(|on_disk| self.file_stamp != Some(on_disk))
    }

    /// Ask where to export, then export there.
    pub(super) fn export_dialog(&mut self) {
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

    /// Export the map to `path` (format from its extension) in the current theme's colours.
    ///
    /// Runs on a background thread (a PNG of a huge map takes seconds); the status line
    /// reports the result when it is done.
    pub(super) fn export_to(&mut self, path: &Path) {
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
    pub(super) fn confirm_discard(&mut self) -> bool {
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
}

/// Ask whether to replace a file that changed on disk since this window read it.
fn confirm_overwrite(path: &Path) -> bool {
    MessageDialog::new()
        .set_title("File changed on disk")
        .set_description(format!(
            "{} was changed by another window or program after you opened it.\n\n\
             Save anyway and replace those changes with this version? \
             (Choose No, then Save As, to keep both.)",
            file_label(path)
        ))
        .set_buttons(MessageButtons::YesNo)
        .show()
        == MessageDialogResult::Yes
}

/// A file dialog pre-configured for mind-map files.
pub(super) fn file_dialog() -> FileDialog {
    FileDialog::new().add_filter(FILE_FILTER.0, FILE_FILTER.1)
}
