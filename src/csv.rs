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
                "Editorial",
                "Mature content",
                "Illustration",
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
                String::new(),
                String::new(),
            ],
            Agency::Shutterstock => vec![
                row.filename.clone(),
                row.caption.clone(),
                keywords,
                String::new(),
                if row.editorial.unwrap_or(false) {
                    "yes".to_string()
                } else {
                    "no".to_string()
                },
                "no".to_string(),
                "no".to_string(),
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CsvRow {
    pub filename: String,
    pub title: String,
    pub caption: String,
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
        rows.push(CsvRow {
            filename,
            title: field(title_column),
            caption: field(caption_column),
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
        assert!(adobe.contains("b.jpg,Harbour at dawn,\"harbour, sunrise\",,"));

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
            "Filename,Description,Keywords,Categories,Editorial,Mature content,Illustration\r\n"
        ));
        assert!(shutterstock.contains(
            "b.jpg,\"A calm harbour at sunrise, with boats and a stone pier.\",\"harbour, sunrise\",,no,no,no"
        ));
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
