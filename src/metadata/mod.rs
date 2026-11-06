pub mod tags;
pub mod writer;

pub use tags::ExistingMetadata;
pub use writer::Metadata;

use anyhow::{bail, Context, Result};
use std::collections::HashSet;
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

    /// Read title, caption, keywords, GPS location, capture date and dimensions from a JPEG.
    pub fn read(&self, path: &Path) -> Result<ExistingMetadata> {
        let mut args: Vec<String> = vec![
            "-j".to_string(),
            "-charset".to_string(),
            "iptc=utf8".to_string(),
        ];
        args.extend(tags::READ_TAGS.iter().map(|tag| tag.to_string()));
        args.push(path.to_string_lossy().to_string());

        let output = self.run(args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("exiftool could not read {}: {}", path.display(), stderr.trim());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        tags::parse_read_json(&stdout)
            .with_context(|| format!("Could not parse metadata for {}", path.display()))
    }

    /// Write title, caption and keywords into EXIF / IPTC / XMP.
    pub fn write(&self, path: &Path, md: &Metadata) -> Result<()> {
        if !path.is_file() {
            bail!("File not found: {}", path.display());
        }

        let mut args = writer::write_args(md);
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

/// Copy the JPEG into a sibling backup directory. Repeated saves keep the
/// first backup, so the pre-AutoWording file remains recoverable.
pub fn backup_original(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .with_context(|| format!("Could not find the parent folder of {}", path.display()))?;
    let name = path
        .file_name()
        .with_context(|| format!("Could not determine the file name of {}", path.display()))?;
    let backup_dir = parent.join(BACKUP_DIR);
    std::fs::create_dir_all(&backup_dir)
        .with_context(|| format!("Could not create {}", backup_dir.display()))?;
    let backup = backup_dir.join(name);
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

    Ok(())
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
            caption: "Fog drifts under the Golden Gate Bridge at sunrise in San Francisco."
                .into(),
            keywords: vec!["bridge".into(), "sunrise".into(), "fog".into()],
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

    #[test]
    fn backup_original_keeps_the_first_copy() {
        let path = temp_jpeg("backup", 16, 16);
        let original = std::fs::read(&path).expect("read original");
        let backup = backup_original(&path).expect("first backup");
        assert_eq!(std::fs::read(&backup).expect("read backup"), original);

        std::fs::write(&path, b"changed after backup").expect("change original");
        let backup_again = backup_original(&path).expect("second backup");
        assert_eq!(backup_again, backup);
        assert_eq!(std::fs::read(&backup_again).expect("read backup again"), original);
    }

    #[test]
    fn verify_written_accepts_caption_fallback_and_case_insensitive_keywords() {
        let metadata = Metadata {
            title: "Golden Gate Sunrise".into(),
            caption: String::new(),
            keywords: vec!["bridge".into(), "Sunrise".into()],
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
            caption: "First caption that is long enough for the stock metadata round-trip."
                .into(),
            keywords: vec!["one".into(), "two".into(), "three".into()],
        };
        tool.write(&path, &first).expect("first write");

        let second = Metadata {
            title: "Second Title".into(),
            caption: "Second caption that is long enough for the stock metadata round-trip."
                .into(),
            keywords: vec!["alpha".into(), "beta".into()],
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
        };
        tool.write(&path, &metadata).expect("write unicode filename");
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
