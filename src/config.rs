use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
            Provider::OpenAi => "Requests go to api.openai.com. The key is stored locally in config.toml.",
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
    /// Optional explicit path to exiftool. Empty means auto-detect.
    pub exiftool_path: String,
    /// Last folder opened in the browser, restored on next launch.
    pub last_folder: String,
    /// System-following or a persisted light/dark override.
    pub theme: ThemePreference,
    /// Browser grid and metadata inspector density.
    pub density: LayoutDensity,
    /// Number of concurrent AI workers for a batch (clamped to 1..=4 at use).
    pub concurrency: usize,
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
            exiftool_path: String::new(),
            last_folder: String::new(),
            theme: ThemePreference::System,
            density: LayoutDensity::Comfortable,
            concurrency: 1,
            path: None,
        }
    }
}

impl Config {
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
        if self.lmstudio.model.trim().eq_ignore_ascii_case("local-model") {
            self.lmstudio.model.clear();
        }
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        let mut cfg = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                eprintln!("Could not parse {}: {err}. Using defaults.", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        };
        cfg.clear_legacy_placeholders();
        cfg.path = Some(path.clone());

        if !path.exists() {
            let _ = cfg.save();
        }
        cfg
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = self.path.clone().unwrap_or_else(Self::config_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
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
        assert_eq!(cfg.anthropic.base_url, Provider::Anthropic.default_base_url());
        assert!(cfg.global_prompt.is_empty());
        assert!(cfg.exiftool_path.is_empty());
        assert!(cfg.last_folder.is_empty());
        assert_eq!(cfg.theme, ThemePreference::System);
        assert_eq!(cfg.density, LayoutDensity::Comfortable);
        assert_eq!(cfg.concurrency, 1);
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
        cfg.concurrency = 3;

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
        assert_eq!(reloaded.concurrency, 3);
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
        let cfg = Config { path: Some(explicit.clone()), ..Config::default() };
        assert_eq!(cfg.display_path(), explicit.display().to_string());
    }

    #[test]
    fn invalid_toml_is_rejected() {
        assert!(toml::from_str::<Config>("provider = [").is_err());
        assert!(toml::from_str::<Config>("provider = \"unknown_provider\"").is_err());
    }
}
