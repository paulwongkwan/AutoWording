pub mod tags;
pub mod writer;

pub use tags::ExistingMetadata;
pub use writer::{Metadata, Rights};

use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

/// Windows: run exiftool without flashing a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone)]
pub struct ExifTool {
    program: PathBuf,
    pub version: String,
}

impl ExifTool {
    /// Locate exiftool: explicit path, bundled resources, then PATH.
    pub fn detect(preferred: &str) -> Option<Self> {
        for candidate in candidates(preferred) {
            if let Some(version) = version_of(&candidate) {
                return Some(Self {
                    program: candidate,
                    version,
                });
            }
        }
        None
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    fn command(&self) -> Command {
        command_for(&self.program)
    }

    /// Run exiftool with arguments passed through a UTF-8 argfile. This avoids
    /// the Windows code-page conversion that mangles non-ASCII tag values.
    fn run(&self, args: Vec<String>) -> Result<Output> {
        let argfile = write_argfile(&args)?;
        let mut command = self.command();
        command
            .arg("-charset")
            .arg("filename=utf8")
            .arg("-@")
            .arg(&argfile);

        let output = command.output();
        let _ = std::fs::remove_file(&argfile);

        output.with_context(|| format!("Could not run exiftool at {}", self.program.display()))
    }

    /// Read title, caption, keywords, GPS location, capture date and dimensions.
    /// Video files use group-qualified XMP/QuickTime Keys tags; images keep the
    /// existing XMP-first, IPTC/EXIF-fallback read.
    pub fn read(&self, path: &Path) -> Result<ExistingMetadata> {
        let video = is_video_path(path);
        let mut args: Vec<String> = vec![
            "-j".to_string(),
            "-charset".to_string(),
            "iptc=utf8".to_string(),
        ];
        if video {
            args.push("-G1".to_string());
        }
        let read_tags = if video {
            tags::VIDEO_READ_TAGS
        } else {
            tags::READ_TAGS
        };
        args.extend(read_tags.iter().map(|tag| tag.to_string()));
        args.push(path.to_string_lossy().to_string());

        let output = self.run(args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "exiftool could not read {}: {}",
                path.display(),
                stderr.trim()
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed = if video {
            tags::parse_video_read_json(&stdout)
        } else {
            // ExifTool normally reports Orientation from EXIF or XMP. When it
            // does not, read the JPEG's own tags so an XMP-only orientation
            // still swaps the reported dimensions.
            let fallback = if tags::has_orientation(&stdout) {
                None
            } else {
                std::fs::read(path)
                    .ok()
                    .and_then(|bytes| crate::orientation::from_jpeg(&bytes))
            };
            match fallback {
                Some(orientation) => {
                    tags::parse_read_json_with_orientation(&stdout, Some(orientation))
                }
                None => tags::parse_read_json(&stdout),
            }
        };
        parsed.with_context(|| format!("Could not parse metadata for {}", path.display()))
    }

    /// Read metadata for a whole folder, splitting video and image files so
    /// each gets the correct tag groups and parser.
    pub fn read_many(&self, paths: &[PathBuf]) -> Result<HashMap<PathBuf, ExistingMetadata>> {
        if paths.is_empty() {
            return Ok(HashMap::new());
        }

        let (videos, images): (Vec<PathBuf>, Vec<PathBuf>) =
            paths.iter().cloned().partition(|path| is_video_path(path));
        let mut metadata = HashMap::new();
        if !images.is_empty() {
            metadata.extend(self.read_many_group(&images, tags::READ_TAGS, false)?);
        }
        if !videos.is_empty() {
            metadata.extend(self.read_many_group(&videos, tags::VIDEO_READ_TAGS, true)?);
        }
        Ok(metadata)
    }

    fn read_many_group(
        &self,
        paths: &[PathBuf],
        read_tags: &[&str],
        grouped: bool,
    ) -> Result<HashMap<PathBuf, ExistingMetadata>> {
        let mut args: Vec<String> = vec![
            "-j".to_string(),
            "-charset".to_string(),
            "iptc=utf8".to_string(),
        ];
        if grouped {
            args.push("-G1".to_string());
        }
        args.extend(read_tags.iter().map(|tag| tag.to_string()));
        args.extend(paths.iter().map(|path| path.to_string_lossy().to_string()));

        let output = self.run(args)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let value: serde_json::Value = serde_json::from_str(&stdout).with_context(|| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!("Could not parse exiftool metadata JSON: {stderr}")
        })?;
        let Some(objects) = value.as_array() else {
            bail!("exiftool returned non-array metadata JSON");
        };

        let mut requested: HashMap<String, PathBuf> = paths
            .iter()
            .cloned()
            .map(|path| (path_key(&path), path))
            .collect();
        let mut metadata = HashMap::new();
        for object in objects {
            let Some(source) = object.get("SourceFile").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(path) = requested.remove(&path_key(Path::new(source))) else {
                continue;
            };
            if grouped {
                metadata.insert(path, tags::parse_video_read_object(object));
                continue;
            }
            let fallback = if tags::has_orientation_in_object(object) {
                None
            } else {
                std::fs::read(&path)
                    .ok()
                    .and_then(|bytes| crate::orientation::from_jpeg(&bytes))
            };
            metadata.insert(path, tags::parse_read_object(object, fallback));
        }
        Ok(metadata)
    }

    /// Write title, caption, keywords and category into EXIF / IPTC / XMP for
    /// images, or XMP / QuickTime Keys for MP4/MOV video.
    pub fn write(&self, path: &Path, md: &Metadata) -> Result<()> {
        let args = if is_video_path(path) {
            writer::video_write_args(md, &Rights::default(), false)
        } else {
            writer::write_args(md)
        };
        self.write_with_args(path, args)
    }

    /// Write generated metadata plus the rights fields configured in Settings.
    pub fn write_with_rights(&self, path: &Path, md: &Metadata, rights: &Rights) -> Result<()> {
        self.write_with_rights_and_clear(path, md, rights, false)
    }

    /// Write metadata, the resolved rights fields, and optionally remove both
    /// keyword tags when the user explicitly cleared the keyword list.
    pub fn write_with_rights_and_clear(
        &self,
        path: &Path,
        md: &Metadata,
        rights: &Rights,
        clear_keywords: bool,
    ) -> Result<()> {
        let args = if is_video_path(path) {
            writer::video_write_args(md, rights, clear_keywords)
        } else {
            let mut args = writer::write_args(md);
            args.extend(writer::rights_args(rights));
            if clear_keywords {
                args.extend(writer::clear_keyword_args());
            }
            args
        };
        self.write_with_args(path, args)
    }

    fn write_with_args(&self, path: &Path, mut args: Vec<String>) -> Result<()> {
        if !path.is_file() {
            bail!("File not found: {}", path.display());
        }

        args.push(path.to_string_lossy().to_string());

        let output = self.run(args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            bail!("exiftool failed for {}: {}", path.display(), detail);
        }

        Ok(())
    }
}

pub const BACKUP_DIR: &str = ".autowording-backups";

/// Where the pre-write copy of a JPEG lives (see [`backup_original`]).
pub fn backup_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().unwrap_or_default();
    parent.join(BACKUP_DIR).join(name)
}

/// Copy the JPEG into a sibling backup directory. Repeated saves keep the
/// first backup, so the pre-AutoWording file remains recoverable.
pub fn backup_original(path: &Path) -> Result<PathBuf> {
    let backup = backup_path(path);
    let backup_dir = backup.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(backup_dir)
        .with_context(|| format!("Could not create {}", backup_dir.display()))?;
    if !backup.exists() {
        std::fs::copy(path, &backup).with_context(|| {
            format!(
                "Could not back up {} to {}",
                path.display(),
                backup.display()
            )
        })?;
    }
    Ok(backup)
}

/// Refuse to overwrite a file that changed on disk after it was loaded. When
/// no baseline was captured (for example a background batch that never read
/// metadata), the write is allowed.
pub fn check_unchanged(
    path: &Path,
    expected_len: Option<u64>,
    expected_modified: Option<std::time::SystemTime>,
) -> Result<()> {
    let (Some(expected_len), Some(expected_modified)) = (expected_len, expected_modified) else {
        return Ok(());
    };
    let metadata =
        std::fs::metadata(path).with_context(|| format!("Could not inspect {}", path.display()))?;
    if metadata.len() != expected_len || metadata.modified().ok() != Some(expected_modified) {
        bail!(
            "{} changed on disk since it was loaded; not overwritten. Refresh and review it.",
            path.display()
        );
    }
    Ok(())
}

/// Verify the values ExifTool reads back after a write. Caption verification
/// accounts for the writer's title fallback when the caption is empty.
pub fn verify_written(md: &Metadata, read: &ExistingMetadata) -> Result<()> {
    let title = md.title.trim();
    let read_title = read
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .with_context(|| "Saved title could not be read back")?;
    if read_title != title {
        bail!(
            "Saved title did not verify: expected {:?}, found {:?}",
            title,
            read_title
        );
    }

    let expected_caption = if md.caption.trim().is_empty() {
        title
    } else {
        md.caption.trim()
    };
    let read_caption = read
        .caption
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .with_context(|| "Saved caption could not be read back")?;
    if read_caption != expected_caption {
        bail!(
            "Saved caption did not verify: expected {:?}, found {:?}",
            expected_caption,
            read_caption
        );
    }

    let expected_keywords: HashSet<String> = md
        .keywords
        .iter()
        .map(|keyword| keyword.trim().to_lowercase())
        .filter(|keyword| !keyword.is_empty())
        .collect();
    let read_keywords: HashSet<String> = read
        .keywords
        .iter()
        .map(|keyword| keyword.trim().to_lowercase())
        .filter(|keyword| !keyword.is_empty())
        .collect();
    // The writer leaves existing keywords in place when the list is empty, so
    // there is nothing to compare in that case.
    if !expected_keywords.is_empty() && read_keywords != expected_keywords {
        bail!(
            "Saved keywords did not verify: expected {} keyword(s), found {}",
            expected_keywords.len(),
            read_keywords.len()
        );
    }

    let expected_category = md.category.trim();
    if !expected_category.is_empty() {
        let read_category = read
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .with_context(|| "Saved category could not be read back")?;
        if read_category != expected_category {
            bail!(
                "Saved category did not verify: expected {:?}, found {:?}",
                expected_category,
                read_category
            );
        }
    }

    Ok(())
}

/// Verify the configured rights fields that ExifTool reads back after a write.
/// Blank settings are not compared because they are not written.
pub fn verify_rights(rights: &Rights, read: &ExistingMetadata) -> Result<()> {
    let checks = [
        ("Creator", rights.creator.trim(), read.creator.as_deref()),
        (
            "Copyright Notice",
            rights.copyright.trim(),
            read.copyright.as_deref(),
        ),
        (
            "Credit Line",
            rights.credit_line.trim(),
            read.credit_line.as_deref(),
        ),
        (
            "Rights Usage Terms",
            rights.usage_terms.trim(),
            read.usage_terms.as_deref(),
        ),
    ];
    for (label, expected, found) in checks {
        if expected.is_empty() {
            continue;
        }
        let found = found
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .with_context(|| format!("Saved {label} could not be read back"))?;
        if found != expected {
            bail!(
                "Saved {label} did not verify: expected {:?}, found {:?}",
                expected,
                found
            );
        }
    }
    Ok(())
}

fn is_video_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("mp4" | "mov")
    )
}

fn command_for(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn path_key(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

fn argfile_text(args: &[String]) -> String {
    let mut text = String::new();
    for arg in args {
        // One argument per line: newlines inside values would start a new
        // argument, so they are flattened to spaces.
        for ch in arg.chars() {
            text.push(if ch == '\n' || ch == '\r' { ' ' } else { ch });
        }
        text.push('\n');
    }
    text
}

fn write_argfile(args: &[String]) -> Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "autowording-exif-{}-{}.args",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, argfile_text(args))
        .with_context(|| format!("Could not write exiftool argument file {}", path.display()))?;
    Ok(path)
}

fn version_of(program: &Path) -> Option<String> {
    let output = command_for(program).arg("-ver").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!version.is_empty()).then_some(version)
}

fn candidates(preferred: &str) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = Vec::new();

    let preferred = preferred.trim().trim_matches('"');
    if !preferred.is_empty() {
        let path = PathBuf::from(preferred);
        if path.is_dir() {
            list.push(path.join("exiftool.exe"));
            list.push(path.join("exiftool"));
        } else {
            list.push(path);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            list.push(dir.join("exiftool.exe"));
            list.push(dir.join("resources").join("exiftool").join("exiftool.exe"));
            list.push(dir.join("resources").join("exiftool.exe"));
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        list.push(cwd.join("resources").join("exiftool").join("exiftool.exe"));
        list.push(cwd.join("resources").join("exiftool.exe"));
    }

    list.push(PathBuf::from("exiftool"));
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a real JPEG and verifies title, caption and keywords round-trip
    /// through exiftool into EXIF / IPTC / XMP.
    #[test]
    fn exiftool_round_trip() {
        let Some(tool) = ExifTool::detect("") else {
            eprintln!("exiftool not available; skipping round-trip test");
            return;
        };

        let dir = std::env::temp_dir().join("autowording-tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("roundtrip.jpg");

        let image = image::RgbImage::from_pixel(64, 48, image::Rgb([120, 80, 40]));
        image.save(&path).expect("write test jpeg");

        let metadata = Metadata {
            title: "Golden Gate Sunrise".into(),
            caption: "Fog drifts under the Golden Gate Bridge at sunrise in San Francisco.".into(),
            keywords: vec!["bridge".into(), "sunrise".into(), "fog".into()],
            category: "Nature".into(),
        };

        tool.write(&path, &metadata).expect("metadata write");
        let read = tool.read(&path).expect("metadata read");
        verify_written(&metadata, &read).expect("metadata verification");

        assert_eq!(read.title.as_deref(), Some("Golden Gate Sunrise"));
        assert_eq!(
            read.caption.as_deref(),
            Some("Fog drifts under the Golden Gate Bridge at sunrise in San Francisco.")
        );
        assert!(read.keywords.iter().any(|keyword| keyword == "bridge"));
        assert!(read.keywords.iter().any(|keyword| keyword == "sunrise"));
        assert!(read.keywords.iter().any(|keyword| keyword == "fog"));
        assert_eq!(read.category.as_deref(), Some("Nature"));
        assert_eq!(read.dims, Some((64, 48)));
    }

    fn temp_jpeg(name: &str, width: u32, height: u32) -> PathBuf {
        let dir = std::env::temp_dir().join("autowording-tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(format!("{}-{name}.jpg", std::process::id()));
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([120, 80, 40]));
        image.save(&path).expect("write test jpeg");
        path
    }

    fn require_tool() -> Option<ExifTool> {
        let tool = ExifTool::detect("");
        if tool.is_none() {
            eprintln!("exiftool not available; skipping test");
        }
        tool
    }

    fn run_exiftool(tool: &ExifTool, args: &[&str]) {
        let output = command_for(tool.program())
            .args(args)
            .output()
            .expect("run exiftool");
        assert!(
            output.status.success(),
            "exiftool {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn xmp_only_orientation_drives_dimensions_and_rotation() {
        let Some(tool) = require_tool() else { return };
        let path = temp_jpeg("xmp-orientation", 40, 20);
        let path_arg = path.to_string_lossy().to_string();

        // Strip every tag, including EXIF, then write only the XMP orientation.
        run_exiftool(&tool, &["-all=", "-overwrite_original", &path_arg]);
        run_exiftool(
            &tool,
            &["-XMP-tiff:Orientation#=6", "-overwrite_original", &path_arg],
        );

        let bytes = std::fs::read(&path).expect("read jpeg");
        assert_eq!(
            crate::orientation::from_jpeg(&bytes),
            Some(6),
            "an XMP-only orientation should still be readable"
        );

        let read = tool.read(&path).expect("read xmp-only orientation");
        assert_eq!(
            read.dims,
            Some((20, 40)),
            "orientation 6 should swap the stored 40x20 dimensions"
        );
    }

    #[test]
    fn external_change_guard_detects_modified_files() {
        let path = temp_jpeg("guard.jpg", 8, 8);
        let metadata = std::fs::metadata(&path).expect("stat file");
        let len = metadata.len();
        let modified = metadata.modified().expect("mtime");

        check_unchanged(&path, Some(len), Some(modified)).expect("unchanged file is writable");

        std::fs::write(&path, b"a different, longer payload").expect("modify file");
        let error = check_unchanged(&path, Some(len), Some(modified))
            .expect_err("changed file must be refused")
            .to_string();
        assert!(error.contains("changed on disk"), "got: {error}");

        check_unchanged(&path, None, None).expect("unknown baseline allows the write");
    }

    #[test]
    fn backup_original_keeps_the_first_copy() {
        let path = temp_jpeg("backup", 16, 16);
        let original = std::fs::read(&path).expect("read original");
        let backup = backup_original(&path).expect("first backup");
        assert_eq!(std::fs::read(&backup).expect("read backup"), original);

        std::fs::write(&path, b"changed after backup").expect("change original");
        let backup_again = backup_original(&path).expect("second backup");
        assert_eq!(backup_again, backup);
        assert_eq!(
            std::fs::read(&backup_again).expect("read backup again"),
            original
        );
    }

    #[test]
    fn rights_fields_round_trip_and_verify() {
        let Some(tool) = require_tool() else { return };
        let path = temp_jpeg("rights", 32, 24);

        let metadata = Metadata {
            title: "Rights Round Trip".into(),
            caption: String::new(),
            keywords: vec!["rights".into()],
            category: String::new(),
        };
        let rights = Rights {
            creator: "Jane Doe".into(),
            copyright: "© 2026 Jane Doe".into(),
            credit_line: "Jane Doe".into(),
            usage_terms: "Commercial use only".into(),
        };
        tool.write_with_rights(&path, &metadata, &rights)
            .expect("write rights");

        let read = tool.read(&path).expect("read rights");
        verify_rights(&rights, &read).expect("rights verification");
        assert_eq!(read.creator.as_deref(), Some("Jane Doe"));
        assert_eq!(read.copyright.as_deref(), Some("© 2026 Jane Doe"));
        assert_eq!(read.credit_line.as_deref(), Some("Jane Doe"));
        assert_eq!(read.usage_terms.as_deref(), Some("Commercial use only"));
    }

    #[test]
    fn clearing_keywords_removes_both_tag_lists() {
        let Some(tool) = require_tool() else { return };
        let path = temp_jpeg("clear-keywords", 32, 24);
        let metadata = Metadata {
            title: "Clear Keywords".into(),
            caption: "A caption for the clearing test.".into(),
            keywords: vec!["alpha".into(), "beta".into()],
            category: "Nature".into(),
        };
        tool.write_with_rights(&path, &metadata, &Rights::default())
            .expect("seed keywords");
        assert!(!tool.read(&path).expect("read seeded").keywords.is_empty());

        let cleared = Metadata {
            keywords: Vec::new(),
            ..metadata
        };
        tool.write_with_rights_and_clear(&path, &cleared, &Rights::default(), true)
            .expect("clear keywords");
        let read = tool.read(&path).expect("read cleared");
        assert!(read.keywords.is_empty(), "got {:?}", read.keywords);
    }

    #[test]
    fn png_round_trip_writes_xmp_metadata() {
        let Some(tool) = require_tool() else { return };
        let dir = std::env::temp_dir().join("autowording-tests");
        std::fs::create_dir_all(&dir).expect("png test dir");
        let path = dir.join(format!("round-trip-{}.png", std::process::id()));
        image::RgbImage::from_pixel(32, 24, image::Rgb([10, 120, 200]))
            .save(&path)
            .expect("write png fixture");

        let metadata = Metadata {
            title: "Png Round Trip".into(),
            caption: "A png caption for the metadata round-trip test.".into(),
            keywords: vec!["png".into(), "alpha".into(), "beta".into()],
            category: "Nature".into(),
        };
        tool.write(&path, &metadata).expect("write png metadata");
        let read = tool.read(&path).expect("read png metadata");
        verify_written(&metadata, &read).expect("png metadata verification");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verify_written_accepts_caption_fallback_and_case_insensitive_keywords() {
        let metadata = Metadata {
            title: "Golden Gate Sunrise".into(),
            caption: String::new(),
            keywords: vec!["bridge".into(), "Sunrise".into()],
            category: String::new(),
        };
        let read = ExistingMetadata {
            title: Some("Golden Gate Sunrise".into()),
            caption: Some("Golden Gate Sunrise".into()),
            keywords: vec!["bridge".into(), "sunrise".into()],
            ..ExistingMetadata::default()
        };
        verify_written(&metadata, &read).expect("fallback caption and keyword case should verify");

        let bad = ExistingMetadata {
            title: Some("Different Title".into()),
            ..read
        };
        assert!(verify_written(&metadata, &bad).is_err());
    }

    #[test]
    fn overwriting_metadata_replaces_previous_keywords() {
        let Some(tool) = require_tool() else { return };
        let path = temp_jpeg("overwrite", 48, 32);

        let first = Metadata {
            title: "First Title".into(),
            caption: "First caption that is long enough for the stock metadata round-trip.".into(),
            keywords: vec!["one".into(), "two".into(), "three".into()],
            category: String::new(),
        };
        tool.write(&path, &first).expect("first write");

        let second = Metadata {
            title: "Second Title".into(),
            caption: "Second caption that is long enough for the stock metadata round-trip.".into(),
            keywords: vec!["alpha".into(), "beta".into()],
            category: String::new(),
        };
        tool.write(&path, &second).expect("second write");

        let read = tool.read(&path).expect("read after overwrite");
        assert_eq!(read.title.as_deref(), Some("Second Title"));
        assert_eq!(
            read.caption.as_deref(),
            Some("Second caption that is long enough for the stock metadata round-trip.")
        );
        assert_eq!(read.keywords.len(), 2, "got: {:?}", read.keywords);
        assert!(read.keywords.iter().any(|keyword| keyword == "alpha"));
        assert!(read.keywords.iter().any(|keyword| keyword == "beta"));
        assert!(!read.keywords.iter().any(|keyword| keyword == "one"));
        assert!(!read.keywords.iter().any(|keyword| keyword == "two"));
        assert!(!read.keywords.iter().any(|keyword| keyword == "three"));
    }

    #[test]
    fn unicode_metadata_round_trips() {
        let Some(tool) = require_tool() else { return };
        let path = temp_jpeg("unicode", 48, 32);

        let metadata = Metadata {
            title: "Café Terrace at Night".into(),
            caption: "Café tables under warm evening light in an old town square.".into(),
            keywords: vec!["café".into(), "terrace".into(), "naïve".into()],
            category: String::new(),
        };
        tool.write(&path, &metadata).expect("unicode write");
        let read = tool.read(&path).expect("unicode read");

        assert_eq!(read.title.as_deref(), Some("Café Terrace at Night"));
        assert_eq!(
            read.caption.as_deref(),
            Some("Café tables under warm evening light in an old town square.")
        );
        assert!(read
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase() == "café"));
        assert!(read.keywords.iter().any(|keyword| keyword == "naïve"));
    }

    #[test]
    fn fifty_keywords_round_trip() {
        let Some(tool) = require_tool() else { return };
        let path = temp_jpeg("fifty", 48, 32);

        let keywords: Vec<String> = (0..50)
            .map(|index| {
                format!(
                    "{}{}",
                    (b'a' + (index / 26) as u8) as char,
                    (b'a' + (index % 26) as u8) as char
                )
            })
            .collect();

        let metadata = Metadata {
            title: "Fifty Keywords Test".into(),
            caption: "A caption long enough to satisfy the microstock metadata requirements."
                .into(),
            keywords: keywords.clone(),
            category: String::new(),
        };
        tool.write(&path, &metadata).expect("write fifty keywords");
        let read = tool.read(&path).expect("read fifty keywords");

        assert_eq!(read.keywords.len(), 50, "got: {:?}", read.keywords);
        let mut expected = keywords;
        let mut actual = read.keywords;
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn writing_or_reading_a_missing_file_fails() {
        let Some(tool) = require_tool() else { return };
        let missing = std::env::temp_dir()
            .join("autowording-tests")
            .join("definitely-missing.jpg");
        let _ = std::fs::remove_file(&missing);

        let metadata = Metadata {
            title: "Missing".into(),
            caption: String::new(),
            keywords: vec![],
            category: String::new(),
        };
        assert!(tool.write(&missing, &metadata).is_err());
        assert!(tool.read(&missing).is_err());
    }

    #[test]
    fn argfile_text_is_one_argument_per_line_and_flattens_newlines() {
        let args = vec![
            "-XMP-dc:Title=Café".to_string(),
            "-IPTC:Caption-Abstract=line one\nline two".to_string(),
            r"C:\photos\a.jpg".to_string(),
        ];
        let text = argfile_text(&args);
        assert_eq!(
            text,
            "-XMP-dc:Title=Café\n-IPTC:Caption-Abstract=line one line two\nC:\\photos\\a.jpg\n"
        );
        assert!(!text.contains('\r'));
    }

    #[test]
    fn batch_read_returns_metadata_for_each_file() {
        let Some(tool) = require_tool() else { return };
        let first = temp_jpeg("batch-first", 24, 16);
        let second = temp_jpeg("batch-second", 30, 20);

        tool.write(
            &first,
            &Metadata {
                title: "Batch First".into(),
                caption: "First batch caption long enough for stock metadata.".into(),
                keywords: vec!["first".into(), "batch".into()],
                category: String::new(),
            },
        )
        .expect("first metadata write");
        tool.write(
            &second,
            &Metadata {
                title: "Batch Second".into(),
                caption: "Second batch caption long enough for stock metadata.".into(),
                keywords: vec!["second".into(), "batch".into()],
                category: String::new(),
            },
        )
        .expect("second metadata write");

        let read = tool
            .read_many(&[first.clone(), second.clone()])
            .expect("batch metadata read");
        assert_eq!(read.len(), 2);
        assert_eq!(
            read.get(&first).and_then(|meta| meta.title.as_deref()),
            Some("Batch First")
        );
        assert_eq!(
            read.get(&second).and_then(|meta| meta.title.as_deref()),
            Some("Batch Second")
        );
        assert_eq!(read.get(&first).map(|meta| meta.dims), Some(Some((24, 16))));
        assert!(tool.read_many(&[]).expect("empty batch").is_empty());
    }

    #[test]
    fn unicode_file_names_round_trip() {
        let Some(tool) = require_tool() else { return };
        let dir = std::env::temp_dir().join("autowording-tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(format!("Café 北京 {}.jpg", std::process::id()));

        let image = image::RgbImage::from_pixel(32, 24, image::Rgb([50, 90, 130]));
        image.save(&path).expect("write unicode-named jpeg");

        let metadata = Metadata {
            title: "Café 北京 Titel".into(),
            caption: "Café 北京 description with enough characters to look realistic.".into(),
            keywords: vec!["café".into(), "北京".into()],
            category: String::new(),
        };
        tool.write(&path, &metadata)
            .expect("write unicode filename");
        let read = tool.read(&path).expect("read unicode filename");

        assert_eq!(read.title.as_deref(), Some("Café 北京 Titel"));
        assert_eq!(
            read.caption.as_deref(),
            Some("Café 北京 description with enough characters to look realistic.")
        );
        assert!(read.keywords.iter().any(|keyword| keyword == "café"));
        assert!(read.keywords.iter().any(|keyword| keyword == "北京"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn explicit_program_path_is_detected() {
        let Some(tool) = require_tool() else { return };
        assert!(!tool.version.is_empty());
        let explicit = tool.program().to_string_lossy().to_string();

        let detected = ExifTool::detect(&explicit).expect("explicit path should be detected");
        assert_eq!(detected.version, tool.version);

        // A directory is also accepted and expanded to exiftool.exe.
        if let Some(parent) = tool.program().parent() {
            let from_dir = ExifTool::detect(&parent.to_string_lossy())
                .expect("directory should resolve to exiftool");
            assert!(!from_dir.version.is_empty());
        }
    }
}
