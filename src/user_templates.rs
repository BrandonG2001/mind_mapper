//! Templates the user saves from their own maps, shown on the start screen after the
//! built-in ones.
//!
//! Each is an ordinary mind-map file in the `templates` folder of the app's data
//! directory, named after its central idea, so they can also be copied or shared by hand.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::model::{MindMap, Node};
use crate::storage::{self, StorageError};

/// Name of the folder inside the app's data directory.
const FOLDER: &str = "templates";
/// Longest file name stem, in characters, so paths stay well inside system limits.
const MAX_STEM_CHARS: usize = 80;
/// File names Windows reserves for devices, whatever their extension.
const RESERVED_STEMS: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A template saved by the user.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedTemplate {
    name: String,
    path: PathBuf,
    map: MindMap,
}

impl SavedTemplate {
    /// The name shown on its card: the first line of its central idea.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The file it is stored in.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The map a new document starts from.
    #[must_use]
    pub const fn map(&self) -> &MindMap {
        &self.map
    }
}

/// What [`TemplateFolder::save`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    /// The template's name.
    pub name: String,
    /// Whether it replaced an existing template of the same name.
    pub replaced: bool,
}

/// The folder holding the user's templates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateFolder {
    dir: PathBuf,
}

impl TemplateFolder {
    /// The templates folder inside the app's data directory `data_dir` (created on the
    /// first save).
    #[must_use]
    pub fn in_dir(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join(FOLDER),
        }
    }

    /// Every template that can be read, sorted by name (ignoring case). Files that cannot
    /// be read, such as ones from a newer version, are left out.
    #[must_use]
    pub fn load(&self) -> Vec<SavedTemplate> {
        // No folder yet (nothing saved) or an unreadable one: no templates.
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut templates: Vec<SavedTemplate> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .filter_map(|path| {
                let (map, _) = storage::load_from_path(&path).ok()?;
                Some(SavedTemplate {
                    name: template_name(&map),
                    path,
                    map,
                })
            })
            .collect();
        // `sort_by_cached_key` lowercases each name once instead of on every comparison.
        templates.sort_by_cached_key(|template| template.name.to_lowercase());
        templates
    }

    /// Save `map` as a template named after its central idea, replacing a template of the
    /// same name.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` if the folder or file cannot be written.
    pub fn save(&self, map: &MindMap) -> Result<Saved, StorageError> {
        fs::create_dir_all(&self.dir)?;
        let name = template_name(map);
        let path = self.dir.join(file_name(&name));
        let replaced = path.exists();
        storage::save_to_path(map, &path)?;
        Ok(Saved { name, replaced })
    }

    /// Delete the template stored at `path`. A file that is already gone is not an error.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Io` if the file exists but cannot be deleted.
    pub fn remove(&self, path: &Path) -> Result<(), StorageError> {
        match fs::remove_file(path) {
            Err(err) if err.kind() != ErrorKind::NotFound => Err(err.into()),
            _ => Ok(()),
        }
    }
}

/// A template's name: the first line of its central idea.
fn template_name(map: &MindMap) -> String {
    let text = map.node(map.root()).map_or("", Node::text);
    let first_line = text.lines().next().unwrap_or_default().trim();
    if first_line.is_empty() {
        "Untitled".to_owned()
    } else {
        first_line.to_owned()
    }
}

/// A file name for template `name` that is valid on every platform: letters, digits,
/// spaces, `-` and `_` are kept, anything else becomes `_`.
fn file_name(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, ' ' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(MAX_STEM_CHARS)
        .collect();
    // Windows drops trailing spaces from file names.
    let stem = stem.trim();
    let stem = if stem.is_empty() { "template" } else { stem };
    if RESERVED_STEMS
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
    {
        format!("{stem}_.json")
    } else {
        format!("{stem}.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn saved_templates_load_sorted_and_same_names_replace() {
        let dir = TempDir::new("user_templates");
        let folder = TemplateFolder::in_dir(dir.path());
        assert_eq!(folder.load(), []);

        let mut weekly = MindMap::new("weekly review\nsecond line");
        let saved = folder.save(&weekly).expect("writable");
        assert_eq!(
            saved,
            Saved {
                name: "weekly review".to_owned(),
                replaced: false
            }
        );
        folder.save(&MindMap::new("Agenda")).expect("writable");
        weekly
            .add_child(weekly.root(), "Wins")
            .expect("root exists");
        assert!(folder.save(&weekly).expect("writable").replaced);

        let templates = folder.load();
        let names: Vec<&str> = templates.iter().map(SavedTemplate::name).collect();
        assert_eq!(names, ["Agenda", "weekly review"]);
        assert_eq!(templates[1].map().len(), 2);

        folder.remove(templates[0].path()).expect("deletable");
        folder
            .remove(templates[0].path())
            .expect("already gone is fine");
        assert_eq!(folder.load().len(), 1);
    }

    #[test]
    fn unreadable_files_are_skipped() {
        let dir = TempDir::new("user_templates_bad");
        let folder = TemplateFolder::in_dir(dir.path());
        folder.save(&MindMap::new("Good")).expect("writable");
        fs::write(dir.path().join(FOLDER).join("broken.json"), "{").expect("writable");
        fs::write(dir.path().join(FOLDER).join("notes.txt"), "hello").expect("writable");

        let names: Vec<String> = folder
            .load()
            .iter()
            .map(|template| template.name().to_owned())
            .collect();

        assert_eq!(names, ["Good"]);
    }

    #[test]
    fn file_names_are_safe_everywhere() {
        assert_eq!(file_name("Plan: Q3/Q4?"), "Plan_ Q3_Q4_.json");
        assert_eq!(file_name("  "), "template.json");
        assert_eq!(file_name("con"), "con_.json");
        assert_eq!(file_name("Ideen für März"), "Ideen für März.json");
        assert_eq!(
            file_name(&"x".repeat(200)).len(),
            MAX_STEM_CHARS + ".json".len()
        );
    }
}
