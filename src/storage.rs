//! Reading and writing mind maps as JSON.
//!
//! The core functions work on any `Read`/`Write` stream, so tests run against in-memory
//! buffers and never touch the file system.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{MapError, MindMap, NodeRecord};

/// File format version written by this build.
const FORMAT_VERSION: u32 = 1;

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

/// Save `map` to `path`, replacing any existing file.
///
/// # Errors
///
/// See [`write_map`]; also fails if the file cannot be created.
pub fn save_to_path(map: &MindMap, path: &Path) -> Result<(), StorageError> {
    // BufWriter batches the many small writes serde makes into few system calls.
    let mut writer = BufWriter::new(File::create(path)?);
    write_map(map, &mut writer)?;
    writer.flush()?;
    Ok(())
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
