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
    clamp_caption_char_range, clamp_keyword_accept_percent, clamp_keyword_target,
    clamp_title_word_range, Provider, ProviderSettings,
};
use crate::metadata::ExistingMetadata;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use std::path::Path;
use std::time::Duration;

const MAX_IMAGE_DIM: u32 = 1600;
/// First-pass sampling. A moderate temperature adds variation while keeping
/// the metadata grounded in the image.
const DEFAULT_TEMPERATURE: f64 = 0.5;
/// The corrective retry samples hotter than the first pass. At the first-pass
/// temperature the model tends to reproduce the reply that already failed.
const RETRY_TEMPERATURE: f64 = 0.7;
/// Last-resort sampling after a retry that changed nothing.
const ESCALATION_TEMPERATURE: f64 = 0.95;
const JPEG_QUALITY: u8 = 85;
/// Reply budget. 50 keywords plus three sentences fit in 900 tokens on most
/// models, but reasoning-heavy local models spend part of the budget on a
/// preamble and truncate the labelled lines, which then fails to parse.
const MAX_REPLY_TOKENS: u32 = 1400;
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
    pub keyword_accept_percent: usize,
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
    let keyword_accept_percent = clamp_keyword_accept_percent(req.keyword_accept_percent);
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
    // The per-image context stored with the file is more specific than the
    // app-wide context, so both are sent, image first.
    let context = prompt::combined_context(req.context.as_deref(), req.existing.context.as_deref());
    let user = prompt::user_prompt(context.as_deref(), Some(&req.existing));
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

        match chat(
            &client,
            &req,
            &system,
            &attempt_user,
            Some(&req.image_base64),
            DEFAULT_TEMPERATURE,
        )
        .await
        {
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
    let minimum_keywords = prompt::minimum_keyword_count(keyword_target, keyword_accept_percent);
    let mut valid = first_valid && result.keywords.len() >= minimum_keywords;
    let mut repeated = false;

    if !valid {
        let retry_user = prompt::retry_prompt(
            context.as_deref(),
            Some(&req.existing),
            keyword_target,
            keyword_accept_percent,
            title_range,
            caption_range,
            &result,
        );
        if let Ok(retry_raw) = chat(
            &client,
            &req,
            &system,
            &retry_user,
            Some(&req.image_base64),
            RETRY_TEMPERATURE,
        )
        .await
        {
            if let Ok(retry) = prompt::parse_response_with_target(&retry_raw, keyword_target) {
                repeated = prompt::same_text(&result.title, &retry.title)
                    && prompt::same_text(&result.caption, &retry.caption);
                merge_candidate(
                    &mut result,
                    retry,
                    title_range,
                    caption_range,
                    keyword_target,
                );
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
            context.as_deref(),
            Some(&req.existing),
            keyword_target,
            keyword_accept_percent,
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
            Some(&req.image_base64),
            ESCALATION_TEMPERATURE,
        )
        .await
        {
            if let Ok(candidate) =
                prompt::parse_response_with_target(&escalated_raw, keyword_target)
            {
                merge_candidate(
                    &mut result,
                    candidate,
                    title_range,
                    caption_range,
                    keyword_target,
                );
            }
        }
    }

    // The keyword floor is the most common remaining miss: a 50-keyword target
    // at 80% acceptance needs 40 distinct single-word terms, and an otherwise
    // correct reply is thrown away over the last few. Ask for just the missing
    // terms instead of regenerating the title and caption, which could lose
    // fields that already passed. It runs text-only (`None` image) with a short
    // system instruction, so the request is cheap and cannot rewrite the text.
    if metadata_ok(&result, title_range, caption_range) && result.keywords.len() < minimum_keywords
    {
        let topup_user = prompt::keyword_topup_prompt(
            context.as_deref(),
            Some(&req.existing),
            keyword_target,
            keyword_accept_percent,
            &result,
        );
        if let Ok(raw) = chat(
            &client,
            &req,
            prompt::KEYWORD_TOPUP_SYSTEM,
            &topup_user,
            None,
            RETRY_TEMPERATURE,
        )
        .await
        {
            let extra = prompt::parse_keywords_reply(&raw, keyword_target);
            append_keywords(&mut result.keywords, &extra, keyword_target);
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

    if !title_ok(&result, title_range) {
        bail!(
            "{} returned a title with {} words after retries; {}–{} words are required.",
            req.provider.label(),
            prompt::title_word_count(&result.title),
            title_range.0,
            title_range.1
        );
    }

    if !caption_ok(&result, caption_range) {
        bail!(
            "{} returned a caption with {} characters after retries; {}–{} characters are required.",
            req.provider.label(),
            prompt::caption_char_count(&result.caption),
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

/// Why the last attempt on an image failed, as far as the message can tell.
/// The batch summary groups failures by this value so the constraint that is
/// actually costing images is visible without opening each one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureCause {
    /// The reply had fewer usable keywords than the acceptance floor.
    Keywords,
    TitleLength,
    CaptionLength,
    NonEnglish,
    /// The reply could not be parsed into the labelled fields at all.
    Reply,
    /// The provider could not be reached or returned a server error.
    Connection,
    /// Missing model, API key or an unusable image.
    Setup,
    Image,
    Other,
}

impl FailureCause {
    pub const ALL: [FailureCause; 9] = [
        FailureCause::Keywords,
        FailureCause::TitleLength,
        FailureCause::CaptionLength,
        FailureCause::NonEnglish,
        FailureCause::Reply,
        FailureCause::Connection,
        FailureCause::Setup,
        FailureCause::Image,
        FailureCause::Other,
    ];

    /// Short label used in the batch summary.
    pub fn label(self) -> &'static str {
        match self {
            FailureCause::Keywords => "keywords short",
            FailureCause::TitleLength => "title length",
            FailureCause::CaptionLength => "caption length",
            FailureCause::NonEnglish => "non-English text",
            FailureCause::Reply => "unreadable reply",
            FailureCause::Connection => "connection",
            FailureCause::Setup => "setup",
            FailureCause::Image => "image",
            FailureCause::Other => "other",
        }
    }
}

/// Classify a stored failure message. The messages come from this module and
/// from the batch worker, so a marker scan is enough and keeps the error text
/// readable for the user.
pub fn classify_failure(message: &str) -> FailureCause {
    let text = message.to_ascii_lowercase();
    if text.contains("usable keywords") {
        return FailureCause::Keywords;
    }
    if text.contains("non-english letters") {
        return FailureCause::NonEnglish;
    }
    if text.contains("a title with") {
        return FailureCause::TitleLength;
    }
    if text.contains("a caption with") {
        return FailureCause::CaptionLength;
    }
    if text.contains("no model configured")
        || text.contains("needs an api key")
        || text.contains("could not be encoded")
    {
        return FailureCause::Setup;
    }
    if text.contains("could not read image") || text.contains("image preparation failed") {
        return FailureCause::Image;
    }
    if text.contains("reply was missing")
        || text.contains("empty reply")
        || text.contains("could not be read")
    {
        return FailureCause::Reply;
    }
    if text.contains("error sending request")
        || text.contains("connection")
        || text.contains("timed out")
        || text.contains("timeout")
        || text.contains("refused")
        || text.contains("unreachable")
        || text.contains("builder error")
        || text.contains("dns")
    {
        return FailureCause::Connection;
    }
    FailureCause::Other
}

fn within_range(value: usize, range: (usize, usize)) -> bool {
    (range.0..=range.1).contains(&value)
}

/// A title is valid when it fits the configured word range and contains English
/// letters only.
fn title_ok(metadata: &prompt::GeneratedMetadata, title_range: (usize, usize)) -> bool {
    within_range(prompt::title_word_count(&metadata.title), title_range)
        && !prompt::contains_non_english_letters(&metadata.title)
}

/// A caption is valid when it fits the configured character range and contains
/// English letters only.
fn caption_ok(metadata: &prompt::GeneratedMetadata, caption_range: (usize, usize)) -> bool {
    within_range(prompt::caption_char_count(&metadata.caption), caption_range)
        && !prompt::contains_non_english_letters(&metadata.caption)
}

/// A reply is valid when both text fields are valid.
fn metadata_ok(
    metadata: &prompt::GeneratedMetadata,
    title_range: (usize, usize),
    caption_range: (usize, usize),
) -> bool {
    title_ok(metadata, title_range) && caption_ok(metadata, caption_range)
}

/// Keep the first reply's keywords and let the candidate fill a short list, so a
/// second pass can only add usable terms.
///
/// Each text field is accepted on its own: a retry that fixes the title while
/// overshooting the caption length (or the reverse) must not throw the repaired
/// field away, otherwise one stubborn field keeps costing whole attempts.
fn merge_candidate(
    result: &mut prompt::GeneratedMetadata,
    candidate: prompt::GeneratedMetadata,
    title_range: (usize, usize),
    caption_range: (usize, usize),
    keyword_target: usize,
) {
    let take_title = title_ok(&candidate, title_range);
    let take_caption = caption_ok(&candidate, caption_range);
    if take_title {
        result.title = candidate.title;
    }
    if take_caption {
        result.caption = candidate.caption;
        // The editorial flag describes the caption that is actually kept.
        result.editorial = candidate.editorial;
    }
    if (take_title && take_caption) || result.category.trim().is_empty() {
        result.category = candidate.category;
    }
    append_keywords(&mut result.keywords, &candidate.keywords, keyword_target);
}

/// Add new terms to an accepted list, keeping its order and re-running the same
/// duplicate, singular/plural and cap rules used for a fresh reply.
fn append_keywords(keywords: &mut Vec<String>, extra: &[String], keyword_target: usize) {
    if extra.is_empty() {
        return;
    }
    let combined = keywords
        .iter()
        .chain(extra.iter())
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    *keywords = prompt::normalize_keywords_with_target(&combined, keyword_target);
}

async fn chat(
    client: &reqwest::Client,
    req: &AiRequest,
    system: &str,
    user: &str,
    image: Option<&str>,
    temperature: f64,
) -> Result<String> {
    let settings = &req.settings;
    let base_url = settings.base_url.trim();
    let model = settings.model.trim();

    match req.provider {
        Provider::Ollama => {
            ollama::chat(client, base_url, model, system, user, image, temperature).await
        }
        Provider::LmStudio => {
            lmstudio::chat(client, base_url, model, system, user, image, temperature).await
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
                image,
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
                image,
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
