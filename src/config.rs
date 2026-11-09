use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default and safe bounds for the user-selectable keyword target.
pub const KEYWORD_TARGET_DEFAULT: usize = 50;
pub const KEYWORD_TARGET_MIN: usize = 1;
pub const KEYWORD_TARGET_MAX: usize = 50;

/// Default and safe bounds for the share of the keyword target accepted.
pub const KEYWORD_ACCEPT_DEFAULT: usize = 80;
pub const KEYWORD_ACCEPT_MIN: usize = 1;
pub const KEYWORD_ACCEPT_MAX: usize = 100;

/// Default and safe bounds for the configurable title word range.
pub const TITLE_WORDS_DEFAULT_MIN: usize = 3;
pub const TITLE_WORDS_DEFAULT_MAX: usize = 5;
pub const TITLE_WORDS_MIN: usize = 1;
pub const TITLE_WORDS_MAX: usize = 20;

/// Default and safe bounds for the configurable caption character range.
pub const CAPTION_CHARS_DEFAULT_MIN: usize = 50;
pub const CAPTION_CHARS_DEFAULT_MAX: usize = 180;
pub const CAPTION_CHARS_MIN: usize = 10;
pub const CAPTION_CHARS_MAX: usize = 1000;

pub fn clamp_keyword_target(target: usize) -> usize {
    target.clamp(KEYWORD_TARGET_MIN, KEYWORD_TARGET_MAX)
}

pub fn clamp_keyword_accept_percent(percent: usize) -> usize {
    percent.clamp(KEYWORD_ACCEPT_MIN, KEYWORD_ACCEPT_MAX)
}

fn clamp_range(min: usize, max: usize, floor: usize, ceiling: usize) -> (usize, usize) {
    let min = min.clamp(floor, ceiling);
    let max = max.clamp(floor, ceiling);
    if min <= max {
        (min, max)
    } else {
        (max, min)
    }
}

pub fn clamp_title_word_range(min: usize, max: usize) -> (usize, usize) {
    clamp_range(min, max, TITLE_WORDS_MIN, TITLE_WORDS_MAX)
}

pub fn clamp_caption_char_range(min: usize, max: usize) -> (usize, usize) {
    clamp_range(min, max, CAPTION_CHARS_MIN, CAPTION_CHARS_MAX)
}

/// Marketplace output profile. Each profile carries the upload caps of one
/// stock agency and the default output limits used for that agency. The app's
/// global safety bounds (1..=50 keywords, 1..=20 title words, 10..=1000 caption
/// characters) always apply on top of these caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Marketplace {
    /// No marketplace-specific cap: the previous general-purpose limits.
    #[default]
    General,
    AdobeStock,
    Shutterstock,
    Istock,
    Dreamstime,
    Alamy,
    Depositphotos,
    Pond5,
}

impl Marketplace {
    pub const ALL: [Marketplace; 8] = [
        Marketplace::General,
        Marketplace::AdobeStock,
        Marketplace::Shutterstock,
        Marketplace::Istock,
        Marketplace::Dreamstime,
        Marketplace::Alamy,
        Marketplace::Depositphotos,
        Marketplace::Pond5,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Marketplace::General => "General stock",
            Marketplace::AdobeStock => "Adobe Stock",
            Marketplace::Shutterstock => "Shutterstock",
            Marketplace::Istock => "iStock",
            Marketplace::Dreamstime => "Dreamstime",
            Marketplace::Alamy => "Alamy",
            Marketplace::Depositphotos => "Depositphotos",
            Marketplace::Pond5 => "Pond5",
        }
    }

    /// Minimum keywords a stock agency will accept.
    pub fn keyword_min(self) -> usize {
        match self {
            Marketplace::Shutterstock => 7,
            _ => KEYWORD_TARGET_MIN,
        }
    }

    /// Minimum title words a stock agency requires.
    pub fn title_words_min(self) -> usize {
        match self {
            Marketplace::Shutterstock => 5,
            _ => TITLE_WORDS_MIN,
        }
    }

    /// Maximum keywords the agency accepts. Adobe Stock is the only agency
    /// here below the app's 50-keyword writer cap.
    pub fn keyword_max(self) -> usize {
        match self {
            Marketplace::AdobeStock => 49,
            _ => KEYWORD_TARGET_MAX,
        }
    }

    /// Maximum title length in words, derived from the agency's title or
    /// description character cap at roughly 12 characters per word.
    pub fn title_words_max(self) -> usize {
        match self {
            Marketplace::Pond5 => 10,
            Marketplace::Alamy | Marketplace::Depositphotos => 12,
            _ => TITLE_WORDS_MAX,
        }
    }

    /// Maximum caption length in characters. Shutterstock allows a 2048-character
    /// description, so its ceiling is the app's own 1000-character guard.
    pub fn caption_chars_max(self) -> usize {
        match self {
            Marketplace::Shutterstock | Marketplace::General => CAPTION_CHARS_MAX,
            Marketplace::Pond5 => 500,
            Marketplace::Istock => 255,
            Marketplace::Dreamstime | Marketplace::Depositphotos => 250,
            Marketplace::AdobeStock => 200,
            Marketplace::Alamy => 150,
        }
    }

    pub fn default_keyword_target(self) -> usize {
        match self {
            Marketplace::AdobeStock => 49,
            _ => KEYWORD_TARGET_DEFAULT,
        }
    }

    /// Shutterstock requires at least five words in a title; other profiles
    /// keep the app's 3–5 default, extended to 7 words where the field is roomy.
    pub fn default_title_words(self) -> (usize, usize) {
        match self {
            Marketplace::General => (TITLE_WORDS_DEFAULT_MIN, TITLE_WORDS_DEFAULT_MAX),
            Marketplace::Shutterstock => (5, 9),
            _ => (TITLE_WORDS_DEFAULT_MIN, 7),
        }
    }

    pub fn default_caption_chars(self) -> (usize, usize) {
        match self {
            Marketplace::General => (CAPTION_CHARS_DEFAULT_MIN, CAPTION_CHARS_DEFAULT_MAX),
            Marketplace::Alamy => (CAPTION_CHARS_DEFAULT_MIN, 150),
            Marketplace::Pond5 => (CAPTION_CHARS_DEFAULT_MIN, 350),
            _ => (CAPTION_CHARS_DEFAULT_MIN, 200),
        }
    }
}

/// The vision providers supported by the unified AI layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Ollama,
    LmStudio,
    OpenAi,
    Anthropic,
}

impl Provider {
    pub const ALL: [Provider; 4] = [
        Provider::Ollama,
        Provider::LmStudio,
        Provider::OpenAi,
        Provider::Anthropic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Provider::Ollama => "Ollama (local)",
            Provider::LmStudio => "LM Studio (local)",
            Provider::OpenAi => "ChatGPT · OpenAI",
            Provider::Anthropic => "Claude · Anthropic",
        }
    }

    /// Compact provider name used by the status bar.
    pub fn status_label(self) -> &'static str {
        match self {
            Provider::Ollama => "Ollama",
            Provider::LmStudio => "LM Studio",
            Provider::OpenAi => "OpenAI",
            Provider::Anthropic => "Anthropic",
        }
    }

    pub fn is_cloud(self) -> bool {
        matches!(self, Provider::OpenAi | Provider::Anthropic)
    }

    pub fn requires_key(self) -> bool {
        self.is_cloud()
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Provider::Ollama => "http://localhost:11434",
            Provider::LmStudio => "http://localhost:1234/v1",
            Provider::OpenAi => "https://api.openai.com/v1",
            Provider::Anthropic => "https://api.anthropic.com/v1",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Ollama => "llava",
            Provider::LmStudio => "",
            Provider::OpenAi => "gpt-4o-mini",
            Provider::Anthropic => "claude-sonnet-4-20250514",
        }
    }

    pub fn base_hint(self) -> &'static str {
        match self {
            Provider::Ollama => "http://localhost:11434",
            Provider::LmStudio => "http://localhost:1234/v1",
            Provider::OpenAi => "https://api.openai.com/v1",
            Provider::Anthropic => "https://api.anthropic.com/v1",
        }
    }

    pub fn model_hint(self) -> &'static str {
        match self {
            Provider::Ollama => "llava, minicpm-v, qwen2-vl …",
            Provider::LmStudio => "Model id shown in LM Studio",
            Provider::OpenAi => "gpt-4o-mini, gpt-4o …",
            Provider::Anthropic => "claude-sonnet-4-20250514 …",
        }
    }

    pub fn guidance(self) -> &'static str {
        match self {
            Provider::Ollama => {
                "Start Ollama locally and pull a vision model, e.g. `ollama pull llava`."
            }
            Provider::LmStudio => {
                "Start LM Studio, load a vision model and enable the local server on port 1234."
            }
            Provider::OpenAi => {
                "Requests go to api.openai.com. The key is stored locally in config.toml."
            }
            Provider::Anthropic => {
                "Requests go to api.anthropic.com. The key is stored locally in config.toml."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSettings {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            model: String::new(),
            api_key: String::new(),
        }
    }
}

impl ProviderSettings {
    pub fn for_provider(provider: Provider) -> Self {
        Self {
            base_url: provider.default_base_url().to_string(),
            model: provider.default_model().to_string(),
            api_key: String::new(),
        }
    }
}

/// User appearance preference. `System` re-reads the Windows app theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub const ALL: [ThemePreference; 3] = [
        ThemePreference::System,
        ThemePreference::Light,
        ThemePreference::Dark,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThemePreference::System => "Follow Windows",
            ThemePreference::Light => "Light",
            ThemePreference::Dark => "Dark",
        }
    }

    pub fn resolve(self, system_dark: bool) -> bool {
        match self {
            ThemePreference::System => system_dark,
            ThemePreference::Light => false,
            ThemePreference::Dark => true,
        }
    }
}

/// Vertical density for the browser cards and metadata inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LayoutDensity {
    #[default]
    Comfortable,
    Compact,
}

impl LayoutDensity {
    pub const ALL: [LayoutDensity; 2] = [LayoutDensity::Comfortable, LayoutDensity::Compact];

    pub fn label(self) -> &'static str {
        match self {
            LayoutDensity::Comfortable => "Comfortable",
            LayoutDensity::Compact => "Compact",
        }
    }

    pub fn is_compact(self) -> bool {
        self == LayoutDensity::Compact
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub provider: Provider,
    pub ollama: ProviderSettings,
    pub lmstudio: ProviderSettings,
    pub openai: ProviderSettings,
    pub anthropic: ProviderSettings,
    /// Extra instructions appended to the built-in system prompt.
    pub global_prompt: String,
    /// Creator name written to XMP-dc:Creator and IPTC:By-line.
    pub creator: String,
    /// Copyright notice written to XMP-dc:Rights and IPTC:CopyrightNotice.
    pub copyright: String,
    /// Credit line written to XMP-photoshop:Credit and IPTC:Credit.
    pub credit_line: String,
    /// Usage terms written to XMP-xmpRights:UsageTerms.
    pub rights_usage_terms: String,
    /// Optional explicit path to exiftool. Empty means auto-detect.
    pub exiftool_path: String,
    /// Last folder opened in the browser, restored on next launch.
    pub last_folder: String,
    /// System-following or a persisted light/dark override.
    pub theme: ThemePreference,
    /// Browser grid and metadata inspector density.
    pub density: LayoutDensity,
    /// Whether the inspector rail was collapsed on the last run.
    pub inspector_collapsed: bool,
    /// Marketplace output profile that caps the generated metadata lengths.
    pub marketplace: Marketplace,
    /// Desired number of keywords per image (clamped to 1..=50 at use).
    pub keyword_target: usize,
    /// Percentage of the keyword target accepted as complete (clamped to 1..=100).
    pub keyword_accept_percent: usize,
    /// Desired title word range (clamped to 1..=20 at use).
    pub title_min_words: usize,
    pub title_max_words: usize,
    /// Desired caption character range (clamped to 10..=1000 at use).
    pub caption_min_chars: usize,
    pub caption_max_chars: usize,
    /// Copy the original JPEG into `.autowording-backups` before the first write.
    pub backup_originals: bool,
    /// Read metadata back after writing and fail the save if it does not match.
    pub verify_writes: bool,
    /// Field-lock choices by lowercased absolute path, restored at startup.
    pub field_locks: std::collections::HashMap<String, crate::state::FieldLocks>,
    /// Where this config was loaded from / will be saved to.
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: Provider::Ollama,
            ollama: ProviderSettings::for_provider(Provider::Ollama),
            lmstudio: ProviderSettings::for_provider(Provider::LmStudio),
            openai: ProviderSettings::for_provider(Provider::OpenAi),
            anthropic: ProviderSettings::for_provider(Provider::Anthropic),
            global_prompt: String::new(),
            creator: String::new(),
            copyright: String::new(),
            credit_line: String::new(),
            rights_usage_terms: String::new(),
            exiftool_path: String::new(),
            last_folder: String::new(),
            theme: ThemePreference::System,
            density: LayoutDensity::Comfortable,
            inspector_collapsed: false,
            marketplace: Marketplace::General,
            keyword_target: KEYWORD_TARGET_DEFAULT,
            keyword_accept_percent: KEYWORD_ACCEPT_DEFAULT,
            title_min_words: TITLE_WORDS_DEFAULT_MIN,
            title_max_words: TITLE_WORDS_DEFAULT_MAX,
            caption_min_chars: CAPTION_CHARS_DEFAULT_MIN,
            caption_max_chars: CAPTION_CHARS_DEFAULT_MAX,
            backup_originals: true,
            verify_writes: true,
            field_locks: std::collections::HashMap::new(),
            path: None,
        }
    }
}

impl Config {
    /// Rights and identity fields stamped onto every saved image.
    pub fn rights(&self) -> crate::metadata::Rights {
        crate::metadata::Rights {
            creator: self.creator.clone(),
            copyright: self.copyright.clone(),
            credit_line: self.credit_line.clone(),
            usage_terms: self.rights_usage_terms.clone(),
        }
    }

    /// `%APPDATA%\AutoWording\config.toml` on Windows, next to the exe as a fallback.
    pub fn config_path() -> PathBuf {
        if let Some(dirs) = directories::ProjectDirs::from("", "", "AutoWording") {
            dirs.config_dir().join("config.toml")
        } else {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("config.toml")))
                .unwrap_or_else(|| PathBuf::from("config.toml"))
        }
    }

    /// Remove the retired LM Studio placeholder from configs written by older builds.
    fn clear_legacy_placeholders(&mut self) {
        if self
            .lmstudio
            .model
            .trim()
            .eq_ignore_ascii_case("local-model")
        {
            self.lmstudio.model.clear();
        }
    }

    pub fn load_with_warning() -> (Self, Option<String>) {
        Self::load_from(&Self::config_path())
    }

    fn load_from(path: &Path) -> (Self, Option<String>) {
        let mut warning = None;
        let mut cfg = match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(cfg) => cfg,
                Err(err) => {
                    eprintln!("Could not parse {}: {err}. Using defaults.", path.display());
                    let backup = backup_corrupt_config(path);
                    warning = Some(match &backup {
                        Some(backup) => format!(
                            "Settings could not be parsed; using defaults. Previous file copied to {}.",
                            backup
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                        ),
                        None => "Settings could not be parsed; using defaults.".to_string(),
                    });
                    Self::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                eprintln!("Could not read {}: {err}. Using defaults.", path.display());
                warning = Some(format!("Could not read settings: {err}. Using defaults."));
                Self::default()
            }
        };
        cfg.clear_legacy_placeholders();
        cfg.path = Some(path.to_path_buf());

        if !path.exists() {
            let _ = cfg.save();
        }
        (cfg, warning)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = self.path.clone().unwrap_or_else(Self::config_path);
        let text = toml::to_string_pretty(self)?;
        crate::persist::write_atomic(&path, text.as_bytes())?;
        Ok(())
    }

    pub fn display_path(&self) -> String {
        self.path
            .clone()
            .unwrap_or_else(Self::config_path)
            .display()
            .to_string()
    }

    pub fn settings(&self, provider: Provider) -> &ProviderSettings {
        match provider {
            Provider::Ollama => &self.ollama,
            Provider::LmStudio => &self.lmstudio,
            Provider::OpenAi => &self.openai,
            Provider::Anthropic => &self.anthropic,
        }
    }

    pub fn settings_mut(&mut self, provider: Provider) -> &mut ProviderSettings {
        match provider {
            Provider::Ollama => &mut self.ollama,
            Provider::LmStudio => &mut self.lmstudio,
            Provider::OpenAi => &mut self.openai,
            Provider::Anthropic => &mut self.anthropic,
        }
    }

    /// The persisted target, clamped to the active marketplace and the app bounds.
    pub fn keyword_target(&self) -> usize {
        clamp_keyword_target(self.keyword_target)
            .max(self.marketplace.keyword_min())
            .min(self.marketplace.keyword_max())
    }

    /// The persisted keyword acceptance percentage, clamped to the app bounds.
    pub fn keyword_accept_percent(&self) -> usize {
        clamp_keyword_accept_percent(self.keyword_accept_percent)
    }

    /// The persisted title range, clamped to the active marketplace, ordered,
    /// and bounded by the app limits in case an old or hand-edited config is invalid.
    pub fn title_word_range(&self) -> (usize, usize) {
        let (min, max) = clamp_title_word_range(self.title_min_words, self.title_max_words);
        let min = min
            .max(self.marketplace.title_words_min())
            .min(self.marketplace.title_words_max());
        let max = max.min(self.marketplace.title_words_max()).max(min);
        (min, max)
    }

    /// The persisted caption range, clamped to the active marketplace, ordered,
    /// and bounded by the app limits in case an old or hand-edited config is invalid.
    pub fn caption_char_range(&self) -> (usize, usize) {
        let (min, max) = clamp_caption_char_range(self.caption_min_chars, self.caption_max_chars);
        (
            min.min(self.marketplace.caption_chars_max()),
            max.min(self.marketplace.caption_chars_max()),
        )
    }

    /// Adopt a marketplace profile and its default output limits.
    pub fn apply_marketplace(&mut self, marketplace: Marketplace) {
        self.marketplace = marketplace;
        self.keyword_target = marketplace.default_keyword_target();
        let (title_min, title_max) = marketplace.default_title_words();
        self.title_min_words = title_min;
        self.title_max_words = title_max;
        let (caption_min, caption_max) = marketplace.default_caption_chars();
        self.caption_min_chars = caption_min;
        self.caption_max_chars = caption_max;
    }
}

/// Keep the unreadable file before the app can overwrite it with defaults.
fn backup_corrupt_config(path: &Path) -> Option<PathBuf> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let backup = path.with_file_name(format!("config.toml.corrupt-{stamp}"));
    std::fs::copy(path, &backup).ok().map(|_| backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join("autowording-config-tests")
            .join(format!("{}-{name}", std::process::id()))
            .join("config.toml")
    }

    #[test]
    fn corrupt_config_is_backed_up_and_reported() {
        let path = temp_config_path("corrupt");
        let dir = path.parent().expect("temp dir").to_path_buf();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        std::fs::write(&path, b"provider = [").expect("seed corrupt config");

        let (cfg, warning) = Config::load_from(&path);
        assert_eq!(cfg.provider, Provider::Ollama);
        let warning = warning.expect("corrupt config warning");
        assert!(warning.contains("config.toml.corrupt-"), "got: {warning}");

        let backups: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("read temp dir")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("config.toml.corrupt-"))
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&backups[0]).expect("read backup"),
            "provider = ["
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn provider_defaults_are_complete_and_consistent() {
        for provider in Provider::ALL {
            assert!(provider.default_base_url().starts_with("http"));
            if provider == Provider::LmStudio {
                assert!(provider.default_model().is_empty());
            } else {
                assert!(!provider.default_model().is_empty());
            }
            assert!(!provider.label().is_empty());
            assert!(!provider.status_label().is_empty());
            assert!(!provider.guidance().is_empty());
            assert!(!provider.base_hint().is_empty());
            assert!(!provider.model_hint().is_empty());

            let settings = ProviderSettings::for_provider(provider);
            assert_eq!(settings.base_url, provider.default_base_url());
            assert_eq!(settings.model, provider.default_model());
            assert!(settings.api_key.is_empty());
        }

        assert!(Provider::OpenAi.requires_key());
        assert!(Provider::Anthropic.requires_key());
        assert!(!Provider::Ollama.requires_key());
        assert!(!Provider::LmStudio.requires_key());
        assert!(!Provider::Ollama.is_cloud());
        assert!(!Provider::LmStudio.is_cloud());
        assert!(Provider::OpenAi.is_cloud());
        assert!(Provider::Anthropic.is_cloud());
    }

    #[test]
    fn legacy_lmstudio_placeholder_is_cleared() {
        let mut cfg = Config::default();
        cfg.lmstudio.model = "local-model".into();
        cfg.clear_legacy_placeholders();
        assert!(cfg.lmstudio.model.is_empty());

        cfg.lmstudio.model = "qwen2-vl".into();
        cfg.clear_legacy_placeholders();
        assert_eq!(cfg.lmstudio.model, "qwen2-vl");
    }

    #[test]
    fn provider_names_round_trip_through_toml() {
        for provider in Provider::ALL {
            let mut cfg = Config::default();
            cfg.provider = provider;
            let text = toml::to_string_pretty(&cfg).expect("serialize config");
            let back: Config = toml::from_str(&text).expect("deserialize config");
            assert_eq!(back.provider, provider);
        }

        let text = toml::to_string_pretty(&Config {
            provider: Provider::LmStudio,
            ..Config::default()
        })
        .expect("serialize");
        assert!(text.contains("lm_studio"), "got: {text}");

        let cfg: Config = toml::from_str("provider = \"open_ai\"").expect("partial config");
        assert_eq!(cfg.provider, Provider::OpenAi);
        assert_eq!(cfg.openai.model, Provider::OpenAi.default_model());
        // All absent sections fall back to their defaults.
        assert_eq!(
            cfg.anthropic.base_url,
            Provider::Anthropic.default_base_url()
        );
        assert!(cfg.global_prompt.is_empty());
        assert!(cfg.creator.is_empty());
        assert!(cfg.copyright.is_empty());
        assert!(cfg.credit_line.is_empty());
        assert!(cfg.rights_usage_terms.is_empty());
        assert!(cfg.exiftool_path.is_empty());
        assert!(cfg.last_folder.is_empty());
        assert_eq!(cfg.theme, ThemePreference::System);
        assert_eq!(cfg.density, LayoutDensity::Comfortable);
        assert!(!cfg.inspector_collapsed);
        assert_eq!(cfg.marketplace, Marketplace::General);
        assert_eq!(cfg.keyword_target, KEYWORD_TARGET_DEFAULT);
        assert_eq!(cfg.keyword_target(), KEYWORD_TARGET_DEFAULT);
        assert_eq!(cfg.keyword_accept_percent, KEYWORD_ACCEPT_DEFAULT);
        assert_eq!(cfg.keyword_accept_percent(), KEYWORD_ACCEPT_DEFAULT);
        assert_eq!(cfg.title_min_words, TITLE_WORDS_DEFAULT_MIN);
        assert_eq!(cfg.title_max_words, TITLE_WORDS_DEFAULT_MAX);
        assert_eq!(cfg.caption_min_chars, CAPTION_CHARS_DEFAULT_MIN);
        assert_eq!(cfg.caption_max_chars, CAPTION_CHARS_DEFAULT_MAX);
        assert_eq!(cfg.title_word_range(), (3, 5));
        assert_eq!(cfg.caption_char_range(), (50, 180));
        assert!(cfg.backup_originals);
        assert!(cfg.verify_writes);
        assert!(cfg.field_locks.is_empty());
    }

    #[test]
    fn keyword_accept_percent_is_clamped() {
        let mut cfg = Config::default();
        cfg.keyword_accept_percent = 0;
        assert_eq!(cfg.keyword_accept_percent(), KEYWORD_ACCEPT_MIN);
        cfg.keyword_accept_percent = 150;
        assert_eq!(cfg.keyword_accept_percent(), KEYWORD_ACCEPT_MAX);
        cfg.keyword_accept_percent = 65;
        assert_eq!(cfg.keyword_accept_percent(), 65);
    }

    #[test]
    fn rights_fields_round_trip_through_toml() {
        let cfg = Config {
            creator: "Jane Doe".into(),
            copyright: "© 2026 Jane Doe".into(),
            credit_line: "Jane Doe".into(),
            rights_usage_terms: "Commercial use only".into(),
            ..Config::default()
        };
        let text = toml::to_string_pretty(&cfg).expect("serialize rights");
        let back: Config = toml::from_str(&text).expect("deserialize rights");
        assert_eq!(back.rights(), cfg.rights());
        assert_eq!(back.rights().creator, "Jane Doe");
        assert_eq!(back.rights().copyright, "© 2026 Jane Doe");
        assert_eq!(back.rights().usage_terms, "Commercial use only");
    }

    #[test]
    fn field_locks_round_trip_through_toml() {
        let mut cfg = Config::default();
        cfg.field_locks.insert(
            "c:/photos/one.jpg".to_string(),
            crate::state::FieldLocks {
                title: true,
                category: true,
                ..Default::default()
            },
        );
        let text = toml::to_string_pretty(&cfg).expect("serialize locks");
        let back: Config = toml::from_str(&text).expect("deserialize locks");
        assert_eq!(
            back.field_locks.get("c:/photos/one.jpg"),
            Some(&crate::state::FieldLocks {
                title: true,
                category: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn theme_preference_round_trips_and_resolves() {
        for preference in ThemePreference::ALL {
            let text = toml::to_string_pretty(&Config {
                theme: preference,
                ..Config::default()
            })
            .expect("serialize theme");
            let back: Config = toml::from_str(&text).expect("deserialize theme");
            assert_eq!(back.theme, preference);
        }

        assert!(!ThemePreference::System.resolve(false));
        assert!(ThemePreference::System.resolve(true));
        assert!(!ThemePreference::Light.resolve(true));
        assert!(ThemePreference::Dark.resolve(false));
    }

    #[test]
    fn layout_density_round_trips_and_labels() {
        for density in LayoutDensity::ALL {
            let text = toml::to_string_pretty(&Config {
                density,
                ..Config::default()
            })
            .expect("serialize density");
            let back: Config = toml::from_str(&text).expect("deserialize density");
            assert_eq!(back.density, density);
            assert!(!density.label().is_empty());
        }
        assert!(LayoutDensity::Compact.is_compact());
        assert!(!LayoutDensity::Comfortable.is_compact());
    }

    #[test]
    fn keyword_target_is_clamped_to_supported_range() {
        assert_eq!(clamp_keyword_target(0), KEYWORD_TARGET_MIN);
        assert_eq!(clamp_keyword_target(1), 1);
        assert_eq!(clamp_keyword_target(37), 37);
        assert_eq!(clamp_keyword_target(50), 50);
        assert_eq!(clamp_keyword_target(99), KEYWORD_TARGET_MAX);

        let mut cfg = Config::default();
        cfg.keyword_target = 0;
        assert_eq!(cfg.keyword_target(), KEYWORD_TARGET_MIN);
        cfg.keyword_target = 99;
        assert_eq!(cfg.keyword_target(), KEYWORD_TARGET_MAX);
    }

    #[test]
    fn title_and_caption_ranges_are_clamped_and_ordered() {
        assert_eq!(
            clamp_title_word_range(0, 0),
            (TITLE_WORDS_MIN, TITLE_WORDS_MIN)
        );
        assert_eq!(clamp_title_word_range(2, 8), (2, 8));
        assert_eq!(clamp_title_word_range(8, 2), (2, 8));
        assert_eq!(clamp_title_word_range(99, 2), (2, TITLE_WORDS_MAX));

        assert_eq!(
            clamp_caption_char_range(1, 2000),
            (CAPTION_CHARS_MIN, CAPTION_CHARS_MAX)
        );
        assert_eq!(clamp_caption_char_range(180, 50), (50, 180));

        let mut cfg = Config::default();
        cfg.title_min_words = 12;
        cfg.title_max_words = 4;
        cfg.caption_min_chars = 400;
        cfg.caption_max_chars = 80;
        assert_eq!(cfg.title_word_range(), (4, 12));
        assert_eq!(cfg.caption_char_range(), (80, 400));
    }

    #[test]
    fn marketplace_profiles_have_consistent_caps_and_defaults() {
        for marketplace in Marketplace::ALL {
            assert!(!marketplace.label().is_empty());
            assert!(marketplace.keyword_max() >= KEYWORD_TARGET_MIN);
            assert!(marketplace.keyword_max() <= KEYWORD_TARGET_MAX);
            assert!(marketplace.default_keyword_target() <= marketplace.keyword_max());

            let (title_min, title_max) = marketplace.default_title_words();
            assert!(title_min >= TITLE_WORDS_MIN);
            assert!(title_min <= title_max);
            assert!(title_max <= marketplace.title_words_max());

            let (caption_min, caption_max) = marketplace.default_caption_chars();
            assert!(caption_min >= CAPTION_CHARS_MIN);
            assert!(caption_min <= caption_max);
            assert!(caption_max <= marketplace.caption_chars_max());
        }

        assert_eq!(Marketplace::AdobeStock.keyword_max(), 49);
        assert_eq!(Marketplace::Shutterstock.keyword_min(), 7);
        assert_eq!(Marketplace::Shutterstock.title_words_min(), 5);
        assert_eq!(Marketplace::Alamy.caption_chars_max(), 150);
        assert_eq!(Marketplace::Pond5.title_words_max(), 10);
        assert_eq!(Marketplace::General.title_words_max(), TITLE_WORDS_MAX);
        assert_eq!(Marketplace::Shutterstock.default_title_words(), (5, 9));

        let text = toml::to_string_pretty(&Config {
            marketplace: Marketplace::AdobeStock,
            ..Config::default()
        })
        .expect("serialize marketplace");
        assert!(
            text.contains("marketplace = \"adobe_stock\""),
            "got: {text}"
        );
        let back: Config = toml::from_str(&text).expect("deserialize marketplace");
        assert_eq!(back.marketplace, Marketplace::AdobeStock);
    }

    #[test]
    fn active_marketplace_clamps_output_limits() {
        let mut cfg = Config::default();
        cfg.apply_marketplace(Marketplace::AdobeStock);
        assert_eq!(cfg.marketplace, Marketplace::AdobeStock);
        assert_eq!(cfg.keyword_target(), 49);
        assert_eq!(cfg.title_word_range(), (3, 7));
        assert_eq!(cfg.caption_char_range(), (50, 200));

        cfg.keyword_target = 50;
        cfg.title_max_words = 20;
        cfg.caption_max_chars = 900;
        assert_eq!(cfg.keyword_target(), 49, "Adobe Stock caps keywords at 49");
        assert_eq!(cfg.title_word_range(), (3, 20));
        assert_eq!(cfg.caption_char_range(), (50, 200), "Adobe caption cap");

        cfg.apply_marketplace(Marketplace::Alamy);
        cfg.title_max_words = 18;
        cfg.caption_max_chars = 400;
        assert_eq!(cfg.keyword_target(), 50);
        assert_eq!(cfg.title_word_range(), (3, 12), "Alamy shorter title cap");
        assert_eq!(cfg.caption_char_range(), (50, 150), "Alamy caption cap");

        cfg.apply_marketplace(Marketplace::Shutterstock);
        cfg.keyword_target = 1;
        cfg.title_min_words = 1;
        assert_eq!(cfg.keyword_target(), 7, "Shutterstock requires 7 keywords");
        assert_eq!(
            cfg.title_word_range().0,
            5,
            "Shutterstock needs 5 title words"
        );

        cfg.apply_marketplace(Marketplace::General);
        assert_eq!(cfg.keyword_target(), 50);
        assert_eq!(cfg.title_word_range(), (3, 5));
        assert_eq!(cfg.caption_char_range(), (50, 180));
    }

    #[test]
    fn settings_accessors_map_every_provider() {
        let mut cfg = Config::default();
        assert_eq!(cfg.settings(Provider::Ollama), &cfg.ollama);
        assert_eq!(cfg.settings(Provider::LmStudio), &cfg.lmstudio);
        assert_eq!(cfg.settings(Provider::OpenAi), &cfg.openai);
        assert_eq!(cfg.settings(Provider::Anthropic), &cfg.anthropic);

        let original_ollama = cfg.ollama.clone();
        cfg.settings_mut(Provider::Anthropic).api_key = "secret".into();
        assert_eq!(cfg.anthropic.api_key, "secret");
        assert_eq!(cfg.openai.api_key, "");
        assert_eq!(cfg.ollama, original_ollama);
    }

    #[test]
    fn save_writes_all_settings_and_round_trips() {
        let path = temp_config_path("roundtrip");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        let mut cfg = Config::default();
        cfg.path = Some(path.clone());
        cfg.provider = Provider::OpenAi;
        cfg.openai.api_key = "sk-test".into();
        cfg.openai.model = "gpt-4o".into();
        cfg.lmstudio.base_url = "http://localhost:4321/v1".into();
        cfg.global_prompt = "Prefer British spelling.".into();
        cfg.exiftool_path = "C:\\tools\\exiftool.exe".into();
        cfg.last_folder = "C:\\Photos\\2026_Stock".into();
        cfg.keyword_target = 37;
        cfg.keyword_accept_percent = 65;
        cfg.title_min_words = 4;
        cfg.title_max_words = 9;
        cfg.caption_min_chars = 70;
        cfg.caption_max_chars = 240;
        cfg.backup_originals = false;
        cfg.verify_writes = false;
        cfg.inspector_collapsed = true;

        cfg.save().expect("save config");
        assert!(path.is_file());

        let text = std::fs::read_to_string(&path).expect("read config");
        assert!(text.contains("sk-test"));
        assert!(text.contains("Prefer British spelling."));

        let mut reloaded: Config = toml::from_str(&text).expect("parse saved config");
        assert_eq!(reloaded.provider, Provider::OpenAi);
        assert_eq!(reloaded.openai.api_key, "sk-test");
        assert_eq!(reloaded.openai.model, "gpt-4o");
        assert_eq!(reloaded.lmstudio.base_url, "http://localhost:4321/v1");
        assert_eq!(reloaded.global_prompt, "Prefer British spelling.");
        assert_eq!(reloaded.exiftool_path, "C:\\tools\\exiftool.exe");
        assert_eq!(reloaded.last_folder, "C:\\Photos\\2026_Stock");
        assert_eq!(reloaded.keyword_target, 37);
        assert_eq!(reloaded.keyword_accept_percent(), 65);
        assert_eq!(reloaded.title_word_range(), (4, 9));
        assert_eq!(reloaded.caption_char_range(), (70, 240));
        assert!(!reloaded.backup_originals);
        assert!(!reloaded.verify_writes);
        assert!(reloaded.inspector_collapsed);
        assert!(reloaded.path.is_none(), "path is not serialized");

        // Saving twice overwrites cleanly.
        reloaded.path = Some(path.clone());
        reloaded.global_prompt.clear();
        reloaded.save().expect("save again");
        let text = std::fs::read_to_string(&path).expect("read again");
        let reloaded: Config = toml::from_str(&text).expect("parse again");
        assert!(reloaded.global_prompt.is_empty());
        assert_eq!(reloaded.openai.api_key, "sk-test");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn display_path_and_default_location_are_stable() {
        let cfg = Config::default();
        let default_path = Config::config_path();
        assert_eq!(default_path.file_name().unwrap(), "config.toml");
        assert!(default_path
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains("autowording"));
        assert_eq!(cfg.display_path(), default_path.display().to_string());

        let explicit = PathBuf::from("C:\\custom\\config.toml");
        let cfg = Config {
            path: Some(explicit.clone()),
            ..Config::default()
        };
        assert_eq!(cfg.display_path(), explicit.display().to_string());
    }

    #[test]
    fn invalid_toml_is_rejected() {
        assert!(toml::from_str::<Config>("provider = [").is_err());
        assert!(toml::from_str::<Config>("provider = \"unknown_provider\"").is_err());
    }
}
