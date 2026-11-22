//! Opening, saving and exporting maps, unsaved-change prompts and settings.

use std::path::{Path, PathBuf};

use rfd::FileDialog;

use super::questions::Question;
use super::{FILE_FILTER, IMPORT_FILTERS, MindMapApp, file_label};
use crate::background::Job;
use crate::export::{self, ExportStyle};
use crate::formats::{self, Format};
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
                check_updates: self.updates.enabled,
                last_update_check: None,
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

    /// Open the map file at `path`, or import it if it is in another app's format.
    pub(super) fn open_path(&mut self, path: &Path) {
        if let Some(format) = Format::from_path(path) {
            self.import_path(path, format);
            return;
        }
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

    /// Open `path`, an OPML or FreeMind file, as a new map that is not linked to any file
    /// yet (saving asks where to keep it as a mind-map file, so the original is never
    /// overwritten with this app's format).
    fn import_path(&mut self, path: &Path, format: Format) {
        match formats::import(path, format) {
            Ok(map) => {
                self.update_settings(|settings| settings.recent.add(path));
                self.replace_map(map, None);
                self.status = format!("Imported {}; Save keeps it as a mind map", file_label(path));
            }
            Err(err) => {
                self.status = format!("Could not import {}: {err}", path.display());
                self.update_settings(|settings| settings.recent.remove(path));
            }
        }
    }

    /// Save the map to `path`, first asking if that would overwrite changes another window
    /// or program made to the file (then the save happens once the user agrees).
    pub(super) fn save_to(&mut self, path: PathBuf) {
        if self.would_overwrite_changes(&path) {
            self.question = Some(Question::Overwrite(path));
        } else {
            self.write_to(path);
        }
    }

    /// Save the map to `path` without asking, and report the result in the status line.
    pub(super) fn write_to(&mut self, path: PathBuf) {
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

    /// Save the map as one of the user's templates, named after its central idea.
    pub(super) fn save_as_template(&mut self) {
        let Some(folder) = &self.template_folder else {
            "Templates cannot be saved: the app's data folder is unavailable"
                .clone_into(&mut self.status);
            return;
        };
        self.status = match folder.save(&self.map) {
            Ok(saved) if saved.replaced => format!("Replaced the template \"{}\"", saved.name),
            Ok(saved) => format!(
                "Saved \"{}\" as a template; New shows it on the start screen",
                saved.name
            ),
            Err(err) => format!("Could not save the template: {err}"),
        };
        self.reload_templates();
    }

    /// Delete the user's template stored at `path`, then refresh the start screen.
    pub(super) fn remove_template(&mut self, name: &str, path: &Path) {
        let Some(folder) = &self.template_folder else {
            return;
        };
        self.status = match folder.remove(path) {
            Ok(()) => format!("Removed the template \"{name}\""),
            Err(err) => format!("Could not remove the template: {err}"),
        };
        self.reload_templates();
    }

    /// Show the templates currently in the templates folder on the start screen.
    pub(super) fn reload_templates(&mut self) {
        if let Some(folder) = &self.template_folder {
            self.start.set_saved(folder.load());
        }
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
            .add_filter("PDF document (light colours, for printing)", &["pdf"])
            .add_filter("Markdown outline", &["md"])
            .add_filter(IMPORT_FILTERS[0].0, IMPORT_FILTERS[0].1)
            .add_filter(IMPORT_FILTERS[1].0, IMPORT_FILTERS[1].1)
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
}

/// A file dialog pre-configured for saving mind-map files.
pub(super) fn file_dialog() -> FileDialog {
    FileDialog::new().add_filter(FILE_FILTER.0, FILE_FILTER.1)
}

/// A file dialog for opening: mind maps, plus the formats that can be imported.
pub(super) fn open_dialog() -> FileDialog {
    let all: Vec<&str> = std::iter::once(FILE_FILTER)
        .chain(IMPORT_FILTERS)
        .flat_map(|(_, extensions)| extensions.iter().copied())
        .collect();
    // The first filter is the one selected when the dialog opens.
    std::iter::once(FILE_FILTER).chain(IMPORT_FILTERS).fold(
        FileDialog::new().add_filter("Mind maps and outlines", &all),
        |dialog, (name, extensions)| dialog.add_filter(name, extensions),
    )
}
