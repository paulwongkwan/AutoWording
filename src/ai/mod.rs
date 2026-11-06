pub mod anthropic;
pub mod lmstudio;
pub mod models;
pub mod ollama;
pub mod openai;
pub mod prompt;

#[cfg(test)]
mod tests;

pub use prompt::GeneratedMetadata;

use crate::config::{
    clamp_caption_char_range, clamp_keyword_target, clamp_title_word_range, Provider,
    ProviderSettings,
};
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
    pub keyword_target: usize,
    pub title_range: (usize, usize),
    pub caption_range: (usize, usize),
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
    let keyword_target = clamp_keyword_target(req.keyword_target);
    let title_range = clamp_title_word_range(req.title_range.0, req.title_range.1);
    let caption_range = clamp_caption_char_range(req.caption_range.0, req.caption_range.1);
    let system = prompt::system_prompt_with_constraints(
        &req.date,
        &req.global_prompt,
        keyword_target,
        title_range,
        caption_range,
    );
    let user = prompt::user_prompt(req.context.as_deref(), Some(&req.existing));
    let settings = &req.settings;

    if settings.model.trim().is_empty() {
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

    let raw = chat(&client, &req, &system, &user).await?;
    let mut result = prompt::parse_response_allow_empty_keywords(&raw, keyword_target).with_context(|| {
        format!(
            "{} returned a reply that could not be parsed",
            req.provider.label()
        )
    })?;

    // One corrective pass covers all recoverable misses. A valid retry replaces
    // out-of-range title/caption values; keywords are always appended so the
    // first result is preserved and only new unique terms fill a short list.
    let first_title_ok = within_range(prompt::title_word_count(&result.title), title_range);
    let first_caption_ok = within_range(prompt::caption_char_count(&result.caption), caption_range);
    let first_valid = first_title_ok && first_caption_ok;
    let keywords_short = result.keywords.len() < keyword_target;
    if !first_valid || keywords_short {
        let retry_user = prompt::retry_prompt(
            req.context.as_deref(),
            Some(&req.existing),
            keyword_target,
            title_range,
            caption_range,
            &result,
        );
        if let Ok(retry_raw) = chat(&client, &req, &system, &retry_user).await {
            if let Ok(retry) = prompt::parse_response_with_target(&retry_raw, keyword_target) {
                let retry_title_ok = within_range(prompt::title_word_count(&retry.title), title_range);
                let retry_caption_ok =
                    within_range(prompt::caption_char_count(&retry.caption), caption_range);
                if !first_valid && retry_title_ok && retry_caption_ok {
                    result.title = retry.title;
                    result.caption = retry.caption;
                    result.editorial = retry.editorial;
                }
                let combined = result
                    .keywords
                    .iter()
                    .chain(retry.keywords.iter())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                result.keywords = prompt::normalize_keywords_with_target(&combined, keyword_target);
            }
        }
    }

    if result.keywords.is_empty() {
        bail!(
            "{} returned no keywords after a retry.",
            req.provider.label()
        );
    }

    Ok(result)
}

fn within_range(value: usize, range: (usize, usize)) -> bool {
    (range.0..=range.1).contains(&value)
}

async fn chat(
    client: &reqwest::Client,
    req: &AiRequest,
    system: &str,
    user: &str,
) -> Result<String> {
    let settings = &req.settings;
    let base_url = settings.base_url.trim();
    let model = settings.model.trim();

    match req.provider {
        Provider::Ollama => {
            ollama::chat(
                client,
                base_url,
                model,
                system,
                user,
                &req.image_base64,
            )
            .await
        }
        Provider::LmStudio => {
            lmstudio::chat(
                client,
                base_url,
                model,
                system,
                user,
                &req.image_base64,
            )
            .await
        }
        Provider::OpenAi => {
            let url = endpoint(base_url, "/chat/completions");
            openai::chat(
                client,
                &url,
                Some(settings.api_key.trim()),
                model,
                system,
                user,
                &req.image_base64,
            )
            .await
        }
        Provider::Anthropic => {
            let url = endpoint(base_url, "/messages");
            anthropic::chat(
                client,
                &url,
                settings.api_key.trim(),
                model,
                system,
                user,
                &req.image_base64,
            )
            .await
        }
    }
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
