use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct ExistingMetadata {
    pub title: Option<String>,
    pub caption: Option<String>,
    pub keywords: Vec<String>,
    pub category: Option<String>,
    pub creator: Option<String>,
    pub copyright: Option<String>,
    pub credit_line: Option<String>,
    pub usage_terms: Option<String>,
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
    "-IPTC:Category",
    "-XMP-photoshop:Category",
    "-XMP-dc:Creator",
    "-XMP-dc:Rights",
    "-XMP-photoshop:Credit",
    "-XMP-xmpRights:UsageTerms",
    "-GPSPosition",
    "-GPSLatitude",
    "-GPSLongitude",
    "-DateTimeOriginal",
    "-CreateDate",
    "-DateTime",
    "-ImageSize",
    "-Orientation#",
];

/// Group-qualified tags for MP4/MOV files. XMP is written by Adobe tools;
/// QuickTime Keys is written by Apple tools. Reading both avoids depending on
/// ExifTool's preferred-group order.
pub const VIDEO_READ_TAGS: &[&str] = &[
    "-XMP-dc:Title",
    "-Keys:Title",
    "-ItemList:Title",
    "-UserData:Title",
    "-XMP-dc:Description",
    "-Keys:Description",
    "-ItemList:Description",
    "-UserData:Description",
    "-XMP-dc:Subject",
    "-Keys:Keywords",
    "-XMP-photoshop:Category",
    "-XMP-dc:Creator",
    "-Keys:Author",
    "-XMP-dc:Rights",
    "-Keys:Copyright",
    "-XMP-photoshop:Credit",
    "-XMP-xmpRights:UsageTerms",
    "-ImageSize",
    "-CreateDate",
    "-Keys:CreationDate",
    "-GPSPosition",
    "-GPSLatitude",
    "-GPSLongitude",
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

/// Parse the group-qualified JSON produced by `exiftool -j -G1` for video.
pub fn parse_video_read_json(text: &str) -> Result<ExistingMetadata> {
    let value: Value = serde_json::from_str(text)?;
    let object = value
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| anyhow!("exiftool returned no metadata"))?;
    Ok(parse_video_read_object(object))
}

/// Parse one group-qualified video entry. XMP wins over QuickTime Keys, and
/// both keyword sources are merged without case-insensitive duplicates.
pub(crate) fn parse_video_read_object(object: &Value) -> ExistingMetadata {
    let title = first_string(
        object,
        &[
            "XMP-dc:Title",
            "Keys:Title",
            "ItemList:Title",
            "UserData:Title",
        ],
    );
    let caption = first_string(
        object,
        &[
            "XMP-dc:Description",
            "Keys:Description",
            "ItemList:Description",
            "UserData:Description",
        ],
    );
    let category = first_string(object, &["XMP-photoshop:Category"]);
    let creator = first_string(object, &["XMP-dc:Creator", "Keys:Author"]);
    let copyright = first_string(object, &["XMP-dc:Rights", "Keys:Copyright"]);
    let credit_line = first_string(object, &["XMP-photoshop:Credit"]);
    let usage_terms = first_string(object, &["XMP-xmpRights:UsageTerms"]);

    let mut keywords: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for source in ["XMP-dc:Subject", "Keys:Keywords"] {
        for keyword in keyword_list(object, source) {
            if seen.insert(keyword.to_lowercase()) {
                keywords.push(keyword);
            }
        }
    }

    let dims = object
        .get("ImageSize")
        .or_else(|| object.get("Composite:ImageSize"))
        .or_else(|| object.get("File:ImageSize"))
        .or_else(|| object.get("QuickTime:ImageSize"))
        .and_then(Value::as_str)
        .and_then(|size| {
            let (width, height) = size.split_once('x')?;
            Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
        });

    ExistingMetadata {
        title,
        caption,
        keywords,
        category,
        creator,
        copyright,
        credit_line,
        usage_terms,
        gps: gps_location(object),
        date: first_string(
            object,
            &[
                "CreateDate",
                "QuickTime:CreateDate",
                "XMP-xmp:CreateDate",
                "Keys:CreationDate",
            ],
        ),
        dims,
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

    let category = first_string(object, &["Category"]);
    let creator = first_string(object, &["Creator"]);
    let copyright = first_string(object, &["Rights"]);
    let credit_line = first_string(object, &["Credit"]);
    let usage_terms = first_string(object, &["UsageTerms"]);

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
        category,
        creator,
        copyright,
        credit_line,
        usage_terms,
        gps,
        date,
        dims,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grouped_video_metadata_and_merges_keyword_sources() {
        let json = r#"[{
            "SourceFile": "D:/clips/harbour.mp4",
            "XMP-dc:Title": "Harbour at Dawn",
            "Keys:Title": "Fallback title",
            "XMP-dc:Description": "Waves break against the stone pier.",
            "Keys:Description": "Fallback description",
            "XMP-dc:Subject": ["harbour", "dawn"],
            "Keys:Keywords": "dawn, ocean, stone pier",
            "XMP-photoshop:Category": "Nature",
            "XMP-dc:Creator": "Jane Doe",
            "Keys:Author": "Fallback author",
            "XMP-dc:Rights": "Copyright 2026 Jane Doe",
            "Keys:Copyright": "Fallback copyright",
            "XMP-photoshop:Credit": "Jane Doe",
            "XMP-xmpRights:UsageTerms": "Commercial use",
            "Composite:ImageSize": "1920x1080",
            "CreateDate": "2026:11:09 10:00:00"
        }]"#;
        let parsed = parse_video_read_json(json).expect("valid video json");
        assert_eq!(parsed.title.as_deref(), Some("Harbour at Dawn"));
        assert_eq!(
            parsed.caption.as_deref(),
            Some("Waves break against the stone pier.")
        );
        assert_eq!(
            parsed.keywords,
            vec!["harbour", "dawn", "ocean", "stone pier"]
        );
        assert_eq!(parsed.category.as_deref(), Some("Nature"));
        assert_eq!(parsed.creator.as_deref(), Some("Jane Doe"));
        assert_eq!(parsed.copyright.as_deref(), Some("Copyright 2026 Jane Doe"));
        assert_eq!(parsed.credit_line.as_deref(), Some("Jane Doe"));
        assert_eq!(parsed.usage_terms.as_deref(), Some("Commercial use"));
        assert_eq!(parsed.dims, Some((1920, 1080)));
        assert_eq!(parsed.date.as_deref(), Some("2026:11:09 10:00:00"));
    }

    #[test]
    fn video_read_falls_back_to_quicktime_keys_and_rejects_empty_payloads() {
        let json = r#"[{
            "Keys:Title": "Keys title",
            "Keys:Description": "Keys description",
            "Keys:Keywords": "one, two",
            "XMP-xmp:CreateDate": "2026:11:09 10:00:00"
        }]"#;
        let parsed = parse_video_read_json(json).expect("keys fallback");
        assert_eq!(parsed.title.as_deref(), Some("Keys title"));
        assert_eq!(parsed.caption.as_deref(), Some("Keys description"));
        assert_eq!(parsed.keywords, vec!["one", "two"]);
        assert_eq!(parsed.date.as_deref(), Some("2026:11:09 10:00:00"));
        assert!(parse_video_read_json("[]").is_err());
    }

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
    fn reads_the_rights_fields_from_xmp() {
        let json = r#"[{"Creator":"Jane Doe","Rights":"© 2026 Jane Doe","Credit":"Jane Doe","UsageTerms":"Commercial use only"}]"#;
        let read = parse_read_json(json).expect("valid json");
        assert_eq!(read.creator.as_deref(), Some("Jane Doe"));
        assert_eq!(read.copyright.as_deref(), Some("© 2026 Jane Doe"));
        assert_eq!(read.credit_line.as_deref(), Some("Jane Doe"));
        assert_eq!(read.usage_terms.as_deref(), Some("Commercial use only"));
    }

    #[test]
    fn reads_the_ipct_category() {
        let json = r#"[{"Category":"Nature"}]"#;
        assert_eq!(
            parse_read_json(json)
                .expect("valid json")
                .category
                .as_deref(),
            Some("Nature")
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
