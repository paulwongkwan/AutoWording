use crate::ai::prompt::GeneratedMetadata;
use crate::config::Provider;
use crate::metadata::{ExistingMetadata, ExifTool, Metadata};
use eframe::egui;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Pending,
    Queued,
    Processing,
    Generated,
    Saved,
    Error,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Pending => "Pending",
            Status::Queued => "In queue",
            Status::Processing => "Processing",
            Status::Generated => "Generated",
            Status::Saved => "Saved",
            Status::Error => "Failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImageItem {
    pub path: PathBuf,
    pub status: Status,
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
    pub editorial: bool,
    pub dirty: bool,
    pub message: Option<String>,
    pub dims: Option<(u32, u32)>,
    pub gps: Option<String>,
    pub date: Option<String>,
    pub meta_loaded: bool,
    pub meta_error: bool,
}

impl ImageItem {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            status: Status::Pending,
            title: String::new(),
            caption: String::new(),
            keywords: Vec::new(),
            editorial: false,
            dirty: false,
            message: None,
            dims: None,
            gps: None,
            date: None,
            meta_loaded: false,
            meta_error: false,
        }
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    pub fn to_metadata(&self) -> Metadata {
        Metadata {
            title: self.title.trim().to_string(),
            caption: self.caption.trim().to_string(),
            keywords: self.keywords.clone(),
        }
    }

    pub fn has_metadata(&self) -> bool {
        !self.title.trim().is_empty()
    }

    #[allow(dead_code)]
    pub fn is_pending(&self) -> bool {
        matches!(self.status, Status::Pending | Status::Error)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BatchProgress {
    pub total: usize,
    pub done: usize,
    pub saving: bool,
}

impl BatchProgress {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.done as f32 / self.total as f32
        }
    }
}

/// Mouse modifiers for grid selection, kept independent from egui for testing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectModifiers {
    pub ctrl: bool,
    pub shift: bool,
}

/// Multi-image selection state for the ACDSee-style grid.
///
/// `selected` uses paths rather than indices so selection survives folder
/// reloads and status updates. `active` is the primary focus (the image shown
/// in the preview/viewer); `anchor` is the Shift-click anchor.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    selected: HashSet<PathBuf>,
    anchor: Option<PathBuf>,
    active: Option<PathBuf>,
}

impl Selection {
    pub fn contains(&self, path: &Path) -> bool {
        self.selected.contains(path)
    }

    pub fn count(&self) -> usize {
        self.selected.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn active(&self) -> Option<&Path> {
        self.active.as_deref()
    }

    #[allow(dead_code)]
    pub fn anchor(&self) -> Option<&Path> {
        self.anchor.as_deref()
    }

    pub fn clear(&mut self) {
        self.selected.clear();
        self.anchor = None;
        self.active = None;
    }

    pub fn select_only(&mut self, path: &Path) {
        self.selected.clear();
        self.selected.insert(path.to_path_buf());
        self.anchor = Some(path.to_path_buf());
        self.active = Some(path.to_path_buf());
    }

    pub fn select_all(&mut self, ordered: &[PathBuf]) {
        self.selected = ordered.iter().cloned().collect();
        self.anchor = ordered.first().cloned();
        if self
            .active
            .as_ref()
            .map(|active| !self.selected.contains(active))
            .unwrap_or(true)
        {
            self.active = ordered.first().cloned();
        }
    }

    pub fn retain_existing(&mut self, ordered: &[PathBuf]) {
        let existing: HashSet<&PathBuf> = ordered.iter().collect();
        self.selected.retain(|path| existing.contains(path));
        if self
            .anchor
            .as_ref()
            .map(|anchor| !existing.contains(anchor))
            .unwrap_or(true)
        {
            self.anchor = ordered.first().cloned();
        }
        if self
            .active
            .as_ref()
            .map(|active| !existing.contains(active))
            .unwrap_or(true)
        {
            self.active = self
                .first_selected_in_order(ordered);
        }
    }

    /// Apply a click from the grid, following ACDSee/Explorer conventions.
    pub fn click(&mut self, ordered: &[PathBuf], path: &Path, modifiers: SelectModifiers) {
        let path_buf = path.to_path_buf();

        if modifiers.shift {
            let anchor = self
                .anchor
                .clone()
                .or_else(|| self.active.clone())
                .or_else(|| ordered.first().cloned());
            let anchor_index = anchor
                .as_ref()
                .and_then(|anchor| ordered.iter().position(|item| item == anchor));
            let target_index = ordered.iter().position(|item| item == path);

            if let (Some(anchor_index), Some(target_index)) = (anchor_index, target_index) {
                if !modifiers.ctrl {
                    self.selected.clear();
                }
                let start = anchor_index.min(target_index);
                let end = anchor_index.max(target_index);
                for item in &ordered[start..=end] {
                    self.selected.insert(item.clone());
                }
            } else {
                self.selected.insert(path_buf.clone());
            }
            if self.anchor.is_none() {
                self.anchor = Some(path_buf.clone());
            }
            self.active = Some(path_buf);
            return;
        }

        if modifiers.ctrl {
            if self.selected.contains(&path_buf) {
                self.selected.remove(&path_buf);
                if self.active.as_deref() == Some(path) {
                    self.active = self.first_selected_in_order(ordered);
                }
            } else {
                self.selected.insert(path_buf.clone());
                self.active = Some(path_buf.clone());
            }
            self.anchor = Some(path_buf);
            return;
        }

        self.select_only(path);
    }

    /// Toggle a checkbox without clearing any other selection.
    pub fn toggle(&mut self, ordered: &[PathBuf], path: &Path) {
        let path_buf = path.to_path_buf();
        if self.selected.remove(&path_buf) {
            if self.active.as_deref() == Some(path) {
                self.active = self.first_selected_in_order(ordered);
            }
        } else {
            self.selected.insert(path_buf.clone());
            self.active = Some(path_buf.clone());
        }
        self.anchor = Some(path_buf);
    }

    /// The image that should be shown in the preview/viewer.
    pub fn primary(&self, ordered: &[PathBuf]) -> Option<PathBuf> {
        if let Some(active) = &self.active {
            if self.selected.contains(active) {
                return Some(active.clone());
            }
        }
        self.first_selected_in_order(ordered)
    }

    fn first_selected_in_order(&self, ordered: &[PathBuf]) -> Option<PathBuf> {
        ordered
            .iter()
            .find(|path| self.selected.contains(*path))
            .cloned()
    }
}

/// Messages sent from tokio tasks back to the UI thread.
pub enum WorkerEvent {
    Thumbnail {
        path: PathBuf,
        image: egui::ColorImage,
    },
    Preview {
        path: PathBuf,
        image: egui::ColorImage,
    },
    ImageFailed {
        path: PathBuf,
        error: String,
    },
    AiStarted {
        path: PathBuf,
    },
    AiDone {
        path: PathBuf,
        result: Result<GeneratedMetadata, String>,
    },
    BatchFinished {
        ok: usize,
        failed: usize,
    },
    SaveDone {
        path: PathBuf,
        result: Result<(), String>,
    },
    BatchSaved {
        ok: usize,
        failed: usize,
    },
    MetaLoaded {
        path: PathBuf,
        meta: ExistingMetadata,
    },
    MetaFailed {
        path: PathBuf,
    },
    ModelsLoaded {
        provider: Provider,
        endpoint: String,
        result: Result<Vec<String>, String>,
    },
    ExifToolReady {
        tool: Option<ExifTool>,
        status: String,
    },
}

pub fn is_jpeg(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "jpe" | "jfif")
    )
}

/// Expand dropped files/folders into a sorted list of JPEG paths.
pub fn collect_jpegs(path: &Path) -> Vec<PathBuf> {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut files = Vec::new();

    if absolute.is_file() {
        if is_jpeg(&absolute) {
            files.push(absolute);
        }
        return files;
    }

    if !absolute.is_dir() {
        return files;
    }

    for entry in walkdir::WalkDir::new(&absolute)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if entry.file_type().is_file() && is_jpeg(entry.path()) {
            files.push(entry.into_path());
        }
    }

    files
}

/// JPEGs directly inside a folder (no recursion), sorted case-insensitively.
/// This is what the ACDSee-style folder grid displays.
pub fn collect_folder_jpegs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_jpeg(path))
        .collect();

    files.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    });
    files
}

/// Clamp a selection step to the queue bounds.
pub fn stepped_index(selected: Option<usize>, len: usize, delta: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let current = match selected {
        Some(index) => index.min(len - 1) as isize,
        None => return Some(0),
    };
    Some(((current + delta).clamp(0, len as isize - 1)) as usize)
}

/// Selection index after removing `removed` from a queue of `remaining` items.
#[allow(dead_code)]
pub fn index_after_removal(
    selected: Option<usize>,
    removed: usize,
    remaining: usize,
) -> Option<usize> {
    if remaining == 0 {
        return None;
    }
    let index = match selected {
        Some(current) if current == removed => current,
        Some(current) if current > removed => current - 1,
        Some(current) => current,
        None => 0,
    };
    Some(index.min(remaining - 1))
}

/// Return the common value for a batch field, or `<Multiple values>`.
pub fn aggregate_common<I, S>(values: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut values = values.into_iter();
    let Some(first) = values.next() else {
        return String::new();
    };
    let first = first.as_ref().to_string();
    for value in values {
        if value.as_ref() != first {
            return "<Multiple values>".to_string();
        }
    }
    first
}

/// Append unique keywords, preserving order and enforcing the 50-word limit.
pub fn append_keywords(existing: &mut Vec<String>, additions: &[String]) {
    for keyword in additions {
        if existing.len() >= crate::ai::prompt::KEYWORD_TARGET {
            break;
        }
        let keyword = keyword.trim();
        if keyword.is_empty() {
            continue;
        }
        if !existing
            .iter()
            .any(|current| current.eq_ignore_ascii_case(keyword))
        {
            existing.push(keyword.to_string());
        }
    }
}

/// Decode a JPEG downscaled for display, returning the texture image and original size.
pub fn decode(path: &Path, max_dim: u32) -> anyhow::Result<(egui::ColorImage, (u32, u32))> {
    let image = image::open(path)?;
    let (width, height) = (image.width(), image.height());

    let image = if width.max(height) > max_dim {
        let scale = max_dim as f32 / width.max(height) as f32;
        image.resize(
            ((width as f32 * scale).round() as u32).max(1),
            ((height as f32 * scale).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };

    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok((
        egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        (width, height),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("autowording-state-tests")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write_jpeg(path: &Path, width: u32, height: u32) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("jpeg dir");
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([200, 100, 50]));
        image.save(path).expect("write jpeg");
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn recognises_jpeg_extensions_case_insensitively() {
        for name in ["a.jpg", "b.JPG", "c.jpeg", "d.JPEG", "e.jpe", "f.JpE", "g.jfif"] {
            assert!(is_jpeg(Path::new(name)), "{name} should be a JPEG");
        }
        for name in ["a.png", "b.gif", "c.txt", "noext", "d.jpg.txt", "e.tiff"] {
            assert!(!is_jpeg(Path::new(name)), "{name} should not be a JPEG");
        }
    }

    #[test]
    fn collect_jpegs_walks_directories_recursively() {
        let root = temp_dir("collect");
        std::fs::write(root.join("a.jpg"), b"jpeg").unwrap();
        std::fs::write(root.join("b.JPEG"), b"jpeg").unwrap();
        std::fs::write(root.join("c.png"), b"png").unwrap();
        std::fs::write(root.join("notes.txt"), b"text").unwrap();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested").join("d.JpE"), b"jpeg").unwrap();
        std::fs::write(root.join("nested").join("e.jpeg"), b"jpeg").unwrap();

        let found = collect_jpegs(&root);
        let names: HashSet<String> = found
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(found.len(), 4, "got: {names:?}");
        assert_eq!(
            names,
            HashSet::from([
                "a.jpg".to_string(),
                "b.JPEG".to_string(),
                "d.JpE".to_string(),
                "e.jpeg".to_string(),
            ])
        );
        assert!(found.iter().all(|path| path.is_absolute()));
    }

    #[test]
    fn collect_folder_jpegs_is_non_recursive_and_sorted() {
        let root = temp_dir("folder-only");
        std::fs::write(root.join("b.JPG"), b"jpeg").unwrap();
        std::fs::write(root.join("a.jpg"), b"jpeg").unwrap();
        std::fs::write(root.join("c.png"), b"png").unwrap();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested").join("d.jpg"), b"jpeg").unwrap();

        let found = collect_folder_jpegs(&root);
        let names: Vec<String> = found
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.jpg", "b.JPG"]);
        assert!(collect_folder_jpegs(&root.join("missing")).is_empty());
    }

    #[test]
    fn collect_jpegs_handles_single_files_and_missing_paths() {
        let root = temp_dir("single");
        let jpeg = root.join("one.jpg");
        std::fs::write(&jpeg, b"jpeg").unwrap();

        let found = collect_jpegs(&jpeg);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name().unwrap(), "one.jpg");

        let png = root.join("one.png");
        std::fs::write(&png, b"png").unwrap();
        assert!(collect_jpegs(&png).is_empty());
        assert!(collect_jpegs(&root.join("missing")).is_empty());
        assert!(collect_jpegs(&root.join("missing.jpg")).is_empty());
    }

    #[test]
    fn decode_downscales_and_reports_original_dimensions() {
        let root = temp_dir("decode");
        let path = root.join("large.jpg");
        write_jpeg(&path, 2000, 1000);

        let (image, dims) = decode(&path, 500).expect("decode");
        assert_eq!(dims, (2000, 1000));
        assert_eq!(image.size, [500, 250]);
        assert_eq!(image.pixels.len(), 500 * 250);
    }

    #[test]
    fn decode_keeps_images_below_the_limit() {
        let root = temp_dir("decode-small");
        let path = root.join("small.jpg");
        write_jpeg(&path, 120, 80);

        let (image, dims) = decode(&path, 500).expect("decode");
        assert_eq!(dims, (120, 80));
        assert_eq!(image.size, [120, 80]);
    }

    #[test]
    fn decode_reports_corrupt_and_missing_files() {
        let root = temp_dir("decode-bad");
        let path = root.join("broken.jpg");
        std::fs::write(&path, b"not an image").unwrap();
        assert!(decode(&path, 500).is_err());
        assert!(decode(&root.join("missing.jpg"), 500).is_err());
    }

    #[test]
    fn image_item_tracks_state_and_converts_to_metadata() {
        let mut item = ImageItem::new(PathBuf::from(r"C:\photos\Sunset Over Bay.jpg"));
        assert_eq!(item.file_name(), "Sunset Over Bay.jpg");
        assert_eq!(item.status, Status::Pending);
        assert!(!item.has_metadata());
        assert!(item.is_pending());

        item.title = "  Sunset Over Bay  ".into();
        item.caption = "  Fog over the water.  ".into();
        item.keywords = vec!["sunset".into(), "bay".into()];
        let metadata = item.to_metadata();
        assert_eq!(metadata.title, "Sunset Over Bay");
        assert_eq!(metadata.caption, "Fog over the water.");
        assert_eq!(metadata.keywords, vec!["sunset", "bay"]);
        assert!(item.has_metadata());

        item.title = "   ".into();
        assert!(!item.has_metadata());

        item.status = Status::Error;
        assert!(item.is_pending());
        for status in [Status::Queued, Status::Processing, Status::Generated, Status::Saved] {
            item.status = status;
            assert!(!item.is_pending());
        }
    }

    #[test]
    fn batch_progress_fraction_handles_bounds() {
        assert_eq!(BatchProgress { total: 0, done: 0, saving: false }.fraction(), 0.0);
        assert_eq!(BatchProgress { total: 4, done: 0, saving: false }.fraction(), 0.0);
        assert_eq!(BatchProgress { total: 4, done: 2, saving: false }.fraction(), 0.5);
        assert_eq!(BatchProgress { total: 4, done: 4, saving: true }.fraction(), 1.0);
    }

    #[test]
    fn selection_steps_clamp_to_queue_bounds() {
        assert_eq!(stepped_index(None, 0, 1), None);
        assert_eq!(stepped_index(Some(0), 0, -1), None);
        assert_eq!(stepped_index(None, 3, 1), Some(0));
        assert_eq!(stepped_index(None, 3, -1), Some(0));
        assert_eq!(stepped_index(Some(0), 3, 1), Some(1));
        assert_eq!(stepped_index(Some(1), 3, 1), Some(2));
        assert_eq!(stepped_index(Some(2), 3, 1), Some(2));
        assert_eq!(stepped_index(Some(1), 3, -1), Some(0));
        assert_eq!(stepped_index(Some(2), 3, -10), Some(0));
        assert_eq!(stepped_index(Some(99), 3, 1), Some(2), "out-of-range selection is clamped");
    }

    #[test]
    fn selection_after_removal_keeps_a_valid_neighbour() {
        assert_eq!(index_after_removal(Some(0), 0, 0), None);
        assert_eq!(index_after_removal(None, 0, 0), None);

        assert_eq!(index_after_removal(Some(0), 0, 3), Some(0));
        assert_eq!(index_after_removal(Some(1), 1, 3), Some(1));
        assert_eq!(index_after_removal(Some(3), 3, 3), Some(2));
        assert_eq!(index_after_removal(Some(2), 0, 3), Some(1));
        assert_eq!(index_after_removal(Some(0), 2, 3), Some(0));
        assert_eq!(index_after_removal(Some(3), 2, 3), Some(2));
        assert_eq!(index_after_removal(None, 1, 3), Some(0));
        assert_eq!(index_after_removal(Some(9), 0, 3), Some(2));
    }

    #[test]
    fn plain_click_selects_one_and_sets_focus() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg"]);
        let mut selection = Selection::default();
        selection.click(&ordered, Path::new("b.jpg"), SelectModifiers::default());
        assert_eq!(selection.count(), 1);
        assert!(selection.contains(Path::new("b.jpg")));
        assert_eq!(selection.active(), Some(Path::new("b.jpg")));
        assert_eq!(selection.anchor(), Some(Path::new("b.jpg")));
        assert_eq!(selection.primary(&ordered).as_deref(), Some(Path::new("b.jpg")));
    }

    #[test]
    fn ctrl_click_toggles_without_clearing() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg"]);
        let mut selection = Selection::default();
        selection.click(&ordered, Path::new("a.jpg"), SelectModifiers::default());
        selection.click(
            &ordered,
            Path::new("c.jpg"),
            SelectModifiers { ctrl: true, shift: false },
        );
        assert_eq!(selection.count(), 2);
        assert!(selection.contains(Path::new("a.jpg")));
        assert!(selection.contains(Path::new("c.jpg")));
        assert_eq!(selection.active(), Some(Path::new("c.jpg")));

        selection.click(
            &ordered,
            Path::new("c.jpg"),
            SelectModifiers { ctrl: true, shift: false },
        );
        assert_eq!(selection.count(), 1);
        assert!(!selection.contains(Path::new("c.jpg")));
        assert_eq!(selection.active(), Some(Path::new("a.jpg")));
    }

    #[test]
    fn shift_click_selects_contiguous_range_from_anchor() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg", "d.jpg"]);
        let mut selection = Selection::default();
        selection.click(&ordered, Path::new("b.jpg"), SelectModifiers::default());
        selection.click(
            &ordered,
            Path::new("d.jpg"),
            SelectModifiers { ctrl: false, shift: true },
        );
        assert_eq!(selection.count(), 3);
        assert!(selection.contains(Path::new("b.jpg")));
        assert!(selection.contains(Path::new("c.jpg")));
        assert!(selection.contains(Path::new("d.jpg")));
        assert_eq!(selection.active(), Some(Path::new("d.jpg")));
        assert_eq!(selection.anchor(), Some(Path::new("b.jpg")));
    }

    #[test]
    fn ctrl_shift_click_extends_the_existing_selection() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg", "d.jpg"]);
        let mut selection = Selection::default();
        selection.click(&ordered, Path::new("a.jpg"), SelectModifiers::default());
        selection.click(
            &ordered,
            Path::new("b.jpg"),
            SelectModifiers { ctrl: false, shift: true },
        );
        assert_eq!(selection.count(), 2);
        selection.click(
            &ordered,
            Path::new("d.jpg"),
            SelectModifiers { ctrl: true, shift: true },
        );
        assert_eq!(selection.count(), 4);
        assert!(selection.contains(Path::new("a.jpg")));
        assert!(selection.contains(Path::new("d.jpg")));
    }

    #[test]
    fn checkbox_toggle_keeps_other_items_selected() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg"]);
        let mut selection = Selection::default();
        selection.click(&ordered, Path::new("a.jpg"), SelectModifiers::default());
        selection.toggle(&ordered, Path::new("c.jpg"));
        assert_eq!(selection.count(), 2);
        assert_eq!(selection.active(), Some(Path::new("c.jpg")));

        selection.toggle(&ordered, Path::new("c.jpg"));
        assert_eq!(selection.count(), 1);
        assert_eq!(selection.active(), Some(Path::new("a.jpg")));
    }

    #[test]
    fn select_all_retain_and_primary_handle_folder_changes() {
        let ordered = paths(&["a.jpg", "b.jpg", "c.jpg"]);
        let mut selection = Selection::default();
        selection.select_all(&ordered);
        assert_eq!(selection.count(), 3);
        assert_eq!(selection.active(), Some(Path::new("a.jpg")));
        assert_eq!(selection.primary(&ordered).as_deref(), Some(Path::new("a.jpg")));

        selection.click(
            &ordered,
            Path::new("c.jpg"),
            SelectModifiers { ctrl: true, shift: false },
        );
        selection.retain_existing(&paths(&["b.jpg", "x.jpg"]));
        assert_eq!(selection.count(), 1);
        assert!(selection.contains(Path::new("b.jpg")));
        assert_eq!(selection.active(), Some(Path::new("b.jpg")));

        selection.retain_existing(&paths(&["x.jpg"]));
        assert!(selection.is_empty());
        assert_eq!(selection.active(), None);
    }

    #[test]
    fn aggregate_common_reports_mixed_values() {
        assert_eq!(aggregate_common(std::iter::empty::<&str>()), "");
        assert_eq!(aggregate_common(["Same", "Same"]), "Same");
        assert_eq!(aggregate_common(["Same", "Different"]), "<Multiple values>");
        assert_eq!(aggregate_common([""]), "");
    }

    #[test]
    fn append_keywords_deduplicates_and_caps() {
        let mut existing = vec!["sunset".to_string(), "Bay".to_string()];
        append_keywords(
            &mut existing,
            &[
                "bay".to_string(),
                "fog".to_string(),
                "  ".to_string(),
                "FOG".to_string(),
            ],
        );
        assert_eq!(existing, vec!["sunset", "Bay", "fog"]);

        let mut full: Vec<String> = (0..50).map(|index| format!("kw{index}")).collect();
        append_keywords(&mut full, &["extra".to_string()]);
        assert_eq!(full.len(), 50);
    }
}
