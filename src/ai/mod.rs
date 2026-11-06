pub mod anthropic;
pub mod lmstudio;
pub mod models;
pub mod ollama;
pub mod openai;
pub mod prompt;

#[cfg(test)]
mod tests;

pub use prompt::GeneratedMetadata;

use crate::config::{Provider, ProviderSettings};
use crate::metadata::ExistingMetadata;
use anyhow::{bail, Context, Result};
use base64::Engine as _;
use std::path::Path;
use std::time::Duration;

const MAX_IMAGE_DIM: u32 = 1600;
const JPEG_QUALITY: u8 = 85;

pub struct AiRequest {
    pub provider: Provider,
    pub settings: ProviderSettings,
    pub image_base64: String,
    pub context: Option<String>,
    pub existing: ExistingMetadata,
    pub global_prompt: String,
    pub date: String,
}

/// Read a JPEG, downscale it for the vision model and return base64 (no data URI prefix).
pub fn prepare_image(path: &Path) -> Result<String> {
    let img = image::open(path)
        .with_context(|| format!("Could not read image {}", path.display()))?;
    let (width, height) = (img.width(), img.height());

    let img = if width.max(height) > MAX_IMAGE_DIM {
        let scale = MAX_IMAGE_DIM as f32 / width.max(height) as f32;
        let new_width = ((width as f32 * scale).round() as u32).max(1);
        let new_height = ((height as f32 * scale).round() as u32).max(1);
        img.resize(new_width, new_height, image::imageops::FilterType::Triangle)
    } else {
        img
    };

    let rgb = img.to_rgb8();
    let mut buffer: Vec<u8> = Vec::new();
    {
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, JPEG_QUALITY);
        encoder.encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )?;
    }

    Ok(base64::engine::general_purpose::STANDARD.encode(buffer))
}

pub async fn generate(req: AiRequest) -> Result<GeneratedMetadata> {
    let system = prompt::system_prompt(&req.date, &req.global_prompt);
    let user = prompt::user_prompt(req.context.as_deref(), Some(&req.existing));
    let settings = &req.settings;
    let base_url = settings.base_url.trim();
    let model = settings.model.trim();

    if model.is_empty() {
        bail!("No model configured for {}.", req.provider.label());
    }
    if req.provider.requires_key() && settings.api_key.trim().is_empty() {
        bail!(
            "{} needs an API key. Add it in the AI provider panel.",
            req.provider.label()
        );
    }
    if req.image_base64.is_empty() {
        bail!("The image could not be encoded.");
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .context("Could not build the HTTP client")?;

    let raw = match req.provider {
        Provider::Ollama => {
            ollama::chat(
                &client,
                base_url,
                model,
                &system,
                &user,
                &req.image_base64,
            )
            .await?
        }
        Provider::LmStudio => {
            lmstudio::chat(
                &client,
                base_url,
                model,
                &system,
                &user,
                &req.image_base64,
            )
            .await?
        }
        Provider::OpenAi => {
            let url = endpoint(base_url, "/chat/completions");
            openai::chat(
                &client,
                &url,
                Some(settings.api_key.trim()),
                model,
                &system,
                &user,
                &req.image_base64,
            )
            .await?
        }
        Provider::Anthropic => {
            let url = endpoint(base_url, "/messages");
            anthropic::chat(
                &client,
                &url,
                settings.api_key.trim(),
                model,
                &system,
                &user,
                &req.image_base64,
            )
            .await?
        }
    };

    prompt::parse_response(&raw).with_context(|| {
        format!(
            "{} returned a reply that could not be parsed",
            req.provider.label()
        )
    })
}

pub fn endpoint(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return path.to_string();
    }
    if base.ends_with(path) {
        base.to_string()
    } else if path == "/api/chat" && base.ends_with("/api") {
        format!("{base}/chat")
    } else {
        format!("{base}{path}")
    }
}

pub fn snippet(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut snippet: String = flat.chars().take(400).collect();
    if flat.chars().count() > 400 {
        snippet.push('…');
    }
    snippet
}
