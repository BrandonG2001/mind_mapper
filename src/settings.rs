//! User settings (theme, interface size, recent files), shared safely between windows.
//!
//! Each change is a read-modify-write of `settings.json` while holding an exclusive lock
//! on `settings.lock`, so two windows changing different settings never overwrite each
//! other: each change is applied on top of whatever the file holds at that moment.

use std::fs::{self, File, OpenOptions};
use std::io::BufReader;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::recent::RecentFiles;
use crate::storage::{StorageError, write_atomically};
use crate::theme::ColorTheme;

/// Name of the settings file inside the app's data directory.
const SETTINGS_FILE_NAME: &str = "settings.json";
/// Name of the lock file that serialises updates from several windows.
const LOCK_FILE_NAME: &str = "settings.lock";
/// Interface sizes accepted when loading, matching the range egui's zoom keys allow.
const UI_ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.2..=5.0;

/// Everything remembered between runs, apart from window size and position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// Missing fields take their default, so files from older or newer versions still load.
#[serde(default)]
pub struct Settings {
    /// Colour theme.
    pub theme: ColorTheme,
    /// Interface size (egui's zoom factor); 1.0 = 100%.
    pub ui_zoom: f32,
    /// Recently opened or saved maps.
    pub recent: RecentFiles,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ColorTheme::default(),
            ui_zoom: 1.0,
            recent: RecentFiles::default(),
        }
    }
}

impl Settings {
    /// Replace values that are out of range with their defaults.
    #[must_use]
    fn sanitized(mut self) -> Self {
        if !UI_ZOOM_RANGE.contains(&self.ui_zoom) {
            self.ui_zoom = 1.0;
        }
        self
    }
}

/// The settings file in the app's data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsFile {
    path: PathBuf,
    lock_path: PathBuf,
}

impl SettingsFile {
    /// The settings file in `dir` (created on the first update).
    #[must_use]
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: dir.join(SETTINGS_FILE_NAME),
            lock_path: dir.join(LOCK_FILE_NAME),
        }
    }

    /// Whether the file has been written yet.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Read the current settings; defaults if the file is missing or unreadable.
    #[must_use]
    pub fn load(&self) -> Settings {
        File::open(&self.path)
            .ok()
            .and_then(|file| serde_json::from_reader::<_, Settings>(BufReader::new(file)).ok())
            .unwrap_or_default()
            .sanitized()
    }

    /// Apply `change` to the latest settings on disk and save the result.
    ///
    /// Holds an exclusive lock for the duration, so updates from several windows are
    /// applied one after another instead of overwriting each other.
    ///
    /// # Returns
    ///
    /// The settings as saved, including changes made by other windows.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` or `StorageError::Json` if the file cannot be written.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<Settings, StorageError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.lock_path)?;
        // Waits while another window is updating; released when `lock` is dropped.
        lock.lock()?;
        let mut settings = self.load();
        change(&mut settings);
        write_atomically(&self.path, |writer| {
            serde_json::to_writer_pretty(writer, &settings)?;
            Ok(())
        })?;
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn missing_or_corrupt_file_gives_defaults() {
        let dir = TempDir::new("settings_missing");
        let file = SettingsFile::in_dir(dir.path());
        assert!(!file.exists());
        assert_eq!(file.load(), Settings::default());

        fs::write(dir.path().join(SETTINGS_FILE_NAME), "not json").expect("writable");

        assert_eq!(file.load(), Settings::default());
    }

    #[test]
    fn updates_from_two_windows_are_merged() {
        let dir = TempDir::new("settings_merge");
        // Two windows, each with its own handle on the same file.
        let first = SettingsFile::in_dir(dir.path());
        let second = SettingsFile::in_dir(dir.path());

        first
            .update(|settings| settings.recent.add(Path::new("a.json")))
            .expect("update succeeds");
        second
            .update(|settings| settings.theme = ColorTheme::DarkBlue)
            .expect("update succeeds");
        let latest = first
            .update(|settings| settings.ui_zoom = 1.5)
            .expect("update succeeds");

        assert_eq!(latest.theme, ColorTheme::DarkBlue);
        assert_eq!(latest.recent.paths().len(), 1);
        assert_eq!(latest.ui_zoom, 1.5);
        assert_eq!(second.load(), latest);
    }

    #[test]
    fn out_of_range_size_and_partial_files_are_tolerated() {
        let dir = TempDir::new("settings_partial");
        let file = SettingsFile::in_dir(dir.path());
        fs::write(
            dir.path().join(SETTINGS_FILE_NAME),
            r#"{"theme": "LightPurple", "ui_zoom": 99.0}"#,
        )
        .expect("writable");

        let settings = file.load();

        assert_eq!(settings.theme, ColorTheme::LightPurple);
        assert_eq!(settings.ui_zoom, 1.0);
        assert_eq!(settings.recent, RecentFiles::default());
    }
}
