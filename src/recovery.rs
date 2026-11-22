//! Crash recovery: a background copy of unsaved work, offered back on the next launch.

use std::fs::{self, File};
use std::io::{BufReader, ErrorKind};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{MindMap, NodeRecord};
use crate::storage::{FORMAT_VERSION, StorageError, write_atomically};

/// Name of the recovery file inside the app's data directory.
const RECOVERY_FILE_NAME: &str = "recovery.json";

/// On-disk layout. Generic over the path type so saving can borrow (`&Path`) while
/// loading owns (`PathBuf`), without cloning.
#[derive(Serialize, Deserialize)]
struct RecoveryFile<P> {
    version: u32,
    /// The file the map was opened from or last saved to, if any.
    source: Option<P>,
    nodes: Vec<NodeRecord>,
}

/// Unsaved work found from a previous session.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovered {
    /// The recovered map.
    pub map: MindMap,
    /// The file it belonged to, if it had been saved before.
    pub source: Option<PathBuf>,
}

impl Recovered {
    /// One-line description for the start screen, e.g. "Unsaved changes to ideas.json".
    #[must_use]
    pub fn description(&self) -> String {
        match self.source.as_deref().and_then(Path::file_name) {
            Some(name) => format!("Unsaved changes to {}", name.to_string_lossy()),
            None => "An unsaved, untitled mind map".to_owned(),
        }
    }
}

/// Reads and writes the recovery file in one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryStore {
    path: PathBuf,
}

impl RecoveryStore {
    /// Keep the recovery file in `dir` (created on first save if needed).
    #[must_use]
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: dir.join(RECOVERY_FILE_NAME),
        }
    }

    /// Write `map` as the current recovery copy, atomically.
    ///
    /// # Arguments
    ///
    /// * `map` - The unsaved map
    /// * `source` - The file it belongs to, if any
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` or `StorageError::Json` if the file cannot be written.
    pub fn save(&self, map: &MindMap, source: Option<&Path>) -> Result<(), StorageError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = RecoveryFile {
            version: FORMAT_VERSION,
            source,
            nodes: map.to_records(),
        };
        write_atomically(&self.path, |writer| {
            serde_json::to_writer(writer, &file)?;
            Ok(())
        })
    }

    /// Read the recovery copy, if there is one.
    ///
    /// # Returns
    ///
    /// `Ok(None)` when no recovery file exists.
    ///
    /// # Errors
    ///
    /// Returns a `StorageError` if the file exists but cannot be read or is invalid.
    pub fn load(&self) -> Result<Option<Recovered>, StorageError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let file: RecoveryFile<PathBuf> = serde_json::from_reader(BufReader::new(file))?;
        if file.version != FORMAT_VERSION {
            return Err(StorageError::UnsupportedVersion(file.version));
        }
        Ok(Some(Recovered {
            map: MindMap::from_records(file.nodes)?,
            source: file.source,
        }))
    }

    /// Delete the recovery copy. Succeeds if there was none.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` if the file exists but cannot be deleted.
    pub fn clear(&self) -> Result<(), StorageError> {
        match fs::remove_file(&self.path) {
            Err(err) if err.kind() != ErrorKind::NotFound => Err(err.into()),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn save_load_and_clear_round_trip() {
        let dir = TempDir::new("recovery_round_trip");
        // A subfolder that does not exist yet, like a fresh install's data directory.
        let store = RecoveryStore::in_dir(&dir.path().join("data"));
        let mut map = MindMap::new("root");
        map.add_child(map.root(), "child").expect("root exists");
        let source = Path::new("ideas.json");

        store.save(&map, Some(source)).expect("save succeeds");
        let recovered = store.load().expect("load succeeds").expect("file exists");

        assert_eq!(recovered.map.to_records(), map.to_records());
        assert_eq!(recovered.source.as_deref(), Some(source));
        store.clear().expect("clear succeeds");
        assert_eq!(store.load().expect("load succeeds"), None);
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = TempDir::new("recovery_missing");
        let store = RecoveryStore::in_dir(dir.path());

        assert_eq!(store.load().expect("load succeeds"), None);
        store.clear().expect("clearing nothing succeeds");
    }

    #[test]
    fn corrupt_file_is_reported() {
        let dir = TempDir::new("recovery_corrupt");
        let store = RecoveryStore::in_dir(dir.path());
        fs::write(dir.path().join(RECOVERY_FILE_NAME), "garbage").expect("writable");

        assert!(matches!(store.load(), Err(StorageError::Json(_))));
    }

    #[test]
    fn description_names_the_source_file() {
        let mut recovered = Recovered {
            map: MindMap::new("root"),
            source: Some(PathBuf::from("notes").join("ideas.json")),
        };
        assert_eq!(recovered.description(), "Unsaved changes to ideas.json");

        recovered.source = None;

        assert_eq!(recovered.description(), "An unsaved, untitled mind map");
    }
}
