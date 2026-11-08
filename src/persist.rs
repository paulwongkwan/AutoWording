use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write a small state file without truncating the previous version first.
/// The temporary file is renamed over the destination only after it is fully
/// written and flushed, so a failed write leaves the old file intact.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let temp = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("autowording-persist-tests")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn replaces_a_file_without_leaving_a_temp_file() {
        let dir = temp_dir("replace");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("config.toml");
        std::fs::write(&path, b"old").expect("seed file");

        write_atomic(&path, b"new").expect("atomic write");
        assert_eq!(std::fs::read(&path).expect("read file"), b"new");

        let leftovers = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn creates_a_missing_parent_directory() {
        let dir = temp_dir("parent");
        let path = dir.join("nested").join("state.json");

        write_atomic(&path, b"{}").expect("atomic write");
        assert_eq!(std::fs::read(&path).expect("read file"), b"{}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
