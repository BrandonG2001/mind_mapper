//! Helpers shared by unit tests.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// In-memory stand-in for eframe's on-disk settings file.
#[derive(Default)]
pub struct MemoryStorage(HashMap<String, String>);

impl eframe::Storage for MemoryStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }

    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }

    fn flush(&mut self) {}
}

/// A fresh, empty directory under the system temp folder, deleted when dropped.
///
/// Each instance gets a unique name (process id plus a counter), so tests running in
/// parallel never share files.
pub struct TempDir(PathBuf);

impl TempDir {
    /// Create a new empty directory whose name includes `label`.
    pub fn new(label: &str) -> Self {
        // A process-wide counter; `fetch_add` is atomic, so parallel tests get unique ids.
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mind_mapper_test_{}_{unique}_{label}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("system temp directory is writable");
        Self(path)
    }

    /// Location of the directory.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Cleanup is best effort; a leftover temp folder is harmless.
        std::fs::remove_dir_all(&self.0).ok();
    }
}
