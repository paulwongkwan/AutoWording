use crate::ai::prompt::GeneratedMetadata;
use crate::metadata::{ExistingMetadata, ExifTool, Metadata};
use eframe::egui;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Pending,
    Processing,
    Generated,
    Saved,
    Error,
}

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
    use std::collections::HashSet;
    use super::*;

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
        for status in [Status::Processing, Status::Generated, Status::Saved] {
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
}
