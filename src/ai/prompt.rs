use crate::config::{
    clamp_caption_char_range, clamp_keyword_accept_percent, clamp_keyword_target,
    clamp_title_word_range, KEYWORD_TARGET_DEFAULT,
};
#[cfg(test)]
use crate::config::{
    CAPTION_CHARS_DEFAULT_MAX, CAPTION_CHARS_DEFAULT_MIN, TITLE_WORDS_DEFAULT_MAX,
    TITLE_WORDS_DEFAULT_MIN,
};
use crate::metadata::ExistingMetadata;
use anyhow::{bail, Result};
use chrono::{Datelike, NaiveDate};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

pub const KEYWORD_TARGET: usize = KEYWORD_TARGET_DEFAULT;

/// Minimum usable keyword count accepted for a target at the configured
/// percentage, rounded up. A reply at or above this floor is saved without a
/// retry; a shorter reply is retried and then treated as a failed generation.
pub fn minimum_keyword_count(keyword_target: usize, keyword_accept_percent: usize) -> usize {
    let target = clamp_keyword_target(keyword_target);
    let percent = clamp_keyword_accept_percent(keyword_accept_percent);
    (target * percent + 99) / 100
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeneratedMetadata {
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
    pub editorial: bool,
    /// One short stock category chosen from the visible subject matter.
    #[serde(default)]
    pub category: String,
}

const SYSTEM_TEMPLATE: &str = r#"Role & Task:
You are an expert photography analyst specializing in metadata tag generation, keywording, and stock image description. Analyze the provided image and generate accurate titles, captions, and keywords following strict guidelines.

Accuracy and safety rules:
1. Ground every claim in the image or supplied data.
   - Name a landmark, building, city, country, species, brand, person, or event only when it is clearly identifiable from visible evidence or supplied GPS/context.
   - Never guess a location, date, event, name, brand, species, or release status. If uncertain, use a generic description and leave the detail out.
   - Do not include trademarks, brand names, product names, real people's names, personal information, URLs, email addresses, phone numbers, or hashtags in commercial metadata.
2. Commercial vs. Editorial Status:
   - Mark EDITORIAL when the image contains a recognizable face, logo/trademark, private property, minor, or newsworthy/sensitive event, unless the supplied release status says commercial use is cleared.
   - For documentary editorial content, open the caption with the place and the supplied date, then a colon and one factual sentence:
     City, Country - Month Day, Year: factual sentence about the subject.
   - Fill that prefix from this image, not from the template. A correctly filled example is: Hong Kong, China - {DATE}: A tram passes a crowded market street. The place must be the supplied GPS/context location or one clearly identifiable in the image.
   - Never output City, Country, Month Day, or Year as literal placeholders, and do not reuse the example's place or sentence unless the image shows it. If no place is known, start with the factual sentence and leave the prefix out.
   - Use the supplied editorial date only. It must be the date the image was taken. If the supplied date is not a capture date, do not invent one; leave the date out.
   - The editorial location must come from supplied GPS/context. If no location is supplied, do not invent one; describe the scene without it.

Editorial date supplied for this request: {DATE}

Hard rules for all five lines (one violation invalidates the reply):
- English only: every word in plain English letters, ASCII only, no accents or non-English characters (write "cafe", not "café").
- Plain text only: no markdown, no quotation marks or backticks around values, no emoji, no hashtags, and no symbols such as &, /, |, @, #, %, +, =, (, ), [, ], {, }.
- No URLs, email addresses, phone numbers, prices, or calls to action, and no promotional wording.
- One value per labelled line, with nothing before, between, or after the five lines.

Output Formatting Constraints:

1. Title:
   - Length: Between {TITLE_MIN_WORDS} and {TITLE_MAX_WORDS} words; aim for exactly {TITLE_AIM_WORDS} words. Count the words before replying.
   - Content: A complete descriptive sentence about the primary subject, not a comma-separated keyword list.
   - Language: English letters only. No accented or non-English letters.
   - Characters: letters and single spaces only. No punctuation at all: no periods, commas, hyphens, apostrophes, colons, digits, or other special characters. Do not end with a period.
   - Do not use promotional language, URLs, contact details, or all caps.

2. Caption:
   - Length: Strict limit of {CAPTION_MIN_CHARS} to {CAPTION_MAX_CHARS} characters including spaces; aim for about {CAPTION_AIM_CHARS} characters. Count every character before replying and stay inside the limit even when the scene is complex.
   - Content: Factual description of the scene (or the editorial structure above).
   - Language: English letters only. No accented or non-English letters.
   - Characters: letters, digits, spaces, commas, and periods only. The editorial location/date prefix may use the single hyphen and the colon shown above; use no other symbols, quotes, hashtags, or emoji.
   - Do not use promotional language, URLs, contact details, hashtags, or keyword stuffing.

3. Keywords:
   - Quantity: Exactly {KEYWORD_TARGET} keywords. Count them before replying; a shorter list is an error even when the scene is simple.
   - Format: Single words only (no multi-word phrases/keyphrases).
   - Separator: Comma-separated.
   - Rules: English letters only (A-Z, a-z). No numbers, punctuation, special characters, or accented letters.
   - Relevance: Only include terms visibly supported by the image. Remove generic filler, title repeats, brands, real-person names, and speculative concepts. When obvious subjects run out, continue with single words for visible colours, materials, textures, light, weather, time of day, season, framing, and mood instead of stock-agency words or filler.
   - Duplicates: Do not include a plural when its singular form is also present. Keep the singular (for example, keep "apple" and omit "apples"). Never repeat a keyword or vary its spelling just to fill the count.

4. Category:
   - One short stock category that best fits the visible subject, in Title Case. Examples: `Nature`, `Travel`, `People`, `Food`, `Business`, `Technology`, `Architecture`, `Animals`.
   - Use one or two plain words, letters and single spaces only. Do not use a sentence, a comma-separated list, a keyword list, or punctuation.

Final check before replying (silently fix any line that fails; do not explain):
- Title has {TITLE_MIN_WORDS}-{TITLE_MAX_WORDS} words, English letters and spaces only.
- Caption has {CAPTION_MIN_CHARS}-{CAPTION_MAX_CHARS} characters including spaces, English only.
- Keywords contain exactly {KEYWORD_TARGET} unique single English words, with no plural paired with its singular.
- No line contains markdown, quotation marks, emoji, URLs, or non-English letters.

Required Output Schema:
Title: [{TITLE_MIN_WORDS}-{TITLE_MAX_WORDS} words]
Editorial Status: [Commercial / Editorial]
Caption: [{CAPTION_MIN_CHARS}-{CAPTION_MAX_CHARS} characters]
Keywords: [word1, word2, word3, ... word{KEYWORD_TARGET}]
Category: [one short stock category]

Reply with the five labelled lines above and nothing else. Do not use markdown."#;

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

#[cfg(test)]
#[test]
fn the_editorial_caption_example_names_a_real_place_and_the_supplied_date() {
    let prompt = system_prompt("November 9, 2026", "");

    assert!(
        prompt.contains(
            "Hong Kong, China - November 9, 2026: A tram passes a crowded market street."
        ),
        "the example must fill the prefix with a real place and the date actually sent"
    );
    assert!(
        prompt.contains("Never output City, Country, Month Day, or Year as literal placeholders")
    );
    assert!(
        !prompt.contains("STATE/COUNTRY") && !prompt.contains("MONTH DAY, YEAR"),
        "placeholder tokens invite the model to copy them verbatim"
    );
    assert!(!prompt.contains("en dash"));
}

/// The editorial caption guidance exactly as it appears in a rendered system
/// prompt, for display in the prompt preview. Extracting it from the rendered
/// text lets the preview reuse the real prompt instead of a hand-kept copy that
/// could drift from it.
pub fn editorial_caption_rule(prompt: &str) -> Vec<String> {
    prompt
        .lines()
        .filter(|line| {
            line.contains("documentary editorial content")
                || line.contains("City, Country - Month Day, Year:")
                || line.contains("correctly filled example")
                || line.contains("literal placeholders")
        })
        .map(|line| line.trim().trim_start_matches("- ").to_string())
        .collect()
}

#[cfg(test)]
#[test]
fn the_prompt_preview_excerpts_the_editorial_caption_rule() {
    let prompt = system_prompt("November 9, 2026", "");
    let rule = editorial_caption_rule(&prompt);

    assert_eq!(rule.len(), 4, "unexpected excerpt: {rule:#?}");
    assert!(rule[0].starts_with("For documentary editorial content"));
    assert!(rule.iter().any(|line| line
        .contains("Hong Kong, China - November 9, 2026: A tram passes a crowded market street.")));
    assert!(
        rule.iter().all(|line| !line.starts_with('-')),
        "list markers are stripped for display: {rule:#?}"
    );
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
    let (caption_min, caption_max) = clamp_caption_char_range(caption_range.0, caption_range.1);
    let title_aim = target_title_words(title_range);
    let caption_aim = target_caption_chars(caption_range);
    let mut prompt = SYSTEM_TEMPLATE
        .replace("{DATE}", date)
        .replace("{KEYWORD_TARGET}", &target.to_string())
        .replace("{TITLE_MIN_WORDS}", &title_min.to_string())
        .replace("{TITLE_MAX_WORDS}", &title_max.to_string())
        .replace("{TITLE_AIM_WORDS}", &title_aim.to_string())
        .replace("{CAPTION_MIN_CHARS}", &caption_min.to_string())
        .replace("{CAPTION_MAX_CHARS}", &caption_max.to_string())
        .replace("{CAPTION_AIM_CHARS}", &caption_aim.to_string());
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
        "Analyze the attached image and return the five labelled fields exactly as specified.",
    );
    prompt
}

/// Words to aim for inside the allowed title range. A range alone lets the model
/// land on the same boundary again, so retries name one exact target.
fn target_title_words(range: (usize, usize)) -> usize {
    let (min, max) = clamp_title_word_range(range.0, range.1);
    min + (max - min) / 2
}

fn target_caption_chars(range: (usize, usize)) -> usize {
    let (min, max) = clamp_caption_char_range(range.0, range.1);
    min + (max - min) / 2
}

/// Show the previous reply verbatim. Without it the model only sees word counts
/// and tends to return the same sentence at low temperature.
fn previous_reply_block(previous: &GeneratedMetadata) -> String {
    let keywords = if previous.keywords.is_empty() {
        "(none usable)".to_string()
    } else {
        previous.keywords.join(", ")
    };
    format!(
        "Your previous reply:\nTitle: {}\nCaption: {}\nKeywords: {keywords}\nCategory: {}\n\n",
        previous.title, previous.caption, previous.category
    )
}

/// Compare two replies ignoring case, punctuation, and spacing, so a retry that
/// reproduces the same sentence is detected even when trimming differs.
pub fn same_text(left: &str, right: &str) -> bool {
    fn normalize(text: &str) -> String {
        text.chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    normalize(left) == normalize(right)
}

/// Second-pass prompt used when the first reply missed the title, caption, or
/// keyword constraints. It repeats the original context and asks for a full
/// reply so the normal five-field parser can be used again.
pub fn retry_prompt(
    context: Option<&str>,
    existing: Option<&ExistingMetadata>,
    keyword_target: usize,
    keyword_accept_percent: usize,
    title_range: (usize, usize),
    caption_range: (usize, usize),
    previous: &GeneratedMetadata,
) -> String {
    let target = clamp_keyword_target(keyword_target);
    let (title_min, title_max) = clamp_title_word_range(title_range.0, title_range.1);
    let (caption_min, caption_max) = clamp_caption_char_range(caption_range.0, caption_range.1);
    let words = title_word_count(&previous.title);
    let chars = caption_char_count(&previous.caption);
    let title_length_ok = (title_min..=title_max).contains(&words);
    let caption_length_ok = (caption_min..=caption_max).contains(&chars);
    let title_english_ok = !contains_non_english_letters(&previous.title);
    let caption_english_ok = !contains_non_english_letters(&previous.caption);
    let title_ok = title_length_ok && title_english_ok;
    let caption_ok = caption_length_ok && caption_english_ok;
    let minimum_keywords = minimum_keyword_count(target, keyword_accept_percent);
    let keywords_short = previous.keywords.len() < minimum_keywords;
    let title_aim = target_title_words(title_range);
    let caption_aim = target_caption_chars(caption_range);
    let keyword_shortfall = minimum_keywords.saturating_sub(previous.keywords.len());

    let mut issues = Vec::new();
    if !title_length_ok {
        issues.push(format!(
            "The title has {words} words; it must have {title_min}–{title_max}."
        ));
    }
    if !title_english_ok {
        issues
            .push("The title contains non-English letters; use English letters only.".to_string());
    }
    if !caption_length_ok {
        issues.push(format!(
            "The caption has {chars} characters; it must have {caption_min}–{caption_max} characters."
        ));
    }
    if !caption_english_ok {
        issues.push(
            "The caption contains non-English letters; use English letters only.".to_string(),
        );
    }
    if keywords_short {
        let verb = if previous.keywords.len() == 1 {
            "keyword was"
        } else {
            "keywords were"
        };
        issues.push(format!(
            "Only {} usable {verb} returned; at least {minimum_keywords} are required (target {target}).",
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
    prompt.push('\n');
    prompt.push_str(&previous_reply_block(previous));
    prompt.push_str(
        "Write a different reply instead of repeating the previous wording. Do not reuse the previous title or caption, and do not start the title with the same word.\n\n",
    );
    prompt.push_str("Return the five labelled fields again. ");
    if title_ok {
        prompt.push_str("Keep the title unchanged. ");
    } else if !title_english_ok && title_length_ok {
        prompt.push_str("Rewrite the title using English letters only. ");
    } else if !title_english_ok {
        prompt.push_str(&format!(
            "Rewrite the title to exactly {title_aim} words ({title_min}–{title_max} allowed) using English letters only. "
        ));
    } else {
        prompt.push_str(&format!(
            "Rewrite the title to exactly {title_aim} words ({title_min}–{title_max} allowed). "
        ));
    }
    if caption_ok {
        prompt.push_str("Keep the caption unchanged. ");
    } else if !caption_english_ok && caption_length_ok {
        prompt.push_str("Rewrite the caption using English letters only. ");
    } else if !caption_english_ok {
        prompt.push_str(&format!(
            "Rewrite the caption to about {caption_aim} characters ({caption_min}–{caption_max} allowed) using English letters only. "
        ));
    } else {
        prompt.push_str(&format!(
            "Rewrite the caption to about {caption_aim} characters ({caption_min}–{caption_max} allowed). "
        ));
    }
    if keywords_short {
        prompt.push_str(&format!(
            "Return at least {keyword_shortfall} additional unique single-word keywords that are not in the previous list, so the total reaches at least {minimum_keywords} (target {target}). The previous list was: {}. ",
            previous.keywords.join(", ")
        ));
    } else {
        prompt.push_str("Keep the keywords unchanged. ");
    }
    prompt.push_str("Do not repeat any keyword, and do not include both a singular and its plural; keep the singular.");
    prompt
}

/// Final prompt shape used when the corrective retry did not fix the reply. It
/// changes the approach rather than repeating the same request: the model must
/// describe the image first and then rewrite the fields from that description.
pub fn escalation_prompt(
    context: Option<&str>,
    existing: Option<&ExistingMetadata>,
    keyword_target: usize,
    keyword_accept_percent: usize,
    title_range: (usize, usize),
    caption_range: (usize, usize),
    previous: &GeneratedMetadata,
    repeated: bool,
) -> String {
    let target = clamp_keyword_target(keyword_target);
    let (title_min, title_max) = clamp_title_word_range(title_range.0, title_range.1);
    let (caption_min, caption_max) = clamp_caption_char_range(caption_range.0, caption_range.1);
    let title_aim = target_title_words(title_range);
    let caption_aim = target_caption_chars(caption_range);
    let minimum_keywords = minimum_keyword_count(target, keyword_accept_percent);
    let chars = caption_char_count(&previous.caption);

    let mut prompt = user_prompt(context, existing);
    prompt.push_str("\n\nThe previous attempt still failed");
    if repeated {
        prompt.push_str(" and repeated the same title and caption");
    }
    prompt.push_str(":\n");
    prompt.push_str(&previous_reply_block(previous));
    prompt.push_str(
        "Do not edit the previous wording. Look at the image again and answer in two steps:\n\
         1. Write one plain sentence beginning with `Observation:` that describes only what is visible.\n\
         2. Then write the five labelled fields on their own lines, built from that observation.\n\n",
    );
    prompt.push_str(&format!(
        "- Title: exactly {title_aim} words ({title_min}–{title_max} allowed), a different sentence from the previous title, English letters only.\n"
    ));
    let caption_advice = if chars > caption_max {
        "the previous caption was too long, so remove detail instead of adding words"
    } else if chars < caption_min {
        "the previous caption was too short, so add visible detail instead of filler adjectives"
    } else {
        "keep the same level of detail"
    };
    prompt.push_str(&format!(
        "- Caption: about {caption_aim} characters ({caption_min}–{caption_max} allowed), {caption_advice}, English letters only.\n"
    ));
    prompt.push_str(&format!(
        "- Keywords: at least {minimum_keywords} unique single-word keywords (target {target}), {} more than the previous list; do not reuse: {}.\n",
        minimum_keywords.saturating_sub(previous.keywords.len()),
        previous.keywords.join(", ")
    ));
    prompt.push_str("- Category: one short stock category from the visible subject.\n");
    prompt.push_str("\n");
    prompt.push_str(
        "Return only the Observation line followed by the five labelled lines. Do not use markdown.",
    );
    prompt
}

/// System instruction for the keywords-only top-up. It replaces the full
/// metadata prompt so a text-only request cannot drift into rewriting the title
/// or caption that already passed.
pub const KEYWORD_TOPUP_SYSTEM: &str = "You extend the keyword list of a stock photo. You are given only text: the accepted title, caption, category and the keywords chosen so far. Reply with exactly one line, `Keywords: ...`, holding new single words that describe what the title and caption describe. Use English letters only, singular nouns and plain adjectives, and no duplicate of an accepted term. Never write a brand, a camera or technical term, or a meta word such as photo, image, picture, stock, background or concept. Never rewrite the title or caption, never explain, and never add a second line.";

/// Prompt used when the only remaining miss is the keyword count. The title and
/// caption have already been accepted, so repeating the whole brief would risk
/// losing them; instead the model is shown what passed and asked for the
/// missing terms alone. Sent without the image, paired with
/// [`KEYWORD_TOPUP_SYSTEM`].
pub fn keyword_topup_prompt(
    context: Option<&str>,
    existing: Option<&ExistingMetadata>,
    keyword_target: usize,
    keyword_accept_percent: usize,
    accepted: &GeneratedMetadata,
) -> String {
    let target = clamp_keyword_target(keyword_target);
    let minimum = minimum_keyword_count(target, keyword_accept_percent);
    let shortfall = minimum.saturating_sub(accepted.keywords.len());
    let mut prompt = user_prompt(context, existing);
    prompt.push_str(
        "\n\nThe title and caption are already accepted. Only the keyword list is short.\n",
    );
    prompt.push_str(&format!("Title: {}\n", accepted.title));
    prompt.push_str(&format!("Caption: {}\n", accepted.caption));
    if !accepted.category.trim().is_empty() {
        prompt.push_str(&format!("Category: {}\n", accepted.category));
    }
    prompt.push_str(&format!(
        "Keywords accepted so far ({} of at least {minimum} required, target {target}): {}\n\n",
        accepted.keywords.len(),
        accepted.keywords.join(", ")
    ));
    prompt.push_str(&format!(
        "Add at least {shortfall} more keywords describing what is visible in the image. Reply with one line only:\nKeywords: <comma separated single words>\n\nRules: English letters only, one word each, singular form, no brand names, no meta words such as photo, image, stock or background, and no term from the accepted list or the title. Do not write the caption again and do not add any other line."
    ));
    prompt
}

/// Read a keywords-only reply. A labelled `Keywords:` line is preferred so
/// surrounding prose cannot leak into the list; a bare list is used as it is.
pub fn parse_keywords_reply(text: &str, keyword_target: usize) -> Vec<String> {
    let fields = parse_fields(text);
    let raw = fields
        .get("keywords")
        .cloned()
        .unwrap_or_else(|| text.to_string());
    normalize_keywords_with_target(&raw, keyword_target)
}

/// Prompt used when a reply could not be parsed at all. It repeats the original
/// request, restates the five labelled lines, and shows the model its previous
/// reply so a formatting mistake can be corrected instead of failing the image.
pub fn unparsable_reply_prompt(base: &str, previous_raw: &str) -> String {
    let mut prompt = String::from(base);
    prompt.push_str(
        "\n\nYour previous reply could not be read because it was missing one or more of the five labelled fields. Reply again with only these lines, each label followed by a colon:\nTitle: ...\nEditorial Status: ...\nCaption: ...\nKeywords: ...\n\nPrevious reply:\n",
    );
    prompt.push_str(&super::snippet(previous_raw));
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
            r"(?i)(?:^|[\s,;])(?:\d+[.)]\s*)?(?:[*#>\-]+\s*)?(Title|Editorial\s+Status|Editorial|Caption|Description|Keywords?|Category|Review|Safety)\s*\**\s*[:\u{FF1A}]",
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
    } else if lower.starts_with("category") {
        "category"
    } else if lower.starts_with("review") || lower.starts_with("safety") {
        "review"
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
        } else if ch.is_control() {
            continue;
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

/// Re-run the normal keyword cleanup (English words, dedupe, singular/plural,
/// target cap) over a list, used by the batch cleanup actions.
pub fn normalize_keyword_list(keywords: &[String], target: usize) -> Vec<String> {
    normalize_keywords_with_target(&keywords.join(", "), target)
}

/// Replace every case-insensitive whole-keyword match, then normalize the list.
pub fn replace_keyword(list: &mut Vec<String>, find: &str, replace: &str, target: usize) {
    let find = find.trim();
    if find.is_empty() {
        return;
    }
    let joined = list
        .iter()
        .map(|keyword| {
            if keyword.eq_ignore_ascii_case(find) {
                replace.trim().to_string()
            } else {
                keyword.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    *list = normalize_keywords_with_target(&joined, target);
}

/// Remove case-insensitive duplicate keywords while preserving the first
/// spelling and the original order. Blank entries are dropped. Unlike the full
/// normalizer, this leaves valid terms, casing and singular/plural pairs alone.
pub fn dedupe_keywords(list: &mut Vec<String>) -> usize {
    let before = list.len();
    let mut seen = HashSet::new();
    list.retain_mut(|keyword| {
        let trimmed = keyword.trim().to_string();
        if trimmed.is_empty() {
            return false;
        }
        let key = trimmed.to_lowercase();
        *keyword = trimmed;
        seen.insert(key)
    });
    before - list.len()
}

pub fn sort_keywords(list: &mut [String]) {
    list.sort_by_key(|keyword| keyword.to_lowercase());
}

pub fn parse_response_with_target(text: &str, keyword_target: usize) -> Result<GeneratedMetadata> {
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

/// Keep a category to a short single-line label: no trailing punctuation and a
/// hard cap so a chatty model cannot fill the metadata field with a sentence.
fn normalize_category(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| c == '.' || c == ',' || c == ';' || c == '*' || c == '`')
        .trim();
    trimmed
        .chars()
        .take(60)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Reduce a title to plain words: punctuation is removed, a token that carried
/// a digit is dropped whole, and runs of whitespace collapse to one space.
///
/// Apostrophes are deleted so `Children's` becomes `Childrens` instead of
/// splitting into two words, while other punctuation becomes a word break.
/// Letters outside English are deliberately kept, so the existing non-English
/// check can still reject the reply instead of this quietly repairing it.
pub fn normalize_title(title: &str) -> String {
    let mut words = Vec::new();
    for token in title.split_whitespace() {
        // Drop the token whole: stripping the digit from `3rd` would leave the
        // meaningless fragment `rd`, which is worse than losing the token.
        if token.chars().any(|ch| ch.is_numeric()) {
            continue;
        }
        let mut cleaned = String::with_capacity(token.len());
        for ch in token.chars() {
            if ch.is_alphabetic() {
                cleaned.push(ch);
            } else if ch == '\'' || ch == '\u{2019}' {
                // Delete rather than break the word.
            } else {
                cleaned.push(' ');
            }
        }
        words.extend(cleaned.split_whitespace().map(str::to_string));
    }
    words.join(" ")
}

fn parse_response_inner(
    text: &str,
    keyword_target: usize,
    require_keywords: bool,
) -> Result<GeneratedMetadata> {
    let target = clamp_keyword_target(keyword_target);
    let fields = parse_fields(text);

    let title = normalize_title(&fields.get("title").cloned().unwrap_or_default());
    let caption = fields.get("caption").cloned().unwrap_or_default();
    let keyword_raw = fields.get("keywords").cloned().unwrap_or_default();
    let editorial_raw = fields
        .get("editorial")
        .cloned()
        .unwrap_or_else(|| "Commercial".to_string());
    let category = normalize_category(&fields.get("category").cloned().unwrap_or_default());

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
    let editorial =
        !editorial_status.contains("commercial") && editorial_status.contains("editorial");

    Ok(GeneratedMetadata {
        title,
        caption,
        keywords,
        editorial,
        category,
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
const KEYWORD_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "had", "has", "have", "her",
    "his", "i", "in", "into", "is", "it", "its", "of", "on", "onto", "or", "our", "over", "that",
    "the", "their", "they", "this", "to", "under", "was", "we", "were", "with", "you", "your",
];

const GENERIC_KEYWORDS: &[&str] = &[
    "adobe",
    "alamy",
    "background",
    "concept",
    "copy",
    "depositphotos",
    "dreamstime",
    "getty",
    "image",
    "istock",
    "photo",
    "photograph",
    "photography",
    "picture",
    "pond5",
    "shutterstock",
    "space",
    "stock",
];

fn is_noise_keyword(keyword: &str) -> bool {
    let lower = keyword.to_ascii_lowercase();
    KEYWORD_STOPWORDS.contains(&lower.as_str()) || GENERIC_KEYWORDS.contains(&lower.as_str())
}

/// Convert common EXIF capture-date forms into the editorial caption date.
pub fn format_capture_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let prefix: String = raw.chars().take(10).collect();
    for pattern in ["%Y:%m:%d", "%Y-%m-%d", "%Y/%m/%d"] {
        if let Ok(date) = NaiveDate::parse_from_str(&prefix, pattern) {
            return Some(format!(
                "{} {}, {}",
                date.format("%B"),
                date.day(),
                date.year()
            ));
        }
    }
    for pattern in ["%B %d, %Y", "%b %d, %Y", "%B %e, %Y"] {
        if let Ok(date) = NaiveDate::parse_from_str(raw, pattern) {
            return Some(format!(
                "{} {}, {}",
                date.format("%B"),
                date.day(),
                date.year()
            ));
        }
    }
    None
}

pub fn normalize_keywords_with_target(raw: &str, keyword_target: usize) -> Vec<String> {
    let target = clamp_keyword_target(keyword_target);
    let mut keywords: Vec<String> = Vec::with_capacity(target);
    let mut seen = HashSet::new();

    for chunk in
        raw.split(|c: char| c == ',' || c == ';' || c == '\n' || c == '\r' || c == '|' || c == '/')
    {
        for word in chunk.split_whitespace() {
            // Drop the whole keyword rather than truncating an accented or
            // non-English word into a misleading ASCII fragment.
            if word.chars().any(|c| !c.is_ascii()) {
                continue;
            }
            let cleaned: String = word.chars().filter(|c| c.is_ascii_alphabetic()).collect();
            if cleaned.chars().count() < 2 || is_noise_keyword(&cleaned) {
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

/// True when text contains letters outside the English ASCII alphabet.
/// ASCII digits, punctuation and whitespace are allowed; accented letters,
/// Cyrillic, CJK and other non-English letters are not.
pub fn contains_non_english_letters(text: &str) -> bool {
    text.chars()
        .any(|ch| ch.is_alphabetic() && !ch.is_ascii_alphabetic())
}

pub fn is_valid_keyword(word: &str) -> bool {
    word.chars().count() >= 2 && word.chars().all(|c| c.is_ascii_alphabetic())
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
        crate::config::KEYWORD_ACCEPT_DEFAULT,
        (TITLE_WORDS_DEFAULT_MIN, TITLE_WORDS_DEFAULT_MAX),
        (CAPTION_CHARS_DEFAULT_MIN, CAPTION_CHARS_DEFAULT_MAX),
    )
}

pub fn validate_with_constraints(
    title: &str,
    caption: &str,
    keywords: &[String],
    keyword_target: usize,
    keyword_accept_percent: usize,
    title_range: (usize, usize),
    caption_range: (usize, usize),
) -> Vec<String> {
    let target = clamp_keyword_target(keyword_target);
    let (title_min, title_max) = clamp_title_word_range(title_range.0, title_range.1);
    let (caption_min, caption_max) = clamp_caption_char_range(caption_range.0, caption_range.1);
    let mut warnings = Vec::new();

    let words = title_word_count(title);
    if !(title_min..=title_max).contains(&words) {
        warnings.push(format!(
            "Title has {words} words (needs {title_min}–{title_max})"
        ));
    }

    if contains_non_english_letters(title) {
        warnings.push("Title contains non-English characters".to_string());
    }

    let chars = caption_char_count(caption);
    if !(caption_min..=caption_max).contains(&chars) {
        warnings.push(format!(
            "Caption is {chars} characters (needs {caption_min}–{caption_max})"
        ));
    }

    if contains_non_english_letters(caption) {
        warnings.push("Caption contains non-English characters".to_string());
    }

    let minimum_keywords = minimum_keyword_count(target, keyword_accept_percent);
    if keywords.len() < minimum_keywords {
        warnings.push(format!(
            "{} keywords (needs at least {minimum_keywords} of {target})",
            keywords.len()
        ));
    }

    if let Some(bad) = keywords.iter().find(|k| !is_valid_keyword(k)) {
        warnings.push(format!("Keyword “{bad}” is not a single English word"));
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
        assert!(parsed
            .keywords
            .iter()
            .all(|k| k.chars().all(char::is_alphabetic)));
    }

    #[test]
    fn parses_single_line_reply_and_editorial_flag() {
        let reply = "Title: Man In Orange Shirt Editorial Status: Editorial Caption: AMSTERDAM, NETHERLANDS – JANUARY 2, 2026: A man in an orange shirt celebrates on a canal boat. Keywords: people, orange, festival, canal, boat, netherlands, celebration, crowd, party, tourism, jacket, hat, water, city, summer, happy, dutch, amsterdam, holiday, event, outdoor, street, joy, culture, weekend, sunny, fun, colorful, tradition, local, travel, portrait, candid, daylight, urban, leisure, group, friendship, lifestyle, europe, parade, costume, music, dance, cheers, orange, bright, vivid, energetic, warm";
        let parsed = parse_response(reply).expect("single line reply should parse");
        assert!(parsed.editorial);
        assert_eq!(
            parsed.caption.chars().count(),
            reply
                .split("Caption: ")
                .nth(1)
                .unwrap()
                .split(" Keywords:")
                .next()
                .unwrap()
                .chars()
                .count()
        );
        // duplicate "orange" is removed
        assert_eq!(
            parsed
                .keywords
                .iter()
                .filter(|k| k.eq_ignore_ascii_case("orange"))
                .count(),
            1
        );
    }

    #[test]
    fn non_english_keywords_are_removed_and_text_is_flagged() {
        assert_eq!(
            normalize_keywords("Sunset, café, 東京, naïve, blue, cafe"),
            vec!["Sunset", "blue", "cafe"]
        );
        assert!(contains_non_english_letters("Café at dawn"));
        assert!(contains_non_english_letters("東京"));
        assert!(!contains_non_english_letters("Cafe - dawn, 2026"));
        assert!(!is_valid_keyword("café"));
        assert!(is_valid_keyword("cafe"));

        let keywords = unique_words(KEYWORD_TARGET);
        assert!(validate("Café at dawn", &"a".repeat(50), &keywords)
            .iter()
            .any(|warning| warning.contains("Title contains non-English")));
        assert!(validate(
            "Three Word Title",
            &format!("café {}", "a".repeat(50)),
            &keywords
        )
        .iter()
        .any(|warning| warning.contains("Caption contains non-English")));
    }

    #[test]
    fn a_stray_review_line_is_ignored() {
        let reply = "Title: London Street Scene\nEditorial Status: Editorial\nCaption: London, UK – April 19, 2026: A factual street scene with people walking past shops.\nKeywords: apple, banana, cherry\nCategory: Travel\nReview: face; uncertain-location";
        let parsed = parse_response(reply).expect("parse a reply with a stray review line");
        assert_eq!(parsed.title, "London Street Scene");
        assert!(parsed.editorial);
        assert_eq!(parsed.keywords, vec!["apple", "banana", "cherry"]);
        assert_eq!(parsed.category, "Travel");
        assert_eq!(
            format_capture_date("2026:04:19 20:14:03"),
            Some("April 19, 2026".to_string())
        );
        assert_eq!(format_capture_date("not a date"), None);
        assert_eq!(
            normalize_keywords("the, photo, sunset, image, mountain"),
            vec!["sunset", "mountain"]
        );
    }

    #[test]
    fn normalization_strips_numbers_punctuation_and_duplicates() {
        let keywords =
            normalize_keywords("Sunset, 2024, new york, Sunset, blue-sky, a, 3d, mountain!");
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
        assert!(warnings
            .iter()
            .any(|w| w.contains("Caption is 9 characters")));
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
        let err = parse_response("Title: Only a title")
            .unwrap_err()
            .to_string();
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
            category: Some("Travel".to_string()),
            ..ExistingMetadata::default()
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
        assert!(prompt.contains("open the caption with the place and the supplied date"));
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

        let parsed = parse_response(&format!(
            "{base}Editorial Status: Not editorial - commercial use only"
        ))
        .expect("parse");
        assert!(!parsed.editorial, "explicit commercial must win");

        let parsed = parse_response(&format!("{base}Editorial Status: EDITORIAL")).expect("parse");
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
        assert!(normalize_keywords("Café, naïve, 北京").is_empty());
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
        assert!(!is_valid_keyword("Café"));
        assert!(!is_valid_keyword("北京"));
        assert!(is_valid_keyword("AB"));
        assert!(is_valid_keyword("Cafe"));
    }

    #[test]
    fn keyword_floor_uses_the_configured_percentage() {
        assert_eq!(minimum_keyword_count(0, 80), 1);
        assert_eq!(minimum_keyword_count(1, 80), 1);
        assert_eq!(minimum_keyword_count(3, 80), 3);
        assert_eq!(minimum_keyword_count(10, 80), 8);
        assert_eq!(minimum_keyword_count(11, 80), 9);
        assert_eq!(minimum_keyword_count(49, 80), 40);
        assert_eq!(minimum_keyword_count(50, 80), 40);
        assert_eq!(minimum_keyword_count(10, 50), 5);
        assert_eq!(minimum_keyword_count(10, 100), 10);
        assert_eq!(minimum_keyword_count(3, 50), 2);
    }

    #[test]
    fn keyword_validation_accepts_the_configured_percentage() {
        let validate_count = |count, percent| {
            validate_with_constraints(
                "Three Word Title",
                &"a".repeat(80),
                &unique_words(count),
                10,
                percent,
                (3, 5),
                (50, 180),
            )
        };

        assert!(validate_count(8, 80).is_empty(), "8 of 10 is 80%");
        assert!(validate_count(7, 80)
            .iter()
            .any(|warning| warning.contains("needs at least 8 of 10")));
        assert!(validate_count(5, 50).is_empty(), "5 of 10 is 50%");
        assert!(validate_count(9, 100)
            .iter()
            .any(|warning| warning.contains("needs at least 10 of 10")));
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
        assert!(warnings[0].contains("not a single English word"));

        let warnings = validate("Three Word Title", &"a".repeat(80), &unique_words(39));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("39 keywords")));
    }

    #[test]
    fn keyword_cleanup_replaces_terms_and_sorts() {
        let mut list = vec!["zebra".to_string(), "Car".to_string(), "apple".to_string()];
        replace_keyword(&mut list, "car", "vehicle", 50);
        assert!(list.iter().any(|keyword| keyword == "vehicle"));
        assert!(!list
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case("car")));
        sort_keywords(&mut list);
        assert_eq!(list, vec!["apple", "vehicle", "zebra"]);
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
    fn duplicate_removal_preserves_first_spelling_and_order() {
        let mut keywords = vec![
            "Sunset".to_string(),
            "mountain".to_string(),
            "sunset".to_string(),
            "  MOUNTAIN ".to_string(),
            "blue".to_string(),
            String::new(),
        ];
        assert_eq!(dedupe_keywords(&mut keywords), 3);
        assert_eq!(keywords, vec!["Sunset", "mountain", "blue"]);

        let mut unique = vec!["apple".to_string(), "apples".to_string()];
        assert_eq!(dedupe_keywords(&mut unique), 0);
        assert_eq!(
            unique,
            vec!["apple", "apples"],
            "dedupe must not merge singular/plural pairs"
        );
    }

    #[test]
    fn target_controls_prompt_and_trimming() {
        let raw = unique_words(60).join(", ");
        assert_eq!(normalize_keywords_with_target(&raw, 37).len(), 37);

        let prompt = system_prompt_with_constraints("November 6, 2026", "", 37, (4, 8), (70, 220));
        assert!(prompt.contains("Exactly 37 keywords."));
        assert!(prompt.contains("Between 4 and 8 words; aim for exactly 6 words."));
        assert!(prompt.contains("70 to 220 characters"));
        assert!(prompt.contains("Title: [4-8 words]"));
        assert!(prompt.contains("Caption: [70-220 characters]"));
        assert!(!prompt.contains("{TITLE_MIN_WORDS}"));
        assert!(!prompt.contains("{CAPTION_MAX_CHARS}"));
        assert!(prompt.contains("word37"));
        assert!(!prompt.contains("word50"));
        assert!(prompt.contains("aim for exactly 6 words"));
        assert!(prompt.contains("aim for about 145 characters"));
        assert!(prompt.contains("Title has 4-8 words, English letters and spaces only"));
        assert!(!prompt.contains("{TITLE_AIM_WORDS}"));
        assert!(!prompt.contains("{CAPTION_AIM_CHARS}"));
    }

    #[test]
    #[test]
    fn titles_lose_stray_punctuation_and_digits() {
        assert_eq!(normalize_title("Golden Gate Bridge."), "Golden Gate Bridge");
        assert_eq!(
            normalize_title("\"Sunset\" over 2024 harbor!"),
            "Sunset over harbor"
        );
        assert_eq!(
            normalize_title("Children's Park \u{2014} 3rd visit"),
            "Childrens Park visit"
        );
        assert_eq!(
            normalize_title("Sun-Dried Tomatoes (close up)"),
            "Sun Dried Tomatoes close up"
        );
        // Non-English letters stay so the caller can still reject them.
        assert_eq!(normalize_title("Café Terrace"), "Café Terrace");
        assert_eq!(normalize_title("2024"), "");
    }

    fn retry_prompt_records_every_missed_constraint_and_previous_terms() {
        let previous = GeneratedMetadata {
            title: "Too Many Words In This Title".to_string(),
            caption: "short".to_string(),
            keywords: vec!["apple".to_string()],
            editorial: false,
            category: "Nature".to_string(),
        };
        let prompt = retry_prompt(None, None, 5, 80, (3, 5), (50, 180), &previous);
        assert!(prompt.contains("Only 1 usable keyword was returned"));
        assert!(prompt.contains("at least 4 are required"));
        assert!(prompt.contains("target 5"));
        assert!(prompt.contains("apple"));
        assert!(prompt.contains("keep the singular"));
        assert!(prompt.contains("title has 6 words"));
        assert!(prompt.contains("must have 3–5"));
        assert!(prompt.contains("caption has 5 characters"));
        assert!(prompt.contains("must have 50–180 characters"));
        assert!(prompt.contains("Rewrite the title"));
        assert!(prompt.contains("Rewrite the caption"));
        assert!(prompt.contains("Title: Too Many Words In This Title"));
        assert!(prompt.contains("Caption: short"));
        assert!(prompt.contains("exactly 4 words"));
        assert!(prompt.contains("about 115 characters"));
    }

    #[test]
    fn same_text_ignores_case_spacing_and_punctuation() {
        assert!(same_text("A Quiet Street!", "a quiet   street"));
        assert!(!same_text("A Quiet Street", "A Busy Street"));
    }

    #[test]
    fn escalation_prompt_forces_a_fresh_observation() {
        let previous = GeneratedMetadata {
            title: "Tiny Title".to_string(),
            caption: "short".to_string(),
            keywords: vec!["apple".to_string()],
            editorial: false,
            category: "Nature".to_string(),
        };
        let prompt = escalation_prompt(None, None, 5, 80, (3, 5), (50, 180), &previous, true);
        assert!(prompt.contains("Observation:"));
        assert!(prompt.contains("repeated the same title and caption"));
        assert!(prompt.contains("Title: Tiny Title"));
        assert!(prompt.contains("exactly 4 words"));
        assert!(prompt.contains("about 115 characters"));
        assert!(prompt.contains("too short"));
        assert!(prompt.contains("at least 4 unique single-word keywords"));
    }

    #[test]
    fn unparsable_reply_prompt_restates_labels_and_previous_reply() {
        let prompt = unparsable_reply_prompt(
            "Analyze the attached image and return the five labelled fields exactly as specified.",
            "Sure! Here is a photo of a bridge.\nIt looks nice.",
        );
        assert!(prompt.starts_with("Analyze the attached image"));
        assert!(prompt.contains("Title: ..."));
        assert!(prompt.contains("Editorial Status: ..."));
        assert!(prompt.contains("Caption: ..."));
        assert!(prompt.contains("Keywords: ..."));
        assert!(!prompt.contains("Review:"));
        assert!(prompt.contains("Sure! Here is a photo of a bridge. It looks nice."));
    }

    #[test]
    fn custom_ranges_drive_validation() {
        let keywords = unique_words(5);
        let warnings = validate_with_constraints(
            "One Two Three Four",
            &"a".repeat(70),
            &keywords,
            5,
            80,
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
            80,
            (2, 3),
            (60, 80),
        );
        assert_eq!(warnings.len(), 1, "got: {warnings:?}");
        assert!(warnings[0].contains("Caption is 81 characters (needs 60–80)"));
    }
}
