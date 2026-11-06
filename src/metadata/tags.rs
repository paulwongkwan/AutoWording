use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct ExistingMetadata {
    pub title: Option<String>,
    pub caption: Option<String>,
    pub keywords: Vec<String>,
    pub gps: Option<String>,
    pub date: Option<String>,
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
    "-GPSPosition",
    "-GPSLatitude",
    "-GPSLongitude",
    "-DateTimeOriginal",
    "-CreateDate",
    "-DateTime",
    "-ImageSize",
];

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

fn gps_location(object: &Value) -> Option<String> {
    if let Some(position) = first_string(object, &["GPSPosition"]) {
        return Some(position);
    }
    match (
        first_string(object, &["GPSLatitude"]),
        first_string(object, &["GPSLongitude"]),
    ) {
        (Some(latitude), Some(longitude)) => Some(format!("{latitude}, {longitude}")),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
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

    let gps = gps_location(object);
    let date = first_string(
        object,
        &["DateTimeOriginal", "CreateDate", "DateTime"],
    );

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
        gps,
        date,
        dims,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
                "GPSPosition": "37 deg 48' 30.00\" N, 122 deg 28' 10.00\" W",
                "DateTimeOriginal": "2026:10:31 17:42:11",
                "ImageSize": "6000x4000"
            }
        ]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert_eq!(parsed.title.as_deref(), Some("Golden Gate Sunrise"));
        assert_eq!(parsed.caption.as_deref(), Some("Fog drifts under the bridge."));
        assert_eq!(parsed.keywords, vec!["bridge", "sunrise", "fog"]);
        assert_eq!(
            parsed.gps.as_deref(),
            Some("37 deg 48' 30.00\" N, 122 deg 28' 10.00\" W")
        );
        assert_eq!(parsed.date.as_deref(), Some("2026:10:31 17:42:11"));
        assert_eq!(parsed.dims, Some((6000, 4000)));
    }

    #[test]
    fn parses_gps_lat_lon_and_capture_date_fallbacks() {
        let json = r#"[{
            "GPSLatitude": "51 deg 30' 0.00\" N",
            "GPSLongitude": "0 deg 7' 0.00\" W",
            "CreateDate": "2025:06:01 08:00:00"
        }]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert_eq!(
            parsed.gps.as_deref(),
            Some("51 deg 30' 0.00\" N, 0 deg 7' 0.00\" W")
        );
        assert_eq!(parsed.date.as_deref(), Some("2025:06:01 08:00:00"));
    }

    #[test]
    fn parses_single_string_keywords() {
        let json = r#"[{"Keywords": "one, two, three"}]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert_eq!(parsed.keywords, vec!["one", "two", "three"]);
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
