//! The list of recently opened or saved maps, shown on the start screen.

use std::path::{Path, PathBuf};

use eframe::Storage;
use serde::{Deserialize, Serialize};

/// Key under which the list is kept in eframe's settings storage.
const STORAGE_KEY: &str = "recent_files";
/// How many files are remembered.
const LIMIT: usize = 8;

/// Most-recently-used map files, newest first, without duplicates.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    /// Read the list that versions before 0.5 kept in eframe's storage (used once, to
    /// carry it over into the shared settings file); empty if there is none.
    #[must_use]
    pub fn load(storage: Option<&dyn Storage>) -> Self {
        storage
            .and_then(|storage| eframe::get_value(storage, STORAGE_KEY))
            .unwrap_or_default()
    }

    /// The remembered files, newest first.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Move `path` to the front (adding it if new) and forget the oldest beyond the limit.
    ///
    /// The path is made absolute first, so the same file opened through different
    /// relative paths is listed once.
    pub fn add(&mut self, path: &Path) {
        // `absolute` only joins with the current directory: unlike `canonicalize` it does
        // not touch the file system or produce `\\?\` paths on Windows.
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
        self.paths.retain(|existing| *existing != path);
        self.paths.insert(0, path);
        self.paths.truncate(LIMIT);
    }

    /// Forget `path`, e.g. because it no longer opens.
    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|existing| existing != path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MemoryStorage;

    fn absolute(name: &str) -> PathBuf {
        std::path::absolute(name).expect("current directory is readable")
    }

    #[test]
    fn add_puts_newest_first_without_duplicates() {
        let mut recent = RecentFiles::default();

        recent.add(Path::new("a.json"));
        recent.add(Path::new("b.json"));
        recent.add(Path::new("a.json"));

        assert_eq!(recent.paths(), &[absolute("a.json"), absolute("b.json")]);
    }

    #[test]
    fn list_is_capped() {
        let mut recent = RecentFiles::default();

        for index in 0..LIMIT + 3 {
            recent.add(Path::new(&format!("{index}.json")));
        }

        assert_eq!(recent.paths().len(), LIMIT);
        assert_eq!(recent.paths()[0], absolute(&format!("{}.json", LIMIT + 2)));
    }

    #[test]
    fn remove_forgets_a_path() {
        let mut recent = RecentFiles::default();
        recent.add(Path::new("a.json"));
        recent.add(Path::new("b.json"));

        recent.remove(&absolute("a.json"));

        assert_eq!(recent.paths(), &[absolute("b.json")]);
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut storage = MemoryStorage::default();
        let mut recent = RecentFiles::default();
        recent.add(Path::new("a.json"));

        eframe::set_value(&mut storage, STORAGE_KEY, &recent);

        assert_eq!(RecentFiles::load(Some(&storage)), recent);
        assert_eq!(RecentFiles::load(None), RecentFiles::default());
    }
}
