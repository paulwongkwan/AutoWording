//! Unsaved-edit journal.
//!
//! Inspector edits live in memory until the user saves. This journal mirrors
//! every dirty item to `%APPDATA%\AutoWording\config\edit-journal.json` (debounced
//! by the app), so a crash or an accidental close does not lose manual changes.
//! Entries are dropped as soon as their file is saved or restored.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
    pub editorial: bool,
}

#[derive(Serialize, Deserialize)]
struct JournalFile {
    version: u32,
    entries: Vec<StoredEntry>,
}

#[derive(Serialize, Deserialize)]
struct StoredEntry {
    path: String,
    title: String,
    caption: String,
    keywords: Vec<String>,
    editorial: bool,
}

/// Journal next to `config.toml`.
pub fn default_path() -> PathBuf {
    crate::config::Config::config_path().with_file_name("edit-journal.json")
}

pub fn load() -> HashMap<PathBuf, JournalEntry> {
    load_from(&default_path())
}

pub fn save(entries: &HashMap<PathBuf, JournalEntry>) -> anyhow::Result<()> {
    save_to(entries, &default_path())
}

fn load_from(path: &Path) -> HashMap<PathBuf, JournalEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let parsed: JournalFile = match serde_json::from_str(&text) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!(
                "Ignoring unreadable edit journal at {}: {error}",
                path.display()
            );
            return HashMap::new();
        }
    };
    if parsed.version != JOURNAL_VERSION {
        return HashMap::new();
    }
    parsed
        .entries
        .into_iter()
        .filter_map(|stored| {
            let path = PathBuf::from(stored.path);
            // Entries for deleted files cannot be recovered; drop them instead
            // of carrying stale data forward.
            if !path.is_file() {
                return None;
            }
            Some((
                path,
                JournalEntry {
                    title: stored.title,
                    caption: stored.caption,
                    keywords: stored.keywords,
                    editorial: stored.editorial,
                },
            ))
        })
        .collect()
}

fn save_to(entries: &HashMap<PathBuf, JournalEntry>, path: &Path) -> anyhow::Result<()> {
    if entries.is_empty() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    let mut sorted: Vec<(&PathBuf, &JournalEntry)> = entries.iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(right.0));
    let file = JournalFile {
        version: JOURNAL_VERSION,
        entries: sorted
            .into_iter()
            .map(|(path, entry)| StoredEntry {
                path: path.to_string_lossy().to_string(),
                title: entry.title.clone(),
                caption: entry.caption.clone(),
                keywords: entry.keywords.clone(),
                editorial: entry.editorial,
            })
            .collect(),
    };
    let text = serde_json::to_string_pretty(&file)?;
    crate::persist::write_atomic(path, text.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("autowording-journal-tests")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn entry(title: &str) -> JournalEntry {
        JournalEntry {
            title: title.to_string(),
            caption: "A caption that was never saved.".to_string(),
            keywords: vec!["harbour".to_string(), "dawn".to_string()],
            editorial: true,
        }
    }

    #[test]
    fn journal_round_trips_dirty_entries() {
        let dir = temp_dir("roundtrip");
        let image = dir.join("photo.jpg");
        std::fs::write(&image, b"jpeg").expect("image");
        let path = dir.join("journal.json");

        let mut entries = HashMap::new();
        entries.insert(image.clone(), entry("Unsaved title"));

        save_to(&entries, &path).expect("save journal");
        assert!(path.is_file());
        assert_eq!(load_from(&path), entries);
    }

    #[test]
    fn empty_journal_removes_the_file() {
        let dir = temp_dir("empty");
        let path = dir.join("journal.json");
        std::fs::write(&path, b"{}").expect("seed journal");

        save_to(&HashMap::new(), &path).expect("save empty journal");
        assert!(!path.exists());
        assert!(load_from(&path).is_empty());
    }

    #[test]
    fn unreadable_journal_is_ignored() {
        let dir = temp_dir("corrupt");
        let path = dir.join("journal.json");
        std::fs::write(&path, b"not json").expect("seed corrupt journal");
        assert!(load_from(&path).is_empty());
    }

    #[test]
    fn entries_for_missing_files_are_dropped_on_load() {
        let dir = temp_dir("missing");
        let image = dir.join("gone.jpg");
        std::fs::write(&image, b"jpeg").expect("image");
        let path = dir.join("journal.json");

        let mut entries = HashMap::new();
        entries.insert(image.clone(), entry("Unsaved title"));
        save_to(&entries, &path).expect("save journal");

        std::fs::remove_file(&image).expect("delete image");
        assert!(load_from(&path).is_empty());
    }
}
