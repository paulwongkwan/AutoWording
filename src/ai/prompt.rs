use crate::config::{
    clamp_caption_char_range, clamp_keyword_target, clamp_title_word_range,
    KEYWORD_TARGET_DEFAULT,
};
#[cfg(test)]
use crate::config::{
    CAPTION_CHARS_DEFAULT_MAX, CAPTION_CHARS_DEFAULT_MIN, TITLE_WORDS_DEFAULT_MAX,
    TITLE_WORDS_DEFAULT_MIN,
};
use crate::metadata::ExistingMetadata;
use anyhow::{bail, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

pub const KEYWORD_TARGET: usize = KEYWORD_TARGET_DEFAULT;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeneratedMetadata {
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
    pub editorial: bool,
}

const SYSTEM_TEMPLATE: &str = r#"Role & Task:
You are an expert photography analyst specializing in metadata tag generation, keywording, and stock image description. Analyze the provided image and generate accurate titles, captions, and keywords following strict guidelines.

Analysis Guidelines:
1. Identification Focus:
   - Identify specific landmarks, famous buildings, cities, or locations whenever possible.
   - Specify exact biological classification for fauna and flora (e.g., "Monarch Butterfly - Danaus plexippus", "Quercus robur") rather than broad categories.
   - Detail vehicle makes, models, and types, along with specific technical or industrial objects.
2. Commercial vs. Editorial Status:
   - Check for recognizable human faces.
   - If a recognizable face is present, mark the image as EDITORIAL and construct the caption following the Shutterstock standard format:
     CITY, STATE/COUNTRY – MONTH DAY, YEAR: [Factual sentence describing the subject matter].
   - Use today's date for the editorial timestamp.

Today's date: {DATE}

Output Formatting Constraints:

1. Title:
   - Length: Between {TITLE_MIN_WORDS} and {TITLE_MAX_WORDS} words.
   - Content: Clear, descriptive summary of the primary subject.

2. Caption:
   - Length: Strict limit of {CAPTION_MIN_CHARS} to {CAPTION_MAX_CHARS} characters (including spaces).
   - Content: Factual description of the scene (or Shutterstock editorial format if applicable).

3. Keywords:
   - Quantity: Exactly {KEYWORD_TARGET} keywords.
   - Format: Single words only (no multi-word phrases/keyphrases).
   - Separator: Comma-separated.
   - Rules: Alphabetic characters only. No numbers, punctuation, or special characters.
   - Duplicates: Do not include a plural when its singular form is also present. Keep the singular (for example, keep "apple" and omit "apples").

Required Output Schema:
Title: [{TITLE_MIN_WORDS}-{TITLE_MAX_WORDS} words]
Editorial Status: [Commercial / Editorial]
Caption: [{CAPTION_MIN_CHARS}-{CAPTION_MAX_CHARS} characters]
Keywords: [word1, word2, word3, ... word{KEYWORD_TARGET}]

Reply with the four labelled lines above and nothing else. Do not use markdown."#;

#[cfg(test)]
pub fn system_prompt(date: &str, extra: &str) -> String {
    system_prompt_with_constraints(
        date,
        extra,
        KEYWORD_TARGET,
        (TITLE_WORDS_DEFAULT_MIN, TITLE_WORDS_DEFAULT_MAX),
        (CAPTION_CHARS_DEFAULT_MIN, CAPTION_CHARS_DEFAULT_MAX),
    )
}

pub fn system_prompt_with_constraints(
    date: &str,
    extra: &str,
    keyword_target: usize,
    title_range: (usize, usize),
    caption_range: (usize, usize),
) -> String {
    let target = clamp_keyword_target(keyword_target);
    let (title_min, title_max) = clamp_title_word_range(title_range.0, title_range.1);
    let (caption_min, caption_max) =
        clamp_caption_char_range(caption_range.0, caption_range.1);
    let mut prompt = SYSTEM_TEMPLATE
        .replace("{DATE}", date)
        .replace("{KEYWORD_TARGET}", &target.to_string())
        .replace("{TITLE_MIN_WORDS}", &title_min.to_string())
        .replace("{TITLE_MAX_WORDS}", &title_max.to_string())
        .replace("{CAPTION_MIN_CHARS}", &caption_min.to_string())
        .replace("{CAPTION_MAX_CHARS}", &caption_max.to_string());
    let extra = extra.trim();
    if !extra.is_empty() {
        prompt.push_str("\n\nAdditional instructions from the photographer (follow these too):\n");
        prompt.push_str(extra);
    }
    prompt
}

pub fn user_prompt(context: Option<&str>, existing: Option<&ExistingMetadata>) -> String {
    let metadata = existing
        .map(existing_metadata_block)
        .filter(|block| !block.is_empty());
    let context = context.map(str::trim).filter(|c| !c.is_empty());
    let mut prompt = String::new();

    if let Some(metadata) = metadata {
        prompt.push_str(
            "Existing file metadata (reference only; the returned title, caption, and keywords replace these values):\n",
        );
        prompt.push_str(&metadata);
        prompt.push_str("\n\n");
    }
    if let Some(context) = context {
        prompt.push_str(
            "Situational context supplied by the photographer (treat it as ground truth):\n",
        );
        prompt.push_str(context);
        prompt.push_str("\n\n");
    }
    prompt.push_str(
        "Analyze the attached image and return the four labelled fields exactly as specified.",
    );
    prompt
}

/// Second-pass prompt used when the first reply missed the title, caption, or
/// keyword constraints. It repeats the original context and asks for a full
/// reply so the normal four-field parser can be used again.
pub fn retry_prompt(
    context: Option<&str>,
    existing: Option<&ExistingMetadata>,
    keyword_target: usize,
    title_range: (usize, usize),
    caption_range: (usize, usize),
    previous: &GeneratedMetadata,
) -> String {
    let target = clamp_keyword_target(keyword_target);
    let (title_min, title_max) = clamp_title_word_range(title_range.0, title_range.1);
    let (caption_min, caption_max) =
        clamp_caption_char_range(caption_range.0, caption_range.1);
    let words = title_word_count(&previous.title);
    let chars = caption_char_count(&previous.caption);
    let title_ok = (title_min..=title_max).contains(&words);
    let caption_ok = (caption_min..=caption_max).contains(&chars);
    let keywords_short = previous.keywords.len() < target;

    let mut issues = Vec::new();
    if !title_ok {
        issues.push(format!(
            "The title has {words} words; it must have {title_min}–{title_max}."
        ));
    }
    if !caption_ok {
        issues.push(format!(
            "The caption has {chars} characters; it must have {caption_min}–{caption_max} characters."
        ));
    }
    if keywords_short {
        let verb = if previous.keywords.len() == 1 {
            "keyword was"
        } else {
            "keywords were"
        };
        issues.push(format!(
            "Only {} usable {verb} returned; exactly {target} are required.",
            previous.keywords.len()
        ));
    }

    let mut prompt = user_prompt(context, existing);
    prompt.push_str("\n\nThe previous reply did not satisfy these constraints:\n");
    for issue in issues {
        prompt.push_str("- ");
        prompt.push_str(&issue);
        prompt.push('\n');
    }
    prompt.push_str("\nReturn the four labelled fields again. ");
    if title_ok {
        prompt.push_str("Keep the title unchanged. ");
    } else {
        prompt.push_str(&format!(
            "Rewrite the title to {title_min}–{title_max} words. "
        ));
    }
    if caption_ok {
        prompt.push_str("Keep the caption unchanged. ");
    } else {
        prompt.push_str(&format!(
            "Rewrite the caption to {caption_min}–{caption_max} characters. "
        ));
    }
    if keywords_short {
        prompt.push_str(&format!(
            "Return exactly {target} unique single-word keywords, adding enough new terms that were not in the previous list. Previous keywords: {}. ",
            previous.keywords.join(", ")
        ));
    } else {
        prompt.push_str("Keep the keywords unchanged. ");
    }
    prompt.push_str("Do not repeat any keyword, and do not include both a singular and its plural; keep the singular.");
    prompt
}

fn existing_metadata_block(existing: &ExistingMetadata) -> String {
    let mut lines = Vec::new();
    if let Some(gps) = existing
        .gps
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("GPS location: {gps}"));
    }
    if let Some(date) = existing
        .date
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Capture date: {date}"));
    }
    if let Some(title) = existing
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Title: {title}"));
    }
    if let Some(caption) = existing
        .caption
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Caption: {caption}"));
    }
    if !existing.keywords.is_empty() {
        lines.push(format!("Keywords: {}", existing.keywords.join(", ")));
    }
    if !lines.is_empty() {
        lines.push(
            "Use these details as context and return fresh title, caption, and keywords."
                .to_string(),
        );
    }
    lines.join("\n")
}

fn field_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[\s,;])(?:\d+[.)]\s*)?(?:[*#>\-]+\s*)?(Title|Editorial\s+Status|Editorial|Caption|Description|Keywords?)\s*\**\s*[:\u{FF1A}]",
        )
        .expect("valid field regex")
    })
}

fn label_key(label: &str) -> &'static str {
    let lower = label.to_ascii_lowercase();
    if lower.starts_with("title") {
        "title"
    } else if lower.starts_with("editorial") {
        "editorial"
    } else if lower.starts_with("caption") || lower.starts_with("description") {
        "caption"
    } else {
        "keywords"
    }
}

fn clean_value(value: &str) -> String {
    let trimmed = value
        .trim()
        .trim_matches(|c: char| {
            c == '*' || c == '`' || c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '_'
        })
        .trim();
    let mut cleaned = String::with_capacity(trimmed.len());
    let mut last_space = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !last_space {
                cleaned.push(' ');
            }
            last_space = true;
        } else {
            cleaned.push(ch);
            last_space = false;
        }
    }
    cleaned
        .trim()
        .trim_start_matches('"')
        .trim_end_matches('"')
        .trim()
        .to_string()
}

fn parse_fields(text: &str) -> BTreeMap<&'static str, String> {
    let re = field_regex();
    let matches: Vec<_> = re.find_iter(text).collect();
    let mut fields = BTreeMap::new();

    for (i, m) in matches.iter().enumerate() {
        let Some(captures) = re.captures(m.as_str()) else {
            continue;
        };
        let Some(label) = captures.get(1) else {
            continue;
        };
        let end = matches.get(i + 1).map_or(text.len(), |next| next.start());
        let value = text.get(m.end()..end).unwrap_or_default();
        fields.insert(label_key(label.as_str()), clean_value(value));
    }

    fields
}

#[cfg(test)]
pub fn parse_response(text: &str) -> Result<GeneratedMetadata> {
    parse_response_with_target(text, KEYWORD_TARGET)
}

pub fn parse_response_with_target(
    text: &str,
    keyword_target: usize,
) -> Result<GeneratedMetadata> {
    parse_response_inner(text, keyword_target, true)
}

/// Parse a first-pass reply even when it omitted keywords, so the caller can
/// make the keyword-refresh request instead of failing the whole image.
pub fn parse_response_allow_empty_keywords(
    text: &str,
    keyword_target: usize,
) -> Result<GeneratedMetadata> {
    parse_response_inner(text, keyword_target, false)
}

fn parse_response_inner(
    text: &str,
    keyword_target: usize,
    require_keywords: bool,
) -> Result<GeneratedMetadata> {
    let target = clamp_keyword_target(keyword_target);
    let fields = parse_fields(text);

    let title = fields.get("title").cloned().unwrap_or_default();
    let caption = fields.get("caption").cloned().unwrap_or_default();
    let keyword_raw = fields.get("keywords").cloned().unwrap_or_default();
    let editorial_raw = fields
        .get("editorial")
        .cloned()
        .unwrap_or_else(|| "Commercial".to_string());

    let keywords = normalize_keywords_with_target(&keyword_raw, target);

    let missing_keywords = require_keywords && keywords.is_empty();
    if title.is_empty() || caption.is_empty() || missing_keywords {
        let mut missing = Vec::new();
        if title.is_empty() {
            missing.push("Title");
        }
        if caption.is_empty() {
            missing.push("Caption");
        }
        if missing_keywords {
            missing.push("Keywords");
        }
        let snippet: String = text.trim().chars().take(400).collect();
        bail!(
            "The model reply was missing {}. Raw reply:\n{}",
            missing.join(", "),
            snippet
        );
    }

    // Explicit "Commercial" wins, otherwise look for the word editorial.
    let editorial_status = editorial_raw.to_ascii_lowercase();
    let editorial = !editorial_status.contains("commercial")
        && editorial_status.contains("editorial");

    Ok(GeneratedMetadata {
        title: title.trim_end_matches('.').trim().to_string(),
        caption,
        keywords,
        editorial,
    })
}

/// Strip punctuation/numbers, split multi-word phrases, deduplicate and cap at 50.
#[cfg(test)]
pub fn normalize_keywords(raw: &str) -> Vec<String> {
    normalize_keywords_with_target(raw, KEYWORD_TARGET)
}

/// Same as [`normalize_keywords`], but uses the user's target. Exact duplicates
/// and singular/plural pairs are removed before the list is capped, so excess
/// terms do not crowd out valid alternatives.
pub fn normalize_keywords_with_target(raw: &str, keyword_target: usize) -> Vec<String> {
    let target = clamp_keyword_target(keyword_target);
    let mut keywords: Vec<String> = Vec::with_capacity(target);
    let mut seen = HashSet::new();

    for chunk in raw.split(|c: char| {
        c == ',' || c == ';' || c == '\n' || c == '\r' || c == '|' || c == '/'
    }) {
        for word in chunk.split_whitespace() {
            let cleaned: String = word.chars().filter(|c| c.is_alphabetic()).collect();
            if cleaned.chars().count() < 2 {
                continue;
            }
            if seen.insert(cleaned.to_lowercase()) {
                keywords.push(cleaned);
            }
        }
    }

    let present: HashSet<String> = keywords.iter().map(|word| word.to_lowercase()).collect();
    keywords.retain(|word| !is_plural_form(word, &present));
    keywords.truncate(target);
    keywords
}

const INVARIANT_PLURALS: &[&str] = &[
    "series",
    "species",
    "news",
    "means",
    "sheep",
    "fish",
    "deer",
    "moose",
    "salmon",
    "trout",
    "aircraft",
    "spacecraft",
    "offspring",
    "headquarters",
    "scissors",
    "trousers",
    "clothes",
    "jeans",
    "pants",
    "shorts",
    "bison",
    "swine",
    "cod",
    "shrimp",
];

/// Candidate singular forms for a word, used only to detect a pair that is
/// already present. A plural on its own is left untouched.
fn singular_candidates(word: &str) -> Vec<String> {
    let lower = word.to_lowercase();
    if INVARIANT_PLURALS.contains(&lower.as_str()) {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    let irregular: &[&str] = match lower.as_str() {
        "people" => &["person"],
        "children" => &["child"],
        "men" => &["man"],
        "women" => &["woman"],
        "feet" => &["foot"],
        "teeth" => &["tooth"],
        "mice" => &["mouse"],
        "geese" => &["goose"],
        "oxen" => &["ox"],
        "indices" => &["index"],
        "matrices" => &["matrix"],
        "vertices" => &["vertex"],
        "appendices" => &["appendix"],
        "analyses" => &["analysis"],
        "crises" => &["crisis"],
        "theses" => &["thesis"],
        "diagnoses" => &["diagnosis"],
        "axes" => &["axis", "axe"],
        "bacteria" => &["bacterium"],
        "criteria" => &["criterion"],
        "phenomena" => &["phenomenon"],
        "cacti" => &["cactus"],
        "fungi" => &["fungus"],
        "nuclei" => &["nucleus"],
        "stimuli" => &["stimulus"],
        "alumni" => &["alumnus"],
        "radii" => &["radius"],
        _ => &[],
    };
    candidates.extend(irregular.iter().map(|candidate| candidate.to_string()));

    let chars = lower.chars().count();
    if chars > 3 && lower.ends_with("ies") {
        candidates.push(format!("{}y", &lower[..lower.len() - 3]));
    }
    if chars > 3 && lower.ends_with("ves") {
        let stem = &lower[..lower.len() - 3];
        candidates.push(format!("{stem}f"));
        candidates.push(format!("{stem}fe"));
    }
    if chars > 2 && lower.ends_with("es") {
        candidates.push(lower[..lower.len() - 2].to_string());
    }
    if chars > 2
        && lower.ends_with('s')
        && !lower.ends_with("ss")
        && !lower.ends_with("us")
        && !lower.ends_with("is")
    {
        candidates.push(lower[..lower.len() - 1].to_string());
    }

    candidates.retain(|candidate| {
        candidate.as_str() != lower
            && candidate.chars().count() >= 2
            && candidate.chars().all(char::is_alphabetic)
    });
    candidates.sort();
    candidates.dedup();
    candidates
}

fn is_plural_form(word: &str, present: &HashSet<String>) -> bool {
    singular_candidates(word)
        .iter()
        .any(|candidate| present.contains(candidate))
}

/// True when `plural` is a plural form of `singular`, using the same
/// conservative candidate rules as normalization.
pub fn is_plural_of(plural: &str, singular: &str) -> bool {
    let singular = singular.to_lowercase();
    singular_candidates(plural)
        .iter()
        .any(|candidate| candidate == &singular)
}

pub fn is_valid_keyword(word: &str) -> bool {
    word.chars().count() >= 2 && word.chars().all(|c| c.is_alphabetic())
}

pub fn title_word_count(title: &str) -> usize {
    title.split_whitespace().count()
}

pub fn caption_char_count(caption: &str) -> usize {
    caption.chars().count()
}

#[cfg(test)]
pub fn validate(title: &str, caption: &str, keywords: &[String]) -> Vec<String> {
    validate_with_constraints(
        title,
        caption,
        keywords,
        KEYWORD_TARGET,
        (TITLE_WORDS_DEFAULT_MIN, TITLE_WORDS_DEFAULT_MAX),
        (CAPTION_CHARS_DEFAULT_MIN, CAPTION_CHARS_DEFAULT_MAX),
    )
}

pub fn validate_with_constraints(
    title: &str,
    caption: &str,
    keywords: &[String],
    keyword_target: usize,
    title_range: (usize, usize),
    caption_range: (usize, usize),
) -> Vec<String> {
    let target = clamp_keyword_target(keyword_target);
    let (title_min, title_max) = clamp_title_word_range(title_range.0, title_range.1);
    let (caption_min, caption_max) =
        clamp_caption_char_range(caption_range.0, caption_range.1);
    let mut warnings = Vec::new();

    let words = title_word_count(title);
    if !(title_min..=title_max).contains(&words) {
        warnings.push(format!(
            "Title has {words} words (needs {title_min}–{title_max})"
        ));
    }

    let chars = caption_char_count(caption);
    if !(caption_min..=caption_max).contains(&chars) {
        warnings.push(format!(
            "Caption is {chars} characters (needs {caption_min}–{caption_max})"
        ));
    }

    if keywords.len() != target {
        warnings.push(format!(
            "{} keywords (needs exactly {target})",
            keywords.len()
        ));
    }

    if let Some(bad) = keywords.iter().find(|k| !is_valid_keyword(k)) {
        warnings.push(format!("Keyword “{bad}” is not a single alphabetic word"));
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 50 unique alphabetic two-letter words, so the normaliser keeps all of them.
    fn unique_words(count: usize) -> Vec<String> {
        (0..count)
            .map(|i| {
                let first = (b'a' + (i / 26) as u8) as char;
                let second = (b'a' + (i % 26) as u8) as char;
                format!("{first}{second}")
            })
            .collect()
    }

    fn fifty_keywords() -> String {
        unique_words(KEYWORD_TARGET).join(", ")
    }

    #[test]
    fn parses_markdown_formatted_reply() {
        let keywords = fifty_keywords();
        let reply = format!(
            "**Title:** Sunrise Over Golden Gate\n\
             **Editorial Status:** Commercial\n\
             **Caption:** Early morning light hits the Golden Gate Bridge as fog drifts across the bay.\n\
             **Keywords:** bridge, san, francisco, sunrise, fog, {keywords}"
        );
        let parsed = parse_response(&reply).expect("reply should parse");
        assert_eq!(parsed.title, "Sunrise Over Golden Gate");
        assert!(!parsed.editorial);
        assert!(parsed.caption.starts_with("Early morning light"));
        assert_eq!(
            parsed.keywords.len(),
            KEYWORD_TARGET,
            "duplicates are removed while parsing"
        );
        assert!(parsed.keywords.iter().all(|k| k.chars().all(char::is_alphabetic)));
    }

    #[test]
    fn parses_single_line_reply_and_editorial_flag() {
        let reply = "Title: Man In Orange Shirt Editorial Status: Editorial Caption: AMSTERDAM, NETHERLANDS – JANUARY 2, 2026: A man in an orange shirt celebrates on a canal boat. Keywords: people, orange, festival, canal, boat, netherlands, celebration, crowd, party, tourism, jacket, hat, water, city, summer, happy, dutch, amsterdam, holiday, event, outdoor, street, joy, culture, weekend, sunny, fun, colorful, tradition, local, travel, portrait, candid, daylight, urban, leisure, group, friendship, lifestyle, europe, parade, costume, music, dance, cheers, orange, bright, vivid, energetic, warm";
        let parsed = parse_response(reply).expect("single line reply should parse");
        assert!(parsed.editorial);
        assert_eq!(parsed.caption.chars().count(), reply.split("Caption: ").nth(1).unwrap().split(" Keywords:").next().unwrap().chars().count());
        // duplicate "orange" is removed
        assert_eq!(parsed.keywords.iter().filter(|k| k.eq_ignore_ascii_case("orange")).count(), 1);
    }

    #[test]
    fn normalization_strips_numbers_punctuation_and_duplicates() {
        let keywords = normalize_keywords("Sunset, 2024, new york, Sunset, blue-sky, a, 3d, mountain!");
        // Numbers and punctuation are removed ("blue-sky" becomes one word), duplicates
        // and single-letter fragments are dropped, multi-word phrases are split.
        assert_eq!(
            keywords,
            vec!["Sunset", "new", "york", "bluesky", "mountain"]
        );
    }

    #[test]
    fn caps_keywords_at_fifty() {
        let raw = unique_words(80).join(", ");
        assert_eq!(normalize_keywords(&raw).len(), KEYWORD_TARGET);
    }

    #[test]
    fn validation_reports_constraints() {
        let keywords = vec!["short".to_string()];
        let warnings = validate("Two words", "too short", &keywords);
        assert_eq!(warnings.len(), 3);
        assert!(warnings.iter().any(|w| w.contains("Title has 2 words")));
        assert!(warnings.iter().any(|w| w.contains("Caption is 9 characters")));
        assert!(warnings.iter().any(|w| w.contains("1 keywords")));
    }

    #[test]
    fn system_prompt_substitutes_date_and_appends_extra() {
        let prompt = system_prompt("November 5, 2026", "Prefer British spelling.");
        assert!(prompt.contains("November 5, 2026"));
        assert!(prompt.contains("Prefer British spelling."));
        assert!(!prompt.contains("{DATE}"));
        assert!(prompt.contains("Exactly 50 keywords."));
    }

    #[test]
    fn missing_fields_are_reported() {
        let err = parse_response("Title: Only a title").unwrap_err().to_string();
        assert!(err.contains("Caption"));
        assert!(err.contains("Keywords"));
    }

    #[test]
    fn empty_reply_reports_all_missing_fields() {
        let err = parse_response("").unwrap_err().to_string();
        assert!(err.contains("Title"), "got: {err}");
        assert!(err.contains("Caption"), "got: {err}");
        assert!(err.contains("Keywords"), "got: {err}");
    }

    #[test]
    fn user_prompt_includes_context_only_when_present() {
        assert!(user_prompt(None, None).contains("Analyze the attached image"));
        assert!(!user_prompt(None, None).contains("Situational context"));
        assert!(!user_prompt(Some("   "), None).contains("Situational context"));

        let prompt = user_prompt(Some("  King's Day in Amsterdam  "), None);
        assert!(prompt.contains("King's Day in Amsterdam"));
        assert!(!prompt.contains("  King's Day"));
        assert!(prompt.contains("ground truth"));
    }

    #[test]
    fn user_prompt_includes_existing_file_metadata() {
        let existing = ExistingMetadata {
            title: Some("Canal Sunset".to_string()),
            caption: Some("Warm light over the Amsterdam canal.".to_string()),
            keywords: vec!["canal".to_string(), "sunset".to_string()],
            gps: Some("52 deg 22' 12.00\" N, 4 deg 53' 24.00\" E".to_string()),
            date: Some("2026:04:19 20:14:03".to_string()),
            dims: Some((6000, 4000)),
        };
        let prompt = user_prompt(Some("King's Day in Amsterdam"), Some(&existing));
        assert!(prompt.contains("GPS location: 52 deg 22' 12.00\" N, 4 deg 53' 24.00\" E"));
        assert!(prompt.contains("Capture date: 2026:04:19 20:14:03"));
        assert!(prompt.contains("Title: Canal Sunset"));
        assert!(prompt.contains("Caption: Warm light over the Amsterdam canal."));
        assert!(prompt.contains("Keywords: canal, sunset"));
        assert!(prompt.contains("King's Day in Amsterdam"));
        assert!(prompt.contains("replace these values"));
    }

    #[test]
    fn system_prompt_omits_extra_section_when_blank() {
        let prompt = system_prompt("November 5, 2026", "   \n  ");
        assert!(!prompt.contains("Additional instructions"));
        assert!(prompt.contains("Shutterstock standard format"));
        assert!(prompt.contains("November 5, 2026"));
    }

    #[test]
    fn parses_crlf_code_fences_and_trailing_markdown() {
        let reply = "```\r\n**Title:** Foggy Bridge Sunrise\r\n**Editorial Status:** Commercial\r\n**Caption:** Fog drifts under a red suspension bridge at sunrise near the bay.\r\n**Keywords:** bridge, fog, sunrise\r\n```";
        let parsed = parse_response(reply).expect("parse");
        assert_eq!(parsed.title, "Foggy Bridge Sunrise");
        assert_eq!(parsed.keywords, vec!["bridge", "fog", "sunrise"]);
        assert!(!parsed.editorial);
        assert_eq!(
            parsed.caption,
            "Fog drifts under a red suspension bridge at sunrise near the bay."
        );
    }

    #[test]
    fn parses_numbered_labels() {
        let reply = "1. Title: Harbour At Dusk\n2. Editorial Status: Commercial\n3. Caption: Fishing boats return to the harbour as the last light fades over the water.\n4. Keywords: harbour, boats, dusk";
        let parsed = parse_response(reply).expect("parse");
        assert_eq!(parsed.title, "Harbour At Dusk");
        assert!(parsed.caption.starts_with("Fishing boats return"));
        assert_eq!(parsed.keywords, vec!["harbour", "boats", "dusk"]);
    }

    #[test]
    fn editorial_status_can_be_explicit_commercial_or_uppercase_editorial() {
        let base = "Title: Three Word Title\nCaption: A factual sentence about the scene with enough characters to look real.\nKeywords: one, two, three\n";

        let parsed = parse_response(&format!("{base}Editorial Status: Not editorial - commercial use only"))
            .expect("parse");
        assert!(!parsed.editorial, "explicit commercial must win");

        let parsed = parse_response(&format!("{base}Editorial Status: EDITORIAL"))
            .expect("parse");
        assert!(parsed.editorial);
    }

    #[test]
    fn title_quotes_and_trailing_period_are_cleaned() {
        let reply = "Title: \"Sunset Over Bay.\"\nCaption: A factual sentence about the scene with enough characters to look real.\nKeywords: sunset, bay, water\nEditorial Status: Commercial";
        let parsed = parse_response(reply).expect("parse");
        assert_eq!(parsed.title, "Sunset Over Bay");
    }

    #[test]
    fn caption_and_keywords_can_span_multiple_lines() {
        let reply = "Title: Storm Over City\nEditorial Status: Commercial\nCaption: Heavy rain clouds gather over the city skyline\nwhile the last sunlight lights the towers.\nKeywords: storm, rain, city,\nskyline, clouds, sunset";
        let parsed = parse_response(reply).expect("parse");
        assert!(!parsed.caption.contains('\n'));
        assert!(parsed.caption.contains("city skyline while"));
        assert_eq!(
            parsed.keywords,
            vec!["storm", "rain", "city", "skyline", "clouds", "sunset"]
        );
    }

    #[test]
    fn duplicate_labels_keep_the_last_value() {
        let reply = "Title: First Title\nTitle: Second Title\nEditorial Status: Commercial\nCaption: A factual sentence about the scene with enough characters to look real.\nKeywords: one, two";
        let parsed = parse_response(reply).expect("parse");
        assert_eq!(parsed.title, "Second Title");
    }

    #[test]
    fn full_width_colon_is_accepted() {
        let reply = "Title：Sunset Over Bay\nEditorial Status：Commercial\nCaption：A factual sentence about the scene with enough characters to look real.\nKeywords：sunset, bay, water";
        let parsed = parse_response(reply).expect("parse");
        assert_eq!(parsed.title, "Sunset Over Bay");
        assert_eq!(parsed.keywords, vec!["sunset", "bay", "water"]);
    }

    #[test]
    fn normalisation_handles_empty_unicode_and_separators() {
        assert!(normalize_keywords("").is_empty());
        assert!(normalize_keywords(" , ; | \n ").is_empty());
        assert_eq!(
            normalize_keywords("Café, naïve, 北京"),
            vec!["Café", "naïve", "北京"]
        );
        assert_eq!(
            normalize_keywords("aa, bb; cc|dd\nee/ff"),
            vec!["aa", "bb", "cc", "dd", "ee", "ff"]
        );
    }

    #[test]
    fn normalisation_drops_single_letters_numbers_and_symbols() {
        let keywords = normalize_keywords("a, I, 1, 42, $, %, ab, a1, 3d, x-ray");
        assert_eq!(keywords, vec!["ab", "xray"]);
        assert!(keywords.iter().all(|keyword| is_valid_keyword(keyword)));
    }

    #[test]
    fn keyword_validation_checks_one_alphabetic_word() {
        assert!(!is_valid_keyword("A"));
        assert!(!is_valid_keyword(""));
        assert!(!is_valid_keyword("two words"));
        assert!(!is_valid_keyword("abc1"));
        assert!(is_valid_keyword("AB"));
        assert!(is_valid_keyword("Café"));
        assert!(is_valid_keyword("北京"));
    }

    #[test]
    fn valid_metadata_produces_no_warnings() {
        let caption = "a".repeat(50);
        let warnings = validate("Three Word Title", &caption, &unique_words(KEYWORD_TARGET));
        assert!(warnings.is_empty(), "got: {warnings:?}");
    }

    #[test]
    fn caption_boundaries_are_inclusive() {
        let keywords = unique_words(KEYWORD_TARGET);
        let caption_warnings = |length: usize| {
            validate("Three Word Title", &"a".repeat(length), &keywords)
                .into_iter()
                .filter(|warning| warning.contains("Caption"))
                .count()
        };

        assert_eq!(caption_warnings(49), 1);
        assert_eq!(caption_warnings(50), 0);
        assert_eq!(caption_warnings(180), 0);
        assert_eq!(caption_warnings(181), 1);
    }

    #[test]
    fn validation_flags_invalid_keyword_and_counts() {
        let mut keywords = unique_words(KEYWORD_TARGET);
        keywords[0] = "two words".to_string();
        let warnings = validate("Three Word Title", &"a".repeat(80), &keywords);
        assert_eq!(warnings.len(), 1, "got: {warnings:?}");
        assert!(warnings[0].contains("not a single alphabetic word"));

        let warnings = validate("Three Word Title", &"a".repeat(80), &unique_words(49));
        assert!(warnings.iter().any(|warning| warning.contains("49 keywords")));
    }

    #[test]
    fn parser_keeps_editorial_shutterstock_caption_format() {
        let caption = "AMSTERDAM, NETHERLANDS – JANUARY 2, 2026: A man in an orange shirt celebrates on a canal boat.";
        let reply = format!("Title: Orange Canal Celebration\nEditorial Status: Editorial\nCaption: {caption}\nKeywords: people, orange, canal, celebration");
        let parsed = parse_response(&reply).expect("parse");
        assert!(parsed.editorial);
        assert_eq!(parsed.caption, caption);
        assert!(parsed.caption.contains('–'));
    }

    #[test]
    fn singular_and_plural_duplicates_keep_the_singular() {
        assert_eq!(
            normalize_keywords("apple, apples, banana"),
            vec!["apple", "banana"]
        );
        assert_eq!(normalize_keywords("apples, apple"), vec!["apple"]);
        assert_eq!(
            normalize_keywords("cities, city, boxes, box, knives, knife"),
            vec!["city", "box", "knife"]
        );
        assert_eq!(
            normalize_keywords("apples, bananas"),
            vec!["apples", "bananas"],
            "a plural on its own is valid"
        );
        assert_eq!(
            normalize_keywords("series, sery"),
            vec!["series", "sery"],
            "invariant plurals are not rewritten"
        );
    }

    #[test]
    fn target_controls_prompt_and_trimming() {
        let raw = unique_words(60).join(", ");
        assert_eq!(normalize_keywords_with_target(&raw, 37).len(), 37);

        let prompt = system_prompt_with_constraints(
            "November 6, 2026",
            "",
            37,
            (4, 8),
            (70, 220),
        );
        assert!(prompt.contains("Exactly 37 keywords."));
        assert!(prompt.contains("Between 4 and 8 words."));
        assert!(prompt.contains("70 to 220 characters"));
        assert!(prompt.contains("Title: [4-8 words]"));
        assert!(prompt.contains("Caption: [70-220 characters]"));
        assert!(!prompt.contains("{TITLE_MIN_WORDS}"));
        assert!(!prompt.contains("{CAPTION_MAX_CHARS}"));
        assert!(prompt.contains("word37"));
        assert!(!prompt.contains("word50"));
    }

    #[test]
    fn retry_prompt_records_every_missed_constraint_and_previous_terms() {
        let previous = GeneratedMetadata {
            title: "Too Many Words In This Title".to_string(),
            caption: "short".to_string(),
            keywords: vec!["apple".to_string()],
            editorial: false,
        };
        let prompt = retry_prompt(None, None, 5, (3, 5), (50, 180), &previous);
        assert!(prompt.contains("Only 1 usable keyword was returned"));
        assert!(prompt.contains("exactly 5"));
        assert!(prompt.contains("apple"));
        assert!(prompt.contains("keep the singular"));
        assert!(prompt.contains("title has 6 words"));
        assert!(prompt.contains("must have 3–5"));
        assert!(prompt.contains("caption has 5 characters"));
        assert!(prompt.contains("must have 50–180 characters"));
        assert!(prompt.contains("Rewrite the title"));
        assert!(prompt.contains("Rewrite the caption"));
    }

    #[test]
    fn custom_ranges_drive_validation() {
        let keywords = unique_words(5);
        let warnings = validate_with_constraints(
            "One Two Three Four",
            &"a".repeat(70),
            &keywords,
            5,
            (2, 3),
            (60, 80),
        );
        assert_eq!(warnings.len(), 1, "got: {warnings:?}");
        assert!(warnings[0].contains("Title has 4 words (needs 2–3)"));

        let warnings = validate_with_constraints(
            "One Two",
            &"a".repeat(81),
            &keywords,
            5,
            (2, 3),
            (60, 80),
        );
        assert_eq!(warnings.len(), 1, "got: {warnings:?}");
        assert!(warnings[0].contains("Caption is 81 characters (needs 60–80)"));
    }
}
