use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ExistingMetadata {
    pub title: Option<String>,
    pub caption: Option<String>,
    pub keywords: Vec<String>,
    pub dims: Option<(u32, u32)>,
}

/// Tags requested from exiftool when loading a file (XMP first, IPTC/EXIF fallbacks).
pub const READ_TAGS: &[&str] = &[
    "-Title",
    "-ObjectName",
    "-Description",
    "-Caption-Abstract",
    "-ImageDescription",
    "-XPTitle",
    "-IPTC:Keywords",
    "-Subject",
    "-ImageSize",
];

/// Build the exiftool argument list used to write stock-compliant metadata.
///
/// Title  -> XMP-dc:Title, IPTC:ObjectName, EXIF:XPTitle
/// Caption-> EXIF:ImageDescription, IPTC:Caption-Abstract, XMP-dc:Description
/// Words  -> IPTC:Keywords / XMP-dc:Subject (repeating values)
pub fn write_args(md: &Metadata) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-P".into(),
        "-overwrite_original".into(),
        "-charset".into(),
        "iptc=utf8".into(),
        "-m".into(),
    ];

    let title = md.title.trim();
    let caption = md.caption.trim();

    if !title.is_empty() {
        args.push(format!("-XMP-dc:Title={title}"));
        args.push(format!("-IPTC:ObjectName={title}"));
        args.push(format!("-EXIF:XPTitle={title}"));
    }

    if !caption.is_empty() {
        args.push(format!("-EXIF:ImageDescription={caption}"));
        args.push(format!("-IPTC:Caption-Abstract={caption}"));
        args.push(format!("-XMP-dc:Description={caption}"));
    } else if !title.is_empty() {
        args.push(format!("-EXIF:ImageDescription={title}"));
    }

    let keywords: Vec<&str> = md
        .keywords
        .iter()
        .map(|keyword| keyword.trim())
        .filter(|keyword| !keyword.is_empty())
        .collect();

    // Repeating "=" replaces the existing list. Using any "+=" in the same
    // command would make ExifTool preserve the old values instead.
    for keyword in &keywords {
        args.push(format!("-IPTC:Keywords={keyword}"));
    }
    for keyword in &keywords {
        args.push(format!("-XMP-dc:Subject={keyword}"));
    }

    args
}

fn first_string(object: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn keyword_list(object: &Value, key: &str) -> Vec<String> {
    match object.get(key) {
        Some(Value::String(text)) => text
            .split(',')
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Parse the `exiftool -json` output into the editable fields.
pub fn parse_read_json(text: &str) -> Result<ExistingMetadata> {
    let value: Value = serde_json::from_str(text)?;
    let object = value
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| anyhow!("exiftool returned no metadata"))?;

    let title = first_string(object, &["Title", "XPTitle", "ObjectName"]);
    let caption = first_string(
        object,
        &["Description", "Caption-Abstract", "ImageDescription"],
    );

    let mut keywords: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for source in ["Keywords", "Subject"] {
        for keyword in keyword_list(object, source) {
            if seen.insert(keyword.to_lowercase()) {
                keywords.push(keyword);
            }
        }
    }

    let dims = object
        .get("ImageSize")
        .and_then(Value::as_str)
        .and_then(|size| {
            let (width, height) = size.split_once('x')?;
            Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
        });

    Ok(ExistingMetadata {
        title,
        caption,
        keywords,
        dims,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_all_microstock_tags() {
        let md = Metadata {
            title: "Golden Gate Sunrise".into(),
            caption: "Fog drifts under the Golden Gate Bridge at sunrise in San Francisco.".into(),
            keywords: vec!["bridge".into(), "sunrise".into(), "fog".into()],
        };
        let args = write_args(&md);
        assert!(args.contains(&"-XMP-dc:Title=Golden Gate Sunrise".to_string()));
        assert!(args.contains(&"-IPTC:ObjectName=Golden Gate Sunrise".to_string()));
        assert!(args.contains(&"-EXIF:XPTitle=Golden Gate Sunrise".to_string()));
        assert!(args.contains(&"-EXIF:ImageDescription=Fog drifts under the Golden Gate Bridge at sunrise in San Francisco.".to_string()));
        assert!(args.contains(&"-IPTC:Caption-Abstract=Fog drifts under the Golden Gate Bridge at sunrise in San Francisco.".to_string()));
        assert!(args.contains(&"-XMP-dc:Description=Fog drifts under the Golden Gate Bridge at sunrise in San Francisco.".to_string()));
        assert!(args.contains(&"-IPTC:Keywords=bridge".to_string()));
        assert!(args.contains(&"-IPTC:Keywords=sunrise".to_string()));
        assert!(args.contains(&"-XMP-dc:Subject=fog".to_string()));
        assert!(!args.iter().any(|arg| arg.contains("+=")));
        assert!(args.contains(&"-overwrite_original".to_string()));
    }

    #[test]
    fn falls_back_to_title_for_image_description() {
        let md = Metadata {
            title: "Lone Tree".into(),
            caption: String::new(),
            keywords: vec![],
        };
        let args = write_args(&md);
        assert!(args.contains(&"-EXIF:ImageDescription=Lone Tree".to_string()));
        assert!(!args.iter().any(|arg| arg.starts_with("-IPTC:Keywords")));
    }

    #[test]
    fn parses_exiftool_json_with_ipct_and_xmp_keywords() {
        let json = r#"[
            {
                "SourceFile": "D:/photos/a.jpg",
                "Title": "Golden Gate Sunrise",
                "Description": "Fog drifts under the bridge.",
                "ImageDescription": "fallback",
                "Keywords": ["bridge", "sunrise"],
                "Subject": ["sunrise", "fog"],
                "ImageSize": "6000x4000"
            }
        ]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert_eq!(parsed.title.as_deref(), Some("Golden Gate Sunrise"));
        assert_eq!(parsed.caption.as_deref(), Some("Fog drifts under the bridge."));
        assert_eq!(parsed.keywords, vec!["bridge", "sunrise", "fog"]);
        assert_eq!(parsed.dims, Some((6000, 4000)));
    }

    #[test]
    fn parses_single_string_keywords() {
        let json = r#"[{"Keywords": "one, two, three"}]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert_eq!(parsed.keywords, vec!["one", "two", "three"]);
    }

    #[test]
    fn empty_metadata_writes_only_the_base_flags() {
        let args = write_args(&Metadata::default());
        assert_eq!(
            args,
            vec!["-P", "-overwrite_original", "-charset", "iptc=utf8", "-m"]
        );
    }

    #[test]
    fn caption_without_title_skips_title_tags() {
        let md = Metadata {
            title: "   ".into(),
            caption: "Fog over the bay at dawn.".into(),
            keywords: vec![],
        };
        let args = write_args(&md);
        assert!(args.contains(&"-EXIF:ImageDescription=Fog over the bay at dawn.".to_string()));
        assert!(args.contains(&"-IPTC:Caption-Abstract=Fog over the bay at dawn.".to_string()));
        assert!(args.contains(&"-XMP-dc:Description=Fog over the bay at dawn.".to_string()));
        assert!(!args.iter().any(|arg| arg.starts_with("-XMP-dc:Title")));
        assert!(!args.iter().any(|arg| arg.starts_with("-IPTC:ObjectName")));
        assert!(!args.iter().any(|arg| arg.starts_with("-EXIF:XPTitle")));
    }

    #[test]
    fn keyword_arguments_use_replacement_not_append() {
        let md = Metadata {
            title: "Three Word Title".into(),
            caption: String::new(),
            keywords: vec!["  bridge ".into(), "   ".into(), " sunrise ".into()],
        };
        let args = write_args(&md);
        assert!(args.contains(&"-IPTC:Keywords=bridge".to_string()));
        assert!(args.contains(&"-IPTC:Keywords=sunrise".to_string()));
        assert!(args.contains(&"-XMP-dc:Subject=bridge".to_string()));
        assert!(args.contains(&"-XMP-dc:Subject=sunrise".to_string()));
        // "+=" would make ExifTool keep the previous keyword list.
        assert!(!args.iter().any(|arg| arg.contains("+=")));
        assert_eq!(
            args.iter().filter(|arg| arg.starts_with("-IPTC:Keywords")).count(),
            2
        );
        assert_eq!(
            args.iter().filter(|arg| arg.starts_with("-XMP-dc:Subject")).count(),
            2
        );
    }

    #[test]
    fn unicode_values_are_preserved_in_arguments() {
        let md = Metadata {
            title: "Café Terrace at Night".into(),
            caption: "Café tables under warm evening light in an old town square.".into(),
            keywords: vec!["café".into(), "naïve".into(), "北京".into()],
        };
        let args = write_args(&md);
        assert!(args.contains(&"-XMP-dc:Title=Café Terrace at Night".to_string()));
        assert!(args.contains(&"-IPTC:Keywords=café".to_string()));
        assert!(args.contains(&"-IPTC:Keywords=北京".to_string()));
        assert!(args.contains(&"-XMP-dc:Subject=naïve".to_string()));
    }

    #[test]
    fn read_prefers_xmp_then_exif_then_iptc_fields() {
        let json = r#"[{
            "Title": "XMP title",
            "XPTitle": "XP title",
            "ObjectName": "Object name",
            "Description": "XMP description",
            "Caption-Abstract": "IPTC caption",
            "ImageDescription": "EXIF description"
        }]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert_eq!(parsed.title.as_deref(), Some("XMP title"));
        assert_eq!(parsed.caption.as_deref(), Some("XMP description"));
    }

    #[test]
    fn read_falls_back_through_title_and_caption_tags() {
        let json = r#"[{"XPTitle":"XP title","ObjectName":"Object name","Caption-Abstract":"IPTC caption","ImageDescription":"EXIF description"}]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert_eq!(parsed.title.as_deref(), Some("XP title"));
        assert_eq!(parsed.caption.as_deref(), Some("IPTC caption"));

        let json = r#"[{"ObjectName":"Object name","ImageDescription":"EXIF description"}]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert_eq!(parsed.title.as_deref(), Some("Object name"));
        assert_eq!(parsed.caption.as_deref(), Some("EXIF description"));

        let json = r#"[{"Title":"","XPTitle":"  ","Keywords":[]}]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert!(parsed.title.is_none(), "empty values fall through");
        assert!(parsed.caption.is_none());
    }

    #[test]
    fn read_merges_and_deduplicates_keyword_sources() {
        let json = r#"[{"Keywords":["Bridge","sunrise"],"Subject":["bridge","fog"]}]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert_eq!(parsed.keywords, vec!["Bridge", "sunrise", "fog"]);
    }

    #[test]
    fn read_ignores_invalid_values() {
        let json = r#"[{
            "Title": null,
            "Keywords": 42,
            "Subject": [1, true, "ok"],
            "ImageSize": "6000X4000"
        }]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert!(parsed.title.is_none());
        assert_eq!(parsed.keywords, vec!["ok"]);
        assert!(parsed.dims.is_none(), "uppercase X is not a valid separator");

        let parsed = parse_read_json(r#"[{"ImageSize":"abc"}]"#).expect("valid json");
        assert!(parsed.dims.is_none());

        let parsed = parse_read_json(r#"[{"ImageSize":"1024x768"}]"#).expect("valid json");
        assert_eq!(parsed.dims, Some((1024, 768)));
    }

    #[test]
    fn read_rejects_invalid_payloads() {
        assert!(parse_read_json("not json").is_err());
        assert!(parse_read_json("[]").is_err());
        assert!(parse_read_json("{}").is_err());
    }
}
