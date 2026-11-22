//! Crash recovery: a background copy of unsaved work, offered back on the next launch.
//!
//! Every running window has its own recovery file, `recovery-<id>.json`, and holds an
//! exclusive lock on a matching `window-<id>.lock` for as long as it runs. The operating
//! system releases that lock when the process ends, even in a crash. So at startup a
//! recovery file whose lock can be taken belongs to a window that is gone (an *orphan*,
//! offered for restoring), while one whose lock is held belongs to a window still open
//! and is left alone. The single `recovery.json` written by versions before 0.5 is
//! treated as an orphan too.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{BufReader, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::{MindMap, NodeRecord};
use crate::storage::{FORMAT_VERSION, StorageError, write_atomically};

/// Recovery file written by versions before per-window recovery.
const LEGACY_FILE_NAME: &str = "recovery.json";
/// Recovery files are named `recovery-<id>.json`.
const RECOVERY_PREFIX: &str = "recovery-";
/// Lock files are named `window-<id>.lock`.
const LOCK_PREFIX: &str = "window-";

/// On-disk layout. Generic over the path type so saving can borrow (`&Path`) while
/// loading owns (`PathBuf`), without cloning.
#[derive(Serialize, Deserialize)]
struct RecoveryFile<P> {
    version: u32,
    /// The file the map was opened from or last saved to, if any.
    source: Option<P>,
    nodes: Vec<NodeRecord>,
}

/// The files of an orphaned recovery copy. Holding this keeps the orphan's lock, so no
/// other window offers the same work at the same time.
#[derive(Debug)]
pub struct OrphanFiles {
    recovery: PathBuf,
    /// The lock file and our handle locking it; `None` for the legacy file.
    lock: Option<(PathBuf, File)>,
}

impl OrphanFiles {
    /// Delete the orphaned copy and its lock file (after restoring or discarding it).
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` if a file exists but cannot be deleted.
    pub fn discard(self) -> Result<(), StorageError> {
        remove_if_present(&self.recovery)?;
        if let Some((path, handle)) = self.lock {
            // Windows cannot delete a file that is still open, so close it first.
            drop(handle);
            remove_if_present(&path)?;
        }
        Ok(())
    }
}

/// Unsaved work found from a window that is no longer running.
#[derive(Debug)]
pub struct Recovered {
    /// The recovered map.
    pub map: MindMap,
    /// The file it belonged to, if it had been saved before.
    pub source: Option<PathBuf>,
    /// The orphaned files, to delete once the work is restored or discarded. `None` when
    /// the work did not come from disk.
    pub files: Option<OrphanFiles>,
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

    /// Delete the orphaned files, if any.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` if a file exists but cannot be deleted.
    pub fn discard(self) -> Result<(), StorageError> {
        self.files.map_or(Ok(()), OrphanFiles::discard)
    }
}

/// Writes one window's recovery file; cheap to clone and send to a background thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryWriter {
    path: PathBuf,
}

impl RecoveryWriter {
    /// Write `map` as the recovery copy, atomically.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` or `StorageError::Json` if the file cannot be written.
    pub fn save(&self, map: &MindMap, source: Option<&Path>) -> Result<(), StorageError> {
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
}

/// This window's recovery file, plus the lock that marks it as in use.
#[derive(Debug)]
pub struct RecoveryStore {
    dir: PathBuf,
    path: PathBuf,
    lock_path: PathBuf,
    /// Kept open for the window's lifetime; closing it releases the lock.
    lock: Option<File>,
}

impl RecoveryStore {
    /// Claim a fresh recovery file in `dir` for this window, creating `dir` if needed.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` if the directory or lock file cannot be created or locked.
    pub fn for_this_window(dir: &Path) -> Result<Self, StorageError> {
        fs::create_dir_all(dir)?;
        // Process id plus the current time is unique even if ids are reused later.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let id = format!("{}-{nanos}", std::process::id());
        let lock_path = dir.join(format!("{LOCK_PREFIX}{id}.lock"));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        lock.try_lock().map_err(|err| match err {
            TryLockError::Error(err) => err,
            TryLockError::WouldBlock => ErrorKind::WouldBlock.into(),
        })?;
        Ok(Self {
            dir: dir.to_owned(),
            path: dir.join(format!("{RECOVERY_PREFIX}{id}.json")),
            lock_path,
            lock: Some(lock),
        })
    }

    /// Write `map` as this window's recovery copy, atomically.
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
        self.writer().save(map, source)
    }

    /// A handle that can write this window's recovery copy from another thread. (The
    /// store itself stays on the UI thread, since it owns the window's lock.)
    #[must_use]
    pub fn writer(&self) -> RecoveryWriter {
        RecoveryWriter {
            path: self.path.clone(),
        }
    }

    /// Read this window's recovery copy, if there is one.
    ///
    /// # Errors
    ///
    /// Returns a `StorageError` if the file exists but cannot be read or is invalid.
    pub fn load(&self) -> Result<Option<Recovered>, StorageError> {
        read_recovery(&self.path)
    }

    /// Delete this window's recovery copy. Succeeds if there was none.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` if the file exists but cannot be deleted.
    pub fn clear(&self) -> Result<(), StorageError> {
        remove_if_present(&self.path)
    }

    /// The newest recovery copy left by a window that is no longer running, with its lock
    /// taken so no other window offers it at the same time.
    ///
    /// # Returns
    ///
    /// `Ok(None)` when there is nothing to recover.
    ///
    /// # Errors
    ///
    /// Returns a `StorageError` if the directory cannot be listed or the newest orphan
    /// cannot be read.
    pub fn find_orphan(&self) -> Result<Option<Recovered>, StorageError> {
        let mut newest: Option<(SystemTime, OrphanFiles)> = None;
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path == self.path {
                continue;
            }
            remove_stale_lock(&path);
            let Some(files) = claim_orphan(&path) else {
                continue;
            };
            let modified = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .unwrap_or(UNIX_EPOCH);
            // Keeping only the newest releases the others' locks (their handles drop), so
            // they are offered on a later launch.
            if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
                newest = Some((modified, files));
            }
        }
        let Some((_, files)) = newest else {
            return Ok(None);
        };
        Ok(read_recovery(&files.recovery)?.map(|recovered| Recovered {
            files: Some(files),
            ..recovered
        }))
    }
}

#[cfg(test)]
impl RecoveryStore {
    /// End this window the way a crash would: release the lock but leave its files.
    pub(crate) fn simulate_crash(self) {
        // `ManuallyDrop` stops `Drop` (the normal-exit cleanup) from running.
        let mut store = std::mem::ManuallyDrop::new(self);
        drop(store.lock.take());
    }
}

impl Drop for RecoveryStore {
    /// On a normal exit, remove this window's lock file. (After a crash it stays, but
    /// unlocked, which is how the next launch recognises the orphan.)
    fn drop(&mut self) {
        // Windows cannot delete a file that is still open, so close it first.
        drop(self.lock.take());
        // Best effort: a leftover, unlocked lock file is harmless.
        fs::remove_file(&self.lock_path).ok();
    }
}

/// If `path` is the lock file of a window that is gone and left no recovery file (it had
/// no unsaved work), delete it so such files do not pile up after crashes.
fn remove_stale_lock(path: &Path) {
    let Some(id) = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(LOCK_PREFIX))
        .and_then(|rest| rest.strip_suffix(".lock"))
    else {
        return;
    };
    if path
        .with_file_name(format!("{RECOVERY_PREFIX}{id}.json"))
        .exists()
    {
        return;
    }
    let unlocked = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .is_ok_and(|handle| handle.try_lock().is_ok());
    // The handle above is already closed, so Windows allows the delete. Best effort:
    // a leftover file is harmless and is retried on the next launch.
    if unlocked {
        fs::remove_file(path).ok();
    }
}

/// If `path` is a recovery file whose window is gone, lock it and return its files.
fn claim_orphan(path: &Path) -> Option<OrphanFiles> {
    let name = path.file_name()?.to_str()?;
    if name == LEGACY_FILE_NAME {
        return Some(OrphanFiles {
            recovery: path.to_owned(),
            lock: None,
        });
    }
    let id = name.strip_prefix(RECOVERY_PREFIX)?.strip_suffix(".json")?;
    let lock_path = path.with_file_name(format!("{LOCK_PREFIX}{id}.lock"));
    // A missing lock file means the window is gone too: creating it is fine.
    let handle = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .ok()?;
    // Succeeds only if no running window holds the lock.
    handle.try_lock().ok()?;
    Some(OrphanFiles {
        recovery: path.to_owned(),
        lock: Some((lock_path, handle)),
    })
}

/// Read a recovery file; `Ok(None)` if it does not exist.
fn read_recovery(path: &Path) -> Result<Option<Recovered>, StorageError> {
    let file = match File::open(path) {
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
        files: None,
    }))
}

/// Delete `path`, treating "already gone" as success.
fn remove_if_present(path: &Path) -> Result<(), StorageError> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != ErrorKind::NotFound => Err(err.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn sample_map() -> MindMap {
        let mut map = MindMap::new("root");
        map.add_child(map.root(), "child").expect("root exists");
        map
    }

    /// Leave behind the files of a window that crashed: a recovery copy and an unlocked
    /// lock file.
    fn crashed_window(dir: &Path, map: &MindMap) {
        let crashed = RecoveryStore::for_this_window(dir).expect("store");
        crashed.save(map, None).expect("save succeeds");
        crashed.simulate_crash();
    }

    /// Names of the files in `dir`.
    fn files_in(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .expect("readable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn save_load_and_clear_round_trip() {
        let dir = TempDir::new("recovery_round_trip");
        // A subfolder that does not exist yet, like a fresh install's data directory.
        let store = RecoveryStore::for_this_window(&dir.path().join("data")).expect("store");
        let map = sample_map();
        let source = Path::new("ideas.json");

        store.save(&map, Some(source)).expect("save succeeds");
        let recovered = store.load().expect("load succeeds").expect("file exists");

        assert_eq!(recovered.map.to_records(), map.to_records());
        assert_eq!(recovered.source.as_deref(), Some(source));
        store.clear().expect("clear succeeds");
        assert!(store.load().expect("load succeeds").is_none());
        store.clear().expect("clearing nothing succeeds");
    }

    #[test]
    fn running_windows_are_not_offered_as_orphans() {
        let dir = TempDir::new("recovery_live");
        let first = RecoveryStore::for_this_window(dir.path()).expect("first window");
        let second = RecoveryStore::for_this_window(dir.path()).expect("second window");
        first.save(&sample_map(), None).expect("save succeeds");

        assert_ne!(first.path, second.path);
        assert!(second.find_orphan().expect("listing works").is_none());
    }

    #[test]
    fn crashed_window_is_offered_once_then_discarded() {
        let dir = TempDir::new("recovery_crash");
        crashed_window(dir.path(), &sample_map());
        let first = RecoveryStore::for_this_window(dir.path()).expect("window");
        let second = RecoveryStore::for_this_window(dir.path()).expect("window");

        let orphan = first
            .find_orphan()
            .expect("listing works")
            .expect("orphan found");
        assert_eq!(orphan.map.to_records(), sample_map().to_records());
        // While the first window holds the orphan, nobody else is offered it.
        assert!(second.find_orphan().expect("listing works").is_none());
        orphan.discard().expect("discard succeeds");

        // Only the two running windows' lock files remain.
        let mut remaining = files_in(dir.path());
        remaining.sort();
        assert_eq!(remaining.len(), 2, "{remaining:?}");
        assert!(remaining.iter().all(|name| name.starts_with(LOCK_PREFIX)));
        assert!(second.find_orphan().expect("listing works").is_none());
    }

    #[test]
    fn stale_lock_files_are_cleaned_up_but_live_ones_kept() {
        let dir = TempDir::new("recovery_stale");
        let crashed = RecoveryStore::for_this_window(dir.path()).expect("window");
        crashed.simulate_crash();
        let live = RecoveryStore::for_this_window(dir.path()).expect("window");

        assert!(live.find_orphan().expect("listing works").is_none());

        assert_eq!(
            files_in(dir.path()),
            [live.lock_path.file_name().expect("name").to_string_lossy()]
        );
    }

    #[test]
    fn legacy_file_is_recovered() {
        let dir = TempDir::new("recovery_legacy");
        let store = RecoveryStore::for_this_window(dir.path()).expect("window");
        let legacy = dir.path().join(LEGACY_FILE_NAME);
        let json = format!(
            r#"{{"version":{FORMAT_VERSION},"source":"old.json","nodes":[{{"parent":null,"text":"old","position":{{"x":0,"y":0}}}}]}}"#
        );
        fs::write(&legacy, json).expect("writable");

        let orphan = store.find_orphan().expect("listing works").expect("found");

        assert_eq!(orphan.description(), "Unsaved changes to old.json");
        orphan.discard().expect("discard succeeds");
        assert!(!legacy.exists());
    }

    #[test]
    fn normal_exit_removes_the_lock_file() {
        let dir = TempDir::new("recovery_exit");
        let store = RecoveryStore::for_this_window(dir.path()).expect("window");
        let lock_path = store.lock_path.clone();
        assert!(lock_path.exists());

        drop(store);

        assert!(!lock_path.exists());
    }

    #[test]
    fn corrupt_orphan_is_reported() {
        let dir = TempDir::new("recovery_corrupt");
        let store = RecoveryStore::for_this_window(dir.path()).expect("window");
        fs::write(dir.path().join("recovery-9-9.json"), "garbage").expect("writable");

        assert!(matches!(store.find_orphan(), Err(StorageError::Json(_))));
    }

    #[test]
    fn description_names_the_source_file() {
        let mut recovered = Recovered {
            map: MindMap::new("root"),
            source: Some(PathBuf::from("notes").join("ideas.json")),
            files: None,
        };
        assert_eq!(recovered.description(), "Unsaved changes to ideas.json");

        recovered.source = None;

        assert_eq!(recovered.description(), "An unsaved, untitled mind map");
    }
}
