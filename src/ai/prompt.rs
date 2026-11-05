use anyhow::{bail, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const KEYWORD_TARGET: usize = 50;

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
   - Length: Exactly 3 to 5 words.
   - Content: Clear, descriptive summary of the primary subject.

2. Caption:
   - Length: Strict limit of 50 to 180 characters (including spaces).
   - Content: Factual description of the scene (or Shutterstock editorial format if applicable).

3. Keywords:
   - Quantity: Exactly 50 keywords.
   - Format: Single words only (no multi-word phrases/keyphrases).
   - Separator: Comma-separated.
   - Rules: Alphabetic characters only. No numbers, punctuation, or special characters.

Required Output Schema:
Title: [3-5 words]
Editorial Status: [Commercial / Editorial]
Caption: [50-180 characters]
Keywords: [word1, word2, word3, ... word50]

Reply with the four labelled lines above and nothing else. Do not use markdown."#;

pub fn system_prompt(date: &str, extra: &str) -> String {
    let mut prompt = SYSTEM_TEMPLATE.replace("{DATE}", date);
    let extra = extra.trim();
    if !extra.is_empty() {
        prompt.push_str("\n\nAdditional instructions from the photographer (follow these too):\n");
        prompt.push_str(extra);
    }
    prompt
}

pub fn user_prompt(context: Option<&str>) -> String {
    match context.map(str::trim).filter(|c| !c.is_empty()) {
        Some(context) => format!(
            "Situational context supplied by the photographer (treat it as ground truth):\n{context}\n\nAnalyze the attached image and return the four labelled fields exactly as specified."
        ),
        None => "Analyze the attached image and return the four labelled fields exactly as specified."
            .to_string(),
    }
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

pub fn parse_response(text: &str) -> Result<GeneratedMetadata> {
    let fields = parse_fields(text);

    let title = fields.get("title").cloned().unwrap_or_default();
    let caption = fields.get("caption").cloned().unwrap_or_default();
    let keyword_raw = fields.get("keywords").cloned().unwrap_or_default();
    let editorial_raw = fields
        .get("editorial")
        .cloned()
        .unwrap_or_else(|| "Commercial".to_string());

    let keywords = normalize_keywords(&keyword_raw);

    if title.is_empty() || caption.is_empty() || keywords.is_empty() {
        let mut missing = Vec::new();
        if title.is_empty() {
            missing.push("Title");
        }
        if caption.is_empty() {
            missing.push("Caption");
        }
        if keywords.is_empty() {
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
pub fn normalize_keywords(raw: &str) -> Vec<String> {
    let mut keywords: Vec<String> = Vec::with_capacity(KEYWORD_TARGET);
    let mut seen = std::collections::HashSet::new();

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
                if keywords.len() == KEYWORD_TARGET {
                    return keywords;
                }
            }
        }
    }

    keywords
}

pub fn is_valid_keyword(word: &str) -> bool {
    word.chars().count() >= 2 && word.chars().all(|c| c.is_alphabetic())
}

pub fn validate(title: &str, caption: &str, keywords: &[String]) -> Vec<String> {
    let mut warnings = Vec::new();

    let words = title.split_whitespace().count();
    if !(3..=5).contains(&words) {
        warnings.push(format!("Title has {words} words (needs 3–5)"));
    }

    let chars = caption.chars().count();
    if !(50..=180).contains(&chars) {
        warnings.push(format!("Caption is {chars} characters (needs 50–180)"));
    }

    if keywords.len() != KEYWORD_TARGET {
        warnings.push(format!(
            "{} keywords (needs exactly {KEYWORD_TARGET})",
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
        assert!(user_prompt(None).contains("Analyze the attached image"));
        assert!(!user_prompt(None).contains("Situational context"));
        assert!(!user_prompt(Some("   ")).contains("Situational context"));

        let prompt = user_prompt(Some("  King's Day in Amsterdam  "));
        assert!(prompt.contains("King's Day in Amsterdam"));
        assert!(!prompt.contains("  King's Day"));
        assert!(prompt.contains("ground truth"));
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
}
