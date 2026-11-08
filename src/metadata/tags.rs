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
    "-Orientation#",
];

fn orientation_number(value: &Value) -> Option<u8> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .and_then(|number| u8::try_from(number).ok())
        .filter(|number| (1..=8).contains(number))
}

/// True when the payload contains a usable Orientation value.
pub(crate) fn has_orientation(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    value
        .as_array()
        .and_then(|items| items.first())
        .map(has_orientation_in_object)
        .unwrap_or(false)
}

pub(crate) fn has_orientation_in_object(object: &Value) -> bool {
    object
        .get("Orientation")
        .and_then(orientation_number)
        .is_some()
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
    parse_read_json_with_orientation(text, None)
}

/// Parse the output, using `fallback_orientation` (read from the JPEG's APP1
/// segments) when ExifTool reports no usable Orientation tag.
pub fn parse_read_json_with_orientation(
    text: &str,
    fallback_orientation: Option<u8>,
) -> Result<ExistingMetadata> {
    let value: Value = serde_json::from_str(text)?;
    let object = value
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| anyhow!("exiftool returned no metadata"))?;
    Ok(parse_read_object(object, fallback_orientation))
}

/// Parse one entry from exiftool's JSON array.
pub(crate) fn parse_read_object(
    object: &Value,
    fallback_orientation: Option<u8>,
) -> ExistingMetadata {
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
    let date = first_string(object, &["DateTimeOriginal", "CreateDate", "DateTime"]);

    let mut dims = object
        .get("ImageSize")
        .and_then(Value::as_str)
        .and_then(|size| {
            let (width, height) = size.split_once('x')?;
            Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
        });

    // ImageSize reports stored pixels; orientations 5-8 rotate the displayed
    // image, so the labels must swap to match what the user sees.
    let orientation = object
        .get("Orientation")
        .and_then(orientation_number)
        .or(fallback_orientation)
        .unwrap_or(1);
    if (5..=8).contains(&orientation) {
        dims = dims.map(|(width, height)| (height, width));
    }

    ExistingMetadata {
        title,
        caption,
        keywords,
        gps,
        date,
        dims,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_tag_swaps_dimensions_for_rotated_photos() {
        let json = r#"[{"ImageSize":"6000x4000","Orientation":6}]"#;
        assert_eq!(
            parse_read_json(json).expect("valid json").dims,
            Some((4000, 6000))
        );

        let json = r#"[{"ImageSize":"6000x4000","Orientation":"8"}]"#;
        assert_eq!(
            parse_read_json(json).expect("valid json").dims,
            Some((4000, 6000))
        );

        for upright in [
            r#"[{"ImageSize":"6000x4000","Orientation":1}]"#,
            r#"[{"ImageSize":"6000x4000","Orientation":9}]"#,
            r#"[{"ImageSize":"6000x4000"}]"#,
        ] {
            assert_eq!(
                parse_read_json(upright).expect("valid json").dims,
                Some((6000, 4000)),
                "upright or unknown orientation keeps ImageSize order"
            );
        }
    }

    #[test]
    fn missing_orientation_uses_the_xmp_fallback_for_dimensions() {
        let json = r#"[{"ImageSize":"6000x4000"}]"#;
        assert_eq!(
            parse_read_json_with_orientation(json, Some(6))
                .expect("valid json")
                .dims,
            Some((4000, 6000))
        );
        assert_eq!(
            parse_read_json_with_orientation(json, Some(9))
                .expect("valid json")
                .dims,
            Some((6000, 4000)),
            "an invalid fallback keeps the stored dimensions"
        );
    }

    #[test]
    fn exif_orientation_wins_over_the_xmp_fallback() {
        let json = r#"[{"ImageSize":"6000x4000","Orientation":1}]"#;
        assert_eq!(
            parse_read_json_with_orientation(json, Some(6))
                .expect("valid json")
                .dims,
            Some((6000, 4000)),
            "EXIF says upright, so the XMP rotation must not be applied"
        );
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
                "GPSPosition": "37 deg 48' 30.00\" N, 122 deg 28' 10.00\" W",
                "DateTimeOriginal": "2026:10:31 17:42:11",
                "ImageSize": "6000x4000"
            }
        ]"#;
        let parsed = parse_read_json(json).expect("valid json");
        assert_eq!(parsed.title.as_deref(), Some("Golden Gate Sunrise"));
        assert_eq!(
            parsed.caption.as_deref(),
            Some("Fog drifts under the bridge.")
        );
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
        assert!(
            parsed.dims.is_none(),
            "uppercase X is not a valid separator"
        );

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
