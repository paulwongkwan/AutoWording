//! Agency CSV export and import.
//!
//! Exports use the column layout each agency expects for bulk upload; imports
//! accept those layouts plus the generic one, matching rows to files by name.
//! No spreadsheet library: RFC 4180 quoting is implemented here and covered by
//! round-trip tests.

use crate::ai::prompt::normalize_keywords_with_target;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The drop-down agencies with a known bulk CSV layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agency {
    Generic,
    AdobeStock,
    Shutterstock,
    Istock,
    Dreamstime,
    Alamy,
}

impl Agency {
    pub const ALL: [Agency; 6] = [
        Agency::Generic,
        Agency::AdobeStock,
        Agency::Shutterstock,
        Agency::Istock,
        Agency::Dreamstime,
        Agency::Alamy,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Agency::Generic => "Generic CSV",
            Agency::AdobeStock => "Adobe Stock",
            Agency::Shutterstock => "Shutterstock",
            Agency::Istock => "iStock / Getty",
            Agency::Dreamstime => "Dreamstime",
            Agency::Alamy => "Alamy",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Agency::Generic => "generic",
            Agency::AdobeStock => "adobe-stock",
            Agency::Shutterstock => "shutterstock",
            Agency::Istock => "istock",
            Agency::Dreamstime => "dreamstime",
            Agency::Alamy => "alamy",
        }
    }

    fn header(self) -> &'static [&'static str] {
        match self {
            Agency::Generic => &["Filename", "Title", "Caption", "Keywords", "Editorial"],
            Agency::AdobeStock => &["Filename", "Title", "Keywords", "Category", "Releases"],
            Agency::Shutterstock => &[
                "Filename",
                "Description",
                "Keywords",
                "Categories",
                "Illustration",
                "Mature content",
                "Editorial",
            ],
            // ESP imports also accept this four-column qHero-compatible layout;
            // the wider ESP template adds created date, country and brief code.
            Agency::Istock => &["File Name", "Title", "Description", "Keywords"],
            Agency::Dreamstime => &["Filename", "Title", "Description", "Keywords"],
            // Alamy normally matches by Image Ref after upload; this layout is
            // for the filename-based template contributors send to support.
            Agency::Alamy => &["Filename", "Caption", "Tags", "Supertags"],
        }
    }

    fn fields(self, row: &CsvRow) -> Vec<String> {
        let keywords = row.keywords.join(", ");
        match self {
            Agency::Generic => vec![
                row.filename.clone(),
                row.title.clone(),
                row.caption.clone(),
                keywords,
                if row.editorial.unwrap_or(false) {
                    "Editorial".to_string()
                } else {
                    "Commercial".to_string()
                },
            ],
            Agency::AdobeStock => vec![
                row.filename.clone(),
                row.title.clone(),
                keywords,
                adobe_category_code(&row.category).to_string(),
                String::new(),
            ],
            Agency::Shutterstock => vec![
                row.filename.clone(),
                row.caption.clone(),
                keywords,
                shutterstock_category(&row.category).to_string(),
                "no".to_string(),
                "no".to_string(),
                if row.editorial.unwrap_or(false) {
                    "yes".to_string()
                } else {
                    "no".to_string()
                },
            ],
            Agency::Istock => vec![
                row.filename.clone(),
                row.title.clone(),
                row.caption.clone(),
                keywords,
            ],
            Agency::Dreamstime => vec![
                row.filename.clone(),
                row.title.clone(),
                row.caption.clone(),
                keywords,
            ],
            Agency::Alamy => vec![
                row.filename.clone(),
                row.caption.clone(),
                keywords,
                String::new(),
            ],
        }
    }
}

/// One app category mapped to the agency columns that do not accept free text.
struct CategoryMapping {
    keywords: &'static [&'static str],
    /// Adobe Stock's numeric `Category` code (1–21).
    adobe_code: &'static str,
    /// Shutterstock's exact `Categories` name.
    shutterstock: &'static str,
}

/// Lookup used for the Adobe Stock and Shutterstock CSV layouts. Keywords are
/// matched against whole words of the normalized app category, so `Wildlife`
/// and `animal portraits` both resolve. Order matters: more specific entries
/// come first.
const CATEGORY_MAP: &[CategoryMapping] = &[
    CategoryMapping {
        keywords: &[
            "animals", "animal", "wildlife", "pets", "pet", "birds", "bird", "zoo",
        ],
        adobe_code: "1",
        shutterstock: "Animals/Wildlife",
    },
    CategoryMapping {
        keywords: &[
            "architecture",
            "architectural",
            "buildings",
            "building",
            "landmarks",
            "landmark",
            "cityscape",
            "skyline",
            "city",
            "cities",
        ],
        adobe_code: "2",
        shutterstock: "Buildings/Landmarks",
    },
    CategoryMapping {
        keywords: &["interiors", "interior", "home interior", "room", "rooms"],
        adobe_code: "2",
        shutterstock: "Interiors",
    },
    CategoryMapping {
        keywords: &[
            "business",
            "finance",
            "financial",
            "office",
            "corporate",
            "workplace",
            "teamwork",
            "meeting",
            "money",
            "investment",
            "banking",
        ],
        adobe_code: "3",
        shutterstock: "Business/Finance",
    },
    CategoryMapping {
        keywords: &[
            "environment",
            "environmental",
            "climate",
            "sustainability",
            "recycling",
            "pollution",
            "ecology",
            "conservation",
            "renewable energy",
        ],
        adobe_code: "5",
        shutterstock: "Nature",
    },
    CategoryMapping {
        keywords: &[
            "emotions",
            "emotion",
            "mood",
            "states of mind",
            "happiness",
            "sadness",
            "stress",
            "mental health",
            "mindfulness",
        ],
        adobe_code: "6",
        shutterstock: "People",
    },
    CategoryMapping {
        keywords: &[
            "food",
            "foods",
            "food drink",
            "meal",
            "meals",
            "cooking",
            "cuisine",
            "dessert",
            "desserts",
            "baking",
            "fruit",
            "vegetables",
            "kitchen",
        ],
        adobe_code: "7",
        shutterstock: "Food and drink",
    },
    CategoryMapping {
        keywords: &[
            "drinks",
            "drink",
            "beverages",
            "beverage",
            "coffee",
            "tea",
            "wine",
            "beer",
            "cocktails",
            "cocktail",
        ],
        adobe_code: "4",
        shutterstock: "Food and drink",
    },
    CategoryMapping {
        keywords: &["abstract"],
        adobe_code: "8",
        shutterstock: "Abstract",
    },
    CategoryMapping {
        keywords: &[
            "graphic resources",
            "graphics",
            "patterns",
            "pattern",
            "textures",
            "texture",
            "backgrounds",
            "background",
            "templates",
            "template",
            "icons",
        ],
        adobe_code: "8",
        shutterstock: "Backgrounds/Textures",
    },
    CategoryMapping {
        keywords: &[
            "hobbies",
            "hobby",
            "leisure",
            "gardening",
            "crafts",
            "reading",
            "gaming",
            "board games",
        ],
        adobe_code: "9",
        shutterstock: "Miscellaneous",
    },
    CategoryMapping {
        keywords: &[
            "industry",
            "industrial",
            "factory",
            "factories",
            "manufacturing",
            "construction",
            "mining",
            "machinery",
            "agriculture",
            "engineering",
        ],
        adobe_code: "10",
        shutterstock: "Industrial",
    },
    CategoryMapping {
        keywords: &[
            "landscapes",
            "landscape",
            "nature",
            "scenery",
            "scenic",
            "mountains",
            "mountain",
            "sunset",
            "sunrise",
            "beach",
            "forest",
            "sea",
            "ocean",
            "sky",
            "countryside",
            "lake",
            "waterfall",
            "seaside",
        ],
        adobe_code: "11",
        shutterstock: "Nature",
    },
    CategoryMapping {
        keywords: &["beauty", "fashion", "makeup", "style"],
        adobe_code: "12",
        shutterstock: "Beauty/Fashion",
    },
    CategoryMapping {
        keywords: &["lifestyle", "wellness", "yoga", "relaxation", "shopping"],
        adobe_code: "12",
        shutterstock: "People",
    },
    CategoryMapping {
        keywords: &[
            "people",
            "person",
            "portraits",
            "portrait",
            "woman",
            "women",
            "man",
            "men",
            "child",
            "children",
            "family",
            "families",
            "friends",
            "crowd",
            "couple",
            "baby",
            "babies",
            "seniors",
            "education",
            "students",
            "school",
        ],
        adobe_code: "13",
        shutterstock: "People",
    },
    CategoryMapping {
        keywords: &[
            "plants",
            "plant",
            "flowers",
            "flower",
            "flora",
            "gardens",
            "garden",
            "trees",
            "tree",
            "leaves",
            "botanical",
        ],
        adobe_code: "14",
        shutterstock: "Nature",
    },
    CategoryMapping {
        keywords: &[
            "culture",
            "cultural",
            "heritage",
            "tradition",
            "festivals",
            "festival",
            "museum",
            "arts",
            "art",
            "music",
        ],
        adobe_code: "15",
        shutterstock: "Arts",
    },
    CategoryMapping {
        keywords: &[
            "religion",
            "religious",
            "church",
            "temple",
            "faith",
            "prayer",
            "spiritual",
        ],
        adobe_code: "15",
        shutterstock: "Religion",
    },
    CategoryMapping {
        keywords: &[
            "science",
            "scientific",
            "research",
            "laboratory",
            "lab",
            "space",
            "astronomy",
            "chemistry",
            "biology",
            "physics",
            "microscope",
        ],
        adobe_code: "16",
        shutterstock: "Science",
    },
    CategoryMapping {
        keywords: &[
            "social issues",
            "poverty",
            "homelessness",
            "protest",
            "inequality",
            "charity",
            "volunteers",
            "diversity",
            "human rights",
        ],
        adobe_code: "17",
        shutterstock: "Miscellaneous",
    },
    CategoryMapping {
        keywords: &[
            "sports",
            "sport",
            "fitness",
            "exercise",
            "running",
            "soccer",
            "football",
            "basketball",
            "cycling",
            "swimming",
            "gym",
            "athletes",
            "recreation",
        ],
        adobe_code: "18",
        shutterstock: "Sports/Recreation",
    },
    CategoryMapping {
        keywords: &[
            "technology",
            "tech",
            "computers",
            "computer",
            "laptops",
            "laptop",
            "smartphones",
            "smartphone",
            "phone",
            "internet",
            "software",
            "digital",
            "robots",
            "robot",
            "robotics",
            "ai",
            "electronics",
        ],
        adobe_code: "19",
        shutterstock: "Technology",
    },
    CategoryMapping {
        keywords: &[
            "transport",
            "transportation",
            "traffic",
            "cars",
            "car",
            "roads",
            "road",
            "trains",
            "train",
            "bicycles",
            "bicycle",
            "airplanes",
            "airplane",
            "plane",
            "airport",
            "ships",
            "ship",
            "boats",
            "boat",
            "harbor",
            "harbour",
            "vehicles",
            "vehicle",
            "driving",
            "logistics",
        ],
        adobe_code: "20",
        shutterstock: "Transportation",
    },
    CategoryMapping {
        keywords: &[
            "holidays",
            "holiday",
            "christmas",
            "vacations",
            "vacation",
            "tourism",
            "tourist",
            "tourists",
            "destination",
            "journey",
            "adventure",
            "backpacking",
            "sightseeing",
            "travel",
        ],
        adobe_code: "21",
        shutterstock: "Parks/Outdoor",
    },
];

fn normalize_category_label(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Find the agency mapping for a free-text app category. Whole words only, so
/// `seaside` does not match `sea` and `cards` does not match `car`.
fn category_mapping(category: &str) -> Option<&'static CategoryMapping> {
    let normalized = normalize_category_label(category);
    if normalized.is_empty() {
        return None;
    }
    let padded = format!(" {normalized} ");
    CATEGORY_MAP.iter().find(|mapping| {
        mapping.keywords.iter().any(|keyword| {
            if keyword.contains(' ') {
                padded.contains(&format!(" {keyword} "))
            } else {
                normalized.split(' ').any(|token| token == *keyword)
            }
        })
    })
}

/// Adobe Stock's numeric category code, or an empty cell when the app category
/// is blank or has no known mapping (Adobe rejects free text there).
pub fn adobe_category_code(category: &str) -> &'static str {
    category_mapping(category)
        .map(|mapping| mapping.adobe_code)
        .unwrap_or_default()
}

/// Shutterstock's category name, or an empty cell when the app category is
/// blank or has no known mapping.
pub fn shutterstock_category(category: &str) -> &'static str {
    category_mapping(category)
        .map(|mapping| mapping.shutterstock)
        .unwrap_or_default()
}

/// Canonical app category for each Adobe Stock numeric code. Adobe accepts one
/// code per upload row, so several app categories share a code; the canonical
/// label is the one that maps back to the same code on the next export.
const ADOBE_CATEGORY_LABELS: &[(&str, &str)] = &[
    ("1", "Animals"),
    ("2", "Architecture"),
    ("3", "Business"),
    ("4", "Drinks"),
    ("5", "Environment"),
    ("6", "Emotions"),
    ("7", "Food"),
    ("8", "Abstract"),
    ("9", "Hobbies"),
    ("10", "Industry"),
    ("11", "Nature"),
    ("12", "Lifestyle"),
    ("13", "People"),
    ("14", "Plants"),
    ("15", "Culture"),
    ("16", "Science"),
    ("17", "Social Issues"),
    ("18", "Sports"),
    ("19", "Technology"),
    ("20", "Transport"),
    ("21", "Travel"),
];

/// Canonical app category for each Shutterstock category name. Shutterstock
/// merges several app categories (for example Environment, Nature and Plants
/// all become `Nature`), so the reverse mapping chooses a label that exports to
/// the same Shutterstock name.
const SHUTTERSTOCK_CATEGORY_LABELS: &[(&str, &str)] = &[
    ("Animals/Wildlife", "Animals"),
    ("Buildings/Landmarks", "Architecture"),
    ("Interiors", "Interiors"),
    ("Business/Finance", "Business"),
    ("Nature", "Nature"),
    ("People", "People"),
    ("Food and drink", "Food"),
    ("Abstract", "Abstract"),
    ("Backgrounds/Textures", "Graphic Resources"),
    ("Miscellaneous", "Hobbies"),
    ("Industrial", "Industry"),
    ("Beauty/Fashion", "Beauty"),
    ("Arts", "Culture"),
    ("Religion", "Religion"),
    ("Science", "Science"),
    ("Sports/Recreation", "Sports"),
    ("Technology", "Technology"),
    ("Transportation", "Transport"),
    ("Parks/Outdoor", "Travel"),
];

/// Convert an Adobe Stock numeric category cell back to an app category label.
pub fn app_category_from_adobe(code: &str) -> Option<&'static str> {
    let code = code.trim();
    ADOBE_CATEGORY_LABELS
        .iter()
        .find(|(agency_code, _)| *agency_code == code)
        .map(|(_, label)| *label)
}

/// Convert a Shutterstock category name back to an app category label. Matching
/// is case-insensitive and ignores punctuation/spacing (`parks/outdoor`).
pub fn app_category_from_shutterstock(name: &str) -> Option<&'static str> {
    let normalized = normalize_category_label(name);
    SHUTTERSTOCK_CATEGORY_LABELS
        .iter()
        .find(|(agency_name, _)| normalize_category_label(agency_name) == normalized)
        .map(|(_, label)| *label)
}

/// Map an imported category cell to the app's free-text label, preserving values
/// that are not a known agency code/name so a generic or custom category is not
/// silently discarded.
fn import_category(raw: &str, shutterstock: bool) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let mapped = if shutterstock {
        app_category_from_shutterstock(raw)
    } else {
        app_category_from_adobe(raw)
    };
    mapped
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportIssue {
    pub filename: String,
    pub message: String,
    /// This issue will leave the agency's category cell blank.
    pub category_mapping: bool,
}

/// Check rows against the requirements the agency's bulk uploader enforces.
/// A clean result means the CSV can be written immediately; otherwise the app
/// shows the issues and lets the user export anyway. Issues are grouped by
/// filename so the preflight lists each affected row once, with every problem
/// for that file joined in `message`.
pub fn preflight(agency: Agency, rows: &[CsvRow]) -> Vec<ExportIssue> {
    let mut issues: Vec<ExportIssue> = Vec::new();
    for row in rows {
        let mut messages = Vec::new();
        let mut category_mapping = false;
        {
            let mut report = |message: String, category_mapping_issue: bool| {
                messages.push(message);
                category_mapping |= category_mapping_issue;
            };
            let title_missing = row.title.trim().is_empty();
            let caption_missing = row.caption.trim().is_empty();
            match agency {
                Agency::Generic => {
                    if title_missing {
                        report("Missing title".into(), false);
                    }
                    if caption_missing {
                        report("Missing caption".into(), false);
                    }
                    if row.keywords.is_empty() {
                        report("Missing keywords".into(), false);
                    }
                }
                Agency::AdobeStock => {
                    if title_missing {
                        report("Missing title".into(), false);
                    }
                    let category = row.category.trim();
                    if category.is_empty() {
                        report(
                            "Missing category: Adobe Stock requires a category code".into(),
                            true,
                        );
                    } else if adobe_category_code(category).is_empty() {
                        report(
                            format!(
                                "Category {category:?} has no Adobe Stock mapping; its category cell will be blank"
                            ),
                            true,
                        );
                    }
                    if row.keywords.is_empty() {
                        report("Missing keywords".into(), false);
                    }
                }
                Agency::Shutterstock => {
                    if caption_missing {
                        report("Missing description".into(), false);
                    }
                    if row.keywords.len() < 7 {
                        report(
                            format!(
                                "Shutterstock needs at least 7 keywords (has {})",
                                row.keywords.len()
                            ),
                            false,
                        );
                    }
                    let category = row.category.trim();
                    if category.is_empty() {
                        report(
                            "Missing category: Shutterstock requires a category name".into(),
                            true,
                        );
                    } else if shutterstock_category(category).is_empty() {
                        report(
                            format!(
                                "Category {category:?} has no Shutterstock mapping; its category cell will be blank"
                            ),
                            true,
                        );
                    }
                }
                Agency::Istock | Agency::Dreamstime => {
                    if title_missing {
                        report("Missing title".into(), false);
                    }
                    if caption_missing {
                        report("Missing description".into(), false);
                    }
                    if row.keywords.is_empty() {
                        report("Missing keywords".into(), false);
                    }
                }
                Agency::Alamy => {
                    if caption_missing {
                        report("Missing caption".into(), false);
                    }
                    if caption_chars(&row.caption) > 150 {
                        report(
                            format!(
                                "Alamy captions are capped at 150 characters (has {})",
                                caption_chars(&row.caption)
                            ),
                            false,
                        );
                    }
                    if row.keywords.is_empty() {
                        report("Missing keywords".into(), false);
                    }
                }
            }
        }
        if messages.is_empty() {
            continue;
        }
        let message = messages.join("; ");
        if let Some(issue) = issues
            .iter_mut()
            .find(|issue| issue.filename == row.filename)
        {
            issue.message.push_str("; ");
            issue.message.push_str(&message);
            issue.category_mapping |= category_mapping;
        } else {
            issues.push(ExportIssue {
                filename: row.filename.clone(),
                message,
                category_mapping,
            });
        }
    }
    issues
}

fn caption_chars(caption: &str) -> usize {
    caption.chars().count()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CsvRow {
    pub filename: String,
    pub title: String,
    pub caption: String,
    /// Free-text app category, mapped to agency codes/names on export.
    pub category: String,
    pub keywords: Vec<String>,
    pub editorial: Option<bool>,
}

/// Serialize rows for one agency. Uses CRLF line endings, as spreadsheet
/// applications and agency uploaders expect.
pub fn export(agency: Agency, rows: &[CsvRow]) -> String {
    let mut out = String::new();
    write_record(
        &mut out,
        agency
            .header()
            .iter()
            .map(|field| field.to_string())
            .collect(),
    );
    for row in rows {
        write_record(&mut out, agency.fields(row));
    }
    out
}

fn write_record(out: &mut String, fields: Vec<String>) {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&escape(field));
    }
    out.push_str("\r\n");
}

fn escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Parse a CSV into usable rows. Header names are matched case-insensitively
/// and accept the agency variants (`Description`, `Headline`, `Tags`, …).
/// A row is skipped when it is entirely empty; a missing Filename column is an
/// error so the user learns why nothing matched.
pub fn parse(text: &str) -> Result<Vec<CsvRow>, String> {
    let text = text.trim_start_matches('\u{feff}');
    let records = parse_records(text);
    let Some(header) = records.first() else {
        return Err("The CSV file is empty".to_string());
    };

    let mut filename_column = None;
    let mut title_column = None;
    let mut caption_column = None;
    let mut keywords_column = None;
    let mut category_column = None;
    let mut categories_column = None;
    let mut editorial_column = None;
    for (index, name) in header.iter().enumerate() {
        match normalize_header(name).as_str() {
            "filename" | "file" | "file name" if filename_column.is_none() => {
                filename_column = Some(index)
            }
            "title" | "headline" | "object name" if title_column.is_none() => {
                title_column = Some(index)
            }
            "caption" | "description" | "caption abstract" | "caption-abstract"
                if caption_column.is_none() =>
            {
                caption_column = Some(index)
            }
            "keywords" | "keyword" | "tags" | "subject" if keywords_column.is_none() => {
                keywords_column = Some(index)
            }
            "category" if category_column.is_none() => category_column = Some(index),
            "categories" if categories_column.is_none() => categories_column = Some(index),
            "editorial" | "editorial status" if editorial_column.is_none() => {
                editorial_column = Some(index)
            }
            _ => {}
        }
    }

    let Some(filename_column) = filename_column else {
        return Err("The CSV needs a Filename column".to_string());
    };

    let mut rows = Vec::new();
    for record in records.iter().skip(1) {
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        let field = |column: Option<usize>| {
            column
                .and_then(|index| record.get(index))
                .map(|value| value.trim().to_string())
                .unwrap_or_default()
        };
        let filename = field(Some(filename_column));
        if filename.is_empty() {
            continue;
        }
        let keywords_raw = field(keywords_column);
        let categories_raw = field(categories_column);
        let category = if categories_raw.is_empty() {
            import_category(&field(category_column), false)
        } else {
            import_category(&categories_raw, true)
        };
        rows.push(CsvRow {
            filename,
            title: field(title_column),
            caption: field(caption_column),
            category,
            keywords: normalize_keywords_with_target(&keywords_raw, 50),
            editorial: editorial_column
                .and_then(|index| record.get(index))
                .and_then(|value| parse_editorial(value)),
        });
    }
    Ok(rows)
}

fn normalize_header(value: &str) -> String {
    value.trim().to_lowercase()
}

fn parse_editorial(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "editorial" | "yes" | "true" | "1" => Some(true),
        "commercial" | "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_records(text: &str) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;

    while let Some(character) = chars.next() {
        if quoted {
            match character {
                '"' => {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        quoted = false;
                    }
                }
                _ => field.push(character),
            }
        } else {
            match character {
                '"' => quoted = true,
                ',' => record.push(std::mem::take(&mut field)),
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                }
                '\n' => {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                }
                _ => field.push(character),
            }
        }
    }

    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

/// Map lowercased file names to grid paths, dropping any directory part a CSV
/// may include. First path wins when a folder contains duplicate base names.
pub fn filename_index(paths: &[PathBuf]) -> HashMap<String, PathBuf> {
    let mut index = HashMap::new();
    for path in paths {
        let name = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_lowercase());
        if let Some(name) = name {
            index.entry(name).or_insert_with(|| path.clone());
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(filename: &str) -> CsvRow {
        CsvRow {
            filename: filename.to_string(),
            title: "Harbour at dawn".to_string(),
            caption: "A calm harbour at sunrise, with boats and a stone pier.".to_string(),
            category: "Nature".to_string(),
            keywords: vec!["harbour".to_string(), "sunrise".to_string()],
            editorial: Some(false),
        }
    }

    #[test]
    fn generic_export_round_trips_quoted_values() {
        let mut row = row("a.jpg");
        row.title = "Harbour, dawn".to_string();
        row.caption = "Line one\nLine \"two\"".to_string();
        let csv = export(Agency::Generic, &[row]);

        assert!(csv.starts_with("Filename,Title,Caption,Keywords,Editorial\r\n"));
        assert!(csv.contains("\"Harbour, dawn\""));
        assert!(csv.contains("\"Line one\nLine \"\"two\"\"\""));

        let parsed = parse(&csv).expect("parse exported csv");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].filename, "a.jpg");
        assert_eq!(parsed[0].title, "Harbour, dawn");
        assert_eq!(parsed[0].caption, "Line one\nLine \"two\"");
        assert_eq!(parsed[0].keywords, vec!["harbour", "sunrise"]);
        assert_eq!(parsed[0].editorial, Some(false));
    }

    #[test]
    fn exports_agency_specific_headers_and_columns() {
        let rows = vec![row("b.jpg")];

        let adobe = export(Agency::AdobeStock, &rows);
        assert!(adobe.starts_with("Filename,Title,Keywords,Category,Releases\r\n"));
        assert!(adobe.contains("b.jpg,Harbour at dawn,\"harbour, sunrise\",11,"));

        let istock = export(Agency::Istock, &rows);
        assert!(istock.starts_with("File Name,Title,Description,Keywords\r\n"));
        assert!(istock.contains(
            "b.jpg,Harbour at dawn,\"A calm harbour at sunrise, with boats and a stone pier.\""
        ));

        let dreamstime = export(Agency::Dreamstime, &rows);
        assert!(dreamstime.starts_with("Filename,Title,Description,Keywords\r\n"));

        let alamy = export(Agency::Alamy, &rows);
        assert!(alamy.starts_with("Filename,Caption,Tags,Supertags\r\n"));
        assert!(alamy.contains("b.jpg,\"A calm harbour at sunrise, with boats and a stone pier.\",\"harbour, sunrise\","));

        let shutterstock = export(Agency::Shutterstock, &rows);
        assert!(shutterstock.starts_with(
            "Filename,Description,Keywords,Categories,Illustration,Mature content,Editorial\r\n"
        ));
        assert!(shutterstock.contains(
            "b.jpg,\"A calm harbour at sunrise, with boats and a stone pier.\",\"harbour, sunrise\",Nature,no,no,no"
        ));

        let mut editorial = row("e.jpg");
        editorial.editorial = Some(true);
        let shutterstock = export(Agency::Shutterstock, &[editorial]);
        assert!(
            shutterstock.contains(",Nature,no,no,yes\r\n"),
            "editorial is the final optional column: {shutterstock}"
        );
    }

    #[test]
    fn maps_category_labels_to_adobe_codes_and_shutterstock_names() {
        assert_eq!(adobe_category_code("Nature"), "11");
        assert_eq!(shutterstock_category("Nature"), "Nature");
        assert_eq!(adobe_category_code("  travel "), "21");
        assert_eq!(shutterstock_category("Travel"), "Parks/Outdoor");
        assert_eq!(adobe_category_code("Architecture"), "2");
        assert_eq!(shutterstock_category("architecture"), "Buildings/Landmarks");
        assert_eq!(adobe_category_code("Animals"), "1");
        assert_eq!(shutterstock_category("Wildlife"), "Animals/Wildlife");
        assert_eq!(adobe_category_code("People"), "13");
        assert_eq!(adobe_category_code("Food & Drink"), "7");
        assert_eq!(shutterstock_category("Food & Drink"), "Food and drink");
        assert_eq!(adobe_category_code("Business"), "3");
        assert_eq!(adobe_category_code("Technology"), "19");
        assert_eq!(shutterstock_category("Transport"), "Transportation");
        assert_eq!(shutterstock_category("Flowers"), "Nature");
        assert_eq!(shutterstock_category("Church"), "Religion");
        assert_eq!(shutterstock_category("Abstract"), "Abstract");
        assert_eq!(shutterstock_category("Patterns"), "Backgrounds/Textures");
        assert_eq!(adobe_category_code("Sports"), "18");
        assert_eq!(adobe_category_code("Science"), "16");
    }

    #[test]
    fn unmapped_or_blank_categories_export_empty_cells() {
        assert_eq!(adobe_category_code(""), "");
        assert_eq!(shutterstock_category("   "), "");
        assert_eq!(adobe_category_code("Zebra unicorn"), "");
        assert_eq!(shutterstock_category("Zebra unicorn"), "");

        let mut row = row("c.jpg");
        row.category = "Zebra unicorn".to_string();
        let adobe = export(Agency::AdobeStock, &[row.clone()]);
        assert!(adobe.contains("c.jpg,Harbour at dawn,\"harbour, sunrise\",,\r\n"));
        let shutterstock = export(Agency::Shutterstock, &[row]);
        assert!(shutterstock.contains(",\"harbour, sunrise\",,no,no,no\r\n"));
    }

    #[test]
    fn imports_shutterstock_columns_and_editorial_flag() {
        let text =
            "Filename,Description,Keywords,Categories,Editorial,Mature content,Illustration\r\n\
                    photo.jpg,\"A caption, with comma\",\"bridge, sunrise\",,yes,no,no\r\n\
                    commercial.jpg,Caption,keyword,,commercial,no,no\r\n";
        let rows = parse(text).expect("parse shutterstock csv");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].filename, "photo.jpg");
        assert_eq!(rows[0].caption, "A caption, with comma");
        assert_eq!(rows[0].keywords, vec!["bridge", "sunrise"]);
        assert_eq!(rows[0].editorial, Some(true));
        assert_eq!(rows[1].editorial, Some(false));
        assert_eq!(rows[1].title, "", "Shutterstock has no title column");
    }

    #[test]
    fn imports_adobe_and_generic_headers_with_paths() {
        let adobe = "filename,Title,Keywords\nC:\\photos\\x.jpg,Title One,\"sky, cloud\"\n";
        let rows = parse(adobe).expect("parse adobe csv");
        assert_eq!(rows[0].filename, "C:\\photos\\x.jpg");
        assert_eq!(rows[0].keywords, vec!["sky", "cloud"]);

        let generic = "\u{feff}File Name,Headline,Description,Tags,Editorial Status\r\ny.jpg,Head,Caption,one;two;three,editorial\r\n";
        let rows = parse(generic).expect("parse generic csv");
        assert_eq!(rows[0].title, "Head");
        assert_eq!(rows[0].caption, "Caption");
        assert_eq!(rows[0].keywords, vec!["one", "two", "three"]);
        assert_eq!(rows[0].editorial, Some(true));
    }

    #[test]
    fn rejects_files_without_a_filename_column_and_skips_blank_rows() {
        assert!(parse("").expect_err("empty csv").contains("empty"));
        assert!(parse("Title,Caption\nx,y\n")
            .expect_err("missing filename")
            .contains("Filename"));

        let text = "Filename,Title\n\r\nx.jpg,\n";
        let rows = parse(text).expect("blank rows are ignored");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].filename, "x.jpg");
    }

    #[test]
    fn preflight_groups_all_issues_for_a_filename_into_one_row() {
        let mut generic = row("grouped.jpg");
        generic.title.clear();
        generic.caption.clear();
        generic.keywords.clear();
        let issues = preflight(Agency::Generic, &[generic]);
        assert_eq!(issues.len(), 1, "one preflight row per filename");
        assert_eq!(issues[0].filename, "grouped.jpg");
        assert_eq!(
            issues[0].message,
            "Missing title; Missing caption; Missing keywords"
        );
        assert!(!issues[0].category_mapping);

        let mut adobe = row("grouped.jpg");
        adobe.title.clear();
        adobe.category = "Zebra unicorn".to_string();
        adobe.keywords.clear();
        let issues = preflight(Agency::AdobeStock, &[adobe]);
        assert_eq!(issues.len(), 1, "one preflight row per filename");
        assert!(issues[0].message.contains("Missing title"));
        assert!(issues[0].message.contains("no Adobe Stock mapping"));
        assert!(issues[0].message.contains("Missing keywords"));
        assert!(issues[0].category_mapping);
    }

    #[test]
    fn preflight_flags_agency_specific_gaps() {
        let clean = row("a.jpg");
        assert!(
            preflight(Agency::AdobeStock, &[clean.clone()]).is_empty(),
            "Adobe accepts a title, keywords and mapped category"
        );
        let shutterstock = preflight(Agency::Shutterstock, &[clean.clone()]);
        assert!(shutterstock
            .iter()
            .any(|issue| issue.message.contains("at least 7 keywords")));

        let mut missing = clean;
        missing.title.clear();
        missing.category = "Zebra unicorn".to_string();
        let issues = preflight(Agency::AdobeStock, &[missing]);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("Missing title"));
        assert!(issues[0].message.contains("no Adobe Stock mapping"));
        assert!(issues[0].category_mapping);

        let mut shutterstock_unmapped = row("b.jpg");
        shutterstock_unmapped.category = "Zebra unicorn".to_string();
        let issues = preflight(Agency::Shutterstock, &[shutterstock_unmapped]);
        assert!(issues.iter().any(|issue| {
            issue.category_mapping && issue.message.contains("no Shutterstock mapping")
        }));

        let mut blank = row("c.jpg");
        blank.category.clear();
        let issues = preflight(Agency::AdobeStock, &[blank]);
        assert!(issues
            .iter()
            .any(|issue| { issue.category_mapping && issue.message.contains("Missing category") }));
    }

    #[test]
    fn agency_categories_round_trip_through_import() {
        let mut travel = row("travel.jpg");
        travel.category = "Travel".to_string();
        let adobe = export(Agency::AdobeStock, &[travel]);
        let parsed = parse(&adobe).expect("parse Adobe export");
        assert_eq!(parsed[0].category, "Travel");

        let mut animals = row("animals.jpg");
        animals.category = "Animals".to_string();
        let shutterstock = export(Agency::Shutterstock, &[animals]);
        let parsed = parse(&shutterstock).expect("parse Shutterstock export");
        assert_eq!(parsed[0].category, "Animals");

        let adobe = "Filename,Title,Keywords,Category,Releases\r\n\
                     x.jpg,Title,\"bridge, sunrise\",11,\r\n\
                     y.jpg,Title,\"bridge, sunrise\",Unsorted,\r\n";
        let parsed = parse(adobe).expect("parse Adobe category cells");
        assert_eq!(parsed[0].category, "Nature");
        assert_eq!(parsed[1].category, "Unsorted");

        let shutterstock =
            "Filename,Description,Keywords,Categories,Illustration,Mature content,Editorial\r\n\
                            x.jpg,Caption,\"bridge, sunrise\",nature,no,no,no\r\n\
                            y.jpg,Caption,\"bridge, sunrise\",Custom,no,no,no\r\n";
        let parsed = parse(shutterstock).expect("parse Shutterstock category cells");
        assert_eq!(parsed[0].category, "Nature");
        assert_eq!(parsed[1].category, "Custom");
    }

    #[test]
    fn filename_index_uses_base_names_case_insensitively() {
        let paths = vec![
            PathBuf::from(r"C:\photos\One.JPG"),
            PathBuf::from(r"C:\photos\two.jpg"),
        ];
        let index = filename_index(&paths);
        assert_eq!(index.get("one.jpg"), Some(&paths[0]));
        assert_eq!(index.get("two.jpg"), Some(&paths[1]));
        assert!(
            index.get("TWO.JPG").is_none(),
            "keys are lowercased for lookup"
        );
        assert!(index.get("three.jpg").is_none());
    }
}
