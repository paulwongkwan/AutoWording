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
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use std::path::Path;
use std::time::Duration;

const MAX_IMAGE_DIM: u32 = 1600;
/// First-pass sampling. Low temperature keeps the metadata literal.
const DEFAULT_TEMPERATURE: f64 = 0.2;
/// The corrective retry samples hotter than the first pass. At the first-pass
/// temperature the model tends to reproduce the reply that already failed.
const RETRY_TEMPERATURE: f64 = 0.7;
/// Last-resort sampling after a retry that changed nothing.
const ESCALATION_TEMPERATURE: f64 = 0.95;
const JPEG_QUALITY: u8 = 85;
/// One initial request plus three retries before an image is marked failed.
const MAX_ATTEMPTS: usize = 4;
/// Extra wait per attempt, so a busy local server is not hammered.
const RETRY_DELAY: Duration = Duration::from_millis(300);

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
    let bytes =
        std::fs::read(path).with_context(|| format!("Could not read image {}", path.display()))?;
    let orientation = crate::orientation::from_jpeg(&bytes).unwrap_or(1);
    let img = image::load_from_memory(&bytes)
        .with_context(|| format!("Could not read image {}", path.display()))?;
    let img = crate::orientation::apply(img, orientation);
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
    // Editorial captions need the capture date, not the upload/batch date.
    let capture_date = req
        .existing
        .date
        .as_deref()
        .and_then(prompt::format_capture_date);
    let prompt_date = capture_date.as_deref().unwrap_or(&req.date);
    let system = prompt::system_prompt_with_constraints(
        prompt_date,
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

    // A malformed reply (missing labels) or a transport error is retried a few
    // times before the image is marked failed. After a parse failure the model
    // is shown its own reply and asked for the five labelled lines again.
    let mut parsed: Option<prompt::GeneratedMetadata> = None;
    let mut last_reply: Option<String> = None;
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 1..=MAX_ATTEMPTS {
        let attempt_user = match &last_reply {
            Some(raw) => prompt::unparsable_reply_prompt(&user, raw),
            None => user.clone(),
        };

        match chat(&client, &req, &system, &attempt_user, DEFAULT_TEMPERATURE).await {
            Ok(raw) => match prompt::parse_response_allow_empty_keywords(&raw, keyword_target) {
                Ok(result) => {
                    parsed = Some(result);
                    break;
                }
                Err(error) => {
                    last_reply = Some(raw);
                    last_error = Some(error);
                }
            },
            Err(error) => {
                last_reply = None;
                last_error = Some(error);
            }
        }

        if attempt < MAX_ATTEMPTS {
            tokio::time::sleep(RETRY_DELAY * attempt as u32).await;
        }
    }

    let Some(mut result) = parsed else {
        return Err(last_error
            .unwrap_or_else(|| anyhow!("{} returned an empty reply", req.provider.label()))
            .context(format!(
                "{} failed after {MAX_ATTEMPTS} attempts",
                req.provider.label()
            )));
    };

    // One corrective pass covers all recoverable misses. A valid retry replaces
    // out-of-range title/caption values; keywords are always appended so the
    // first result is preserved and only new unique terms fill a short list.
    let first_valid = metadata_ok(&result, title_range, caption_range);
    let minimum_keywords = prompt::minimum_keyword_count(keyword_target);
    let mut valid = first_valid && result.keywords.len() >= minimum_keywords;
    let mut repeated = false;

    if !valid {
        let retry_user = prompt::retry_prompt(
            req.context.as_deref(),
            Some(&req.existing),
            keyword_target,
            title_range,
            caption_range,
            &result,
        );
        if let Ok(retry_raw) = chat(&client, &req, &system, &retry_user, RETRY_TEMPERATURE).await {
            if let Ok(retry) = prompt::parse_response_with_target(&retry_raw, keyword_target) {
                repeated = prompt::same_text(&result.title, &retry.title)
                    && prompt::same_text(&result.caption, &retry.caption);
                let replace_text = !first_valid && metadata_ok(&retry, title_range, caption_range);
                merge_candidate(&mut result, retry, replace_text, keyword_target);
                valid = metadata_ok(&result, title_range, caption_range)
                    && result.keywords.len() >= minimum_keywords;
            }
        }
    }

    // A retry that leaves the metadata failing the same constraint is not
    // accepted. Escalate once with different conditions instead of handing the
    // identical failure back to the batch layer: hotter sampling, the
    // photographer's extra instructions removed, and a prompt that asks the
    // model to describe the image before rewriting it.
    if !valid {
        let escalated_system = prompt::system_prompt_with_constraints(
            prompt_date,
            "",
            keyword_target,
            title_range,
            caption_range,
        );
        let escalation_user = prompt::escalation_prompt(
            req.context.as_deref(),
            Some(&req.existing),
            keyword_target,
            title_range,
            caption_range,
            &result,
            repeated,
        );
        if let Ok(escalated_raw) = chat(
            &client,
            &req,
            &escalated_system,
            &escalation_user,
            ESCALATION_TEMPERATURE,
        )
        .await
        {
            if let Ok(candidate) =
                prompt::parse_response_with_target(&escalated_raw, keyword_target)
            {
                let replace_text = metadata_ok(&candidate, title_range, caption_range);
                merge_candidate(&mut result, candidate, replace_text, keyword_target);
            }
        }
    }

    if prompt::contains_non_english_letters(&result.title)
        || prompt::contains_non_english_letters(&result.caption)
    {
        bail!(
            "{} returned a title or caption with non-English letters after a retry.",
            req.provider.label()
        );
    }

    if !metadata_ok(&result, title_range, caption_range) {
        bail!(
            "{} returned a title with {} words and a caption with {} characters after retries; {}–{} words and {}–{} characters are required.",
            req.provider.label(),
            prompt::title_word_count(&result.title),
            prompt::caption_char_count(&result.caption),
            title_range.0,
            title_range.1,
            caption_range.0,
            caption_range.1
        );
    }

    if result.keywords.len() < minimum_keywords {
        bail!(
            "{} returned {} usable keywords after a retry; at least {} are required (target {}).",
            req.provider.label(),
            result.keywords.len(),
            minimum_keywords,
            keyword_target
        );
    }

    Ok(result)
}

fn within_range(value: usize, range: (usize, usize)) -> bool {
    (range.0..=range.1).contains(&value)
}

/// A title or caption is valid when it fits the configured range and contains
/// English letters only.
fn metadata_ok(
    metadata: &prompt::GeneratedMetadata,
    title_range: (usize, usize),
    caption_range: (usize, usize),
) -> bool {
    within_range(prompt::title_word_count(&metadata.title), title_range)
        && within_range(prompt::caption_char_count(&metadata.caption), caption_range)
        && !prompt::contains_non_english_letters(&metadata.title)
        && !prompt::contains_non_english_letters(&metadata.caption)
}

/// Keep the first reply's keywords and let the candidate fill a short list, so a
/// second pass can only add usable terms. `replace_text` swaps in the candidate
/// title/caption when the candidate pair is valid.
fn merge_candidate(
    result: &mut prompt::GeneratedMetadata,
    candidate: prompt::GeneratedMetadata,
    replace_text: bool,
    keyword_target: usize,
) {
    if replace_text {
        result.title = candidate.title;
        result.caption = candidate.caption;
        result.editorial = candidate.editorial;
    }
    let combined = result
        .keywords
        .iter()
        .chain(candidate.keywords.iter())
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    result.keywords = prompt::normalize_keywords_with_target(&combined, keyword_target);
}

async fn chat(
    client: &reqwest::Client,
    req: &AiRequest,
    system: &str,
    user: &str,
    temperature: f64,
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
                temperature,
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
                temperature,
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
                temperature,
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
                temperature,
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
