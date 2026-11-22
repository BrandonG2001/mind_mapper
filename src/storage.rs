//! Reading and writing mind maps as JSON.
//!
//! The core functions work on any `Read`/`Write` stream, so tests run against in-memory
//! buffers. Saving to disk goes through [`write_atomically`], so a crash or full disk
//! mid-save can never leave a half-written file behind.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{MapError, MindMap, NodeRecord};

/// File format version written by this build.
pub(crate) const FORMAT_VERSION: u32 = 1;

/// Errors that can occur while saving or loading a map.
#[derive(Debug, Error)]
pub enum StorageError {
    /// The file could not be opened, read or written.
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not valid mind-map JSON.
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The file was written by a newer, incompatible version of the program.
    #[error("unsupported file version {0} (expected {FORMAT_VERSION})")]
    UnsupportedVersion(u32),
    /// The JSON is well formed but does not describe a valid tree.
    #[error("invalid mind map: {0}")]
    InvalidMap(#[from] MapError),
}

/// On-disk layout: a format version plus the nodes in pre-order.
#[derive(Serialize, Deserialize)]
struct MapFile {
    version: u32,
    nodes: Vec<NodeRecord>,
}

/// Write `map` as pretty-printed JSON.
///
/// # Errors
///
/// Returns `StorageError::Json` or `StorageError::Io` if writing fails.
pub fn write_map<W: Write>(map: &MindMap, writer: W) -> Result<(), StorageError> {
    let file = MapFile {
        version: FORMAT_VERSION,
        nodes: map.to_records(),
    };
    serde_json::to_writer_pretty(writer, &file)?;
    Ok(())
}

/// Parse a map previously produced by [`write_map`].
///
/// # Errors
///
/// * `StorageError::Json` if the input is not valid JSON in the expected shape
/// * `StorageError::UnsupportedVersion` for an unknown format version
/// * `StorageError::InvalidMap` if the nodes do not form a valid tree
pub fn read_map<R: Read>(reader: R) -> Result<MindMap, StorageError> {
    let file: MapFile = serde_json::from_reader(reader)?;
    if file.version != FORMAT_VERSION {
        return Err(StorageError::UnsupportedVersion(file.version));
    }
    Ok(MindMap::from_records(file.nodes)?)
}

/// Save `map` to `path`, replacing any existing file atomically.
///
/// # Errors
///
/// See [`write_map`] and [`write_atomically`].
pub fn save_to_path(map: &MindMap, path: &Path) -> Result<(), StorageError> {
    write_atomically(path, |writer| write_map(map, writer))
}

/// Replace the file at `path` with whatever `write` produces, all or nothing.
///
/// The data goes to a temporary file next to `path`, is flushed to disk, and only then
/// renamed over `path`. Renaming within one directory is atomic on Windows and Linux, so
/// `path` always holds either the complete old contents or the complete new contents.
///
/// # Arguments
///
/// * `path` - Final location of the file
/// * `write` - Callback that writes the file's contents
///
/// # Errors
///
/// Returns `StorageError::Io` if the file cannot be written or renamed, or whatever
/// error `write` returns. On failure `path` is untouched and the temporary file is removed.
pub fn write_atomically(
    path: &Path,
    write: impl FnOnce(&mut dyn Write) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    let temp = temp_path(path);
    // An immediately-called closure lets `?` be used while still running the cleanup
    // below on every error path.
    let result = (|| {
        // BufWriter batches the many small writes serde makes into few system calls.
        let mut writer = BufWriter::new(File::create(&temp)?);
        write(&mut writer)?;
        let file = writer
            .into_inner()
            .map_err(std::io::IntoInnerError::into_error)?;
        // Force the bytes to disk before the rename makes them visible.
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        // Best effort: the original error is the one worth reporting.
        fs::remove_file(&temp).ok();
    }
    result
}

/// Sibling path used while writing `path`, e.g. `map.json` -> `map.json.tmp`.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Load a map from `path`.
///
/// # Errors
///
/// See [`read_map`]; also fails if the file cannot be opened.
pub fn load_from_path(path: &Path) -> Result<MindMap, StorageError> {
    read_map(BufReader::new(File::open(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn save_replaces_existing_file_and_leaves_no_temp_file() {
        let dir = TempDir::new("save_replaces");
        let path = dir.path().join("map.json");
        fs::write(&path, "old contents").expect("temp dir is writable");
        let map = MindMap::new("fresh");

        save_to_path(&map, &path).expect("save succeeds");

        let loaded = load_from_path(&path).expect("saved file loads");
        assert_eq!(loaded.to_records(), map.to_records());
        assert!(!temp_path(&path).exists());
    }

    #[test]
    fn failed_write_keeps_original_file() {
        let dir = TempDir::new("failed_write");
        let path = dir.path().join("map.json");
        fs::write(&path, "original").expect("temp dir is writable");

        let result = write_atomically(&path, |writer| {
            writer.write_all(b"partial")?;
            Err(StorageError::UnsupportedVersion(0))
        });

        assert!(matches!(result, Err(StorageError::UnsupportedVersion(0))));
        assert_eq!(fs::read_to_string(&path).expect("file exists"), "original");
        assert!(!temp_path(&path).exists());
    }

    #[test]
    fn temp_path_is_a_sibling() {
        let path = Path::new("dir").join("map.json");

        assert_eq!(temp_path(&path), Path::new("dir").join("map.json.tmp"));
    }

    #[test]
    fn write_then_read_round_trips() {
        let mut map = MindMap::new("root");
        let child = map.add_child(map.root(), "child").expect("root exists");
        map.add_child(child, "grandchild with \"quotes\" and ümlauts")
            .expect("child exists");
        let mut buffer = Vec::new();

        write_map(&map, &mut buffer).expect("writing to memory succeeds");
        let loaded = read_map(buffer.as_slice()).expect("valid file");

        assert_eq!(loaded.to_records(), map.to_records());
    }

    #[test]
    fn rejects_malformed_json() {
        let result = read_map(b"{ not json".as_slice());

        assert!(matches!(result, Err(StorageError::Json(_))));
    }

    #[test]
    fn rejects_unknown_version() {
        let result = read_map(br#"{"version": 99, "nodes": []}"#.as_slice());

        assert!(matches!(result, Err(StorageError::UnsupportedVersion(99))));
    }

    #[test]
    fn rejects_invalid_tree() {
        let json = br#"{"version": 1, "nodes": [
            {"parent": null, "text": "a", "position": {"x": 0, "y": 0}},
            {"parent": 5, "text": "b", "position": {"x": 0, "y": 0}}
        ]}"#;

        let result = read_map(json.as_slice());

        assert!(matches!(
            result,
            Err(StorageError::InvalidMap(MapError::InvalidParent {
                index: 1,
                parent: 5
            }))
        ));
    }
}
