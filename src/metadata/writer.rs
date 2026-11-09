use crate::ai::prompt::KEYWORD_TARGET;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
    pub category: String,
}

/// Rights and identity fields configured once in Settings and stamped onto
/// every saved image. They never come from the model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rights {
    pub creator: String,
    pub copyright: String,
    pub credit_line: String,
    pub usage_terms: String,
}

impl Rights {
    pub fn is_empty(&self) -> bool {
        self.creator.trim().is_empty()
            && self.copyright.trim().is_empty()
            && self.credit_line.trim().is_empty()
            && self.usage_terms.trim().is_empty()
    }
}

/// ExifTool arguments for the configured rights fields. Blank values are
/// skipped so an empty setting never erases what the file already has.
/// Remove both keyword lists. Used only when the user explicitly clears an
/// image's keywords; an empty list otherwise leaves the file's tags alone.
pub fn clear_keyword_args() -> Vec<String> {
    vec![
        "-IPTC:Keywords=".to_string(),
        "-XMP-dc:Subject=".to_string(),
    ]
}

/// ExifTool arguments for MP4/MOV metadata. XMP is the interchange format used
/// by Adobe, while QuickTime Keys is used by Apple and other video tools. Both
/// are written so the same file carries usable metadata in either pipeline.
pub fn video_write_args(md: &Metadata, rights: &Rights, clear_keywords: bool) -> Vec<String> {
    let mut args: Vec<String> = vec!["-P".into(), "-overwrite_original".into(), "-m".into()];

    let title = md.title.trim();
    let caption = md.caption.trim();
    let description = if caption.is_empty() { title } else { caption };

    if !title.is_empty() {
        args.push(format!("-XMP-dc:Title={title}"));
        args.push(format!("-Keys:Title={title}"));
    }
    if !description.is_empty() {
        args.push(format!("-XMP-dc:Description={description}"));
        args.push(format!("-Keys:Description={description}"));
    }

    let category = md.category.trim();
    if !category.is_empty() {
        args.push(format!("-XMP-photoshop:Category={category}"));
    }

    let keywords: Vec<&str> = md
        .keywords
        .iter()
        .map(|keyword| keyword.trim())
        .filter(|keyword| !keyword.is_empty())
        .take(KEYWORD_TARGET)
        .collect();
    for keyword in &keywords {
        args.push(format!("-XMP-dc:Subject={keyword}"));
    }
    if !keywords.is_empty() {
        args.push(format!("-Keys:Keywords={}", keywords.join(", ")));
    }

    let creator = rights.creator.trim();
    if !creator.is_empty() {
        args.push(format!("-XMP-dc:Creator={creator}"));
        args.push(format!("-Keys:Author={creator}"));
    }
    let copyright = rights.copyright.trim();
    if !copyright.is_empty() {
        args.push(format!("-XMP-dc:Rights={copyright}"));
        args.push(format!("-Keys:Copyright={copyright}"));
    }
    let credit_line = rights.credit_line.trim();
    if !credit_line.is_empty() {
        args.push(format!("-XMP-photoshop:Credit={credit_line}"));
    }
    let usage_terms = rights.usage_terms.trim();
    if !usage_terms.is_empty() {
        args.push(format!("-XMP-xmpRights:UsageTerms={usage_terms}"));
    }

    if clear_keywords {
        args.push("-XMP-dc:Subject=".into());
        args.push("-Keys:Keywords=".into());
    }

    args
}

pub fn rights_args(rights: &Rights) -> Vec<String> {
    let mut args = Vec::new();

    let creator = rights.creator.trim();
    if !creator.is_empty() {
        args.push(format!("-XMP-dc:Creator={creator}"));
        args.push(format!("-IPTC:By-line={creator}"));
    }

    let copyright = rights.copyright.trim();
    if !copyright.is_empty() {
        args.push(format!("-XMP-dc:Rights={copyright}"));
        args.push(format!("-IPTC:CopyrightNotice={copyright}"));
    }

    let credit_line = rights.credit_line.trim();
    if !credit_line.is_empty() {
        args.push(format!("-XMP-photoshop:Credit={credit_line}"));
        args.push(format!("-IPTC:Credit={credit_line}"));
    }

    let usage_terms = rights.usage_terms.trim();
    if !usage_terms.is_empty() {
        args.push(format!("-XMP-xmpRights:UsageTerms={usage_terms}"));
    }

    args
}

/// Build the exiftool argument list used to write stock-compliant metadata.
///
/// Title  -> XMP-dc:Title, IPTC:ObjectName, EXIF:XPTitle
/// Caption-> EXIF:ImageDescription, IPTC:Caption-Abstract, XMP-dc:Description
/// Words  -> IPTC:Keywords / XMP-dc:Subject (repeating values)
/// Category -> IPTC:Category, XMP-photoshop:Category
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

    let category = md.category.trim();
    if !category.is_empty() {
        args.push(format!("-IPTC:Category={category}"));
        args.push(format!("-XMP-photoshop:Category={category}"));
    }

    let keywords: Vec<&str> = md
        .keywords
        .iter()
        .map(|keyword| keyword.trim())
        .filter(|keyword| !keyword.is_empty())
        .take(KEYWORD_TARGET)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_all_microstock_tags() {
        let md = Metadata {
            title: "Golden Gate Sunrise".into(),
            caption: "Fog drifts under the Golden Gate Bridge at sunrise in San Francisco.".into(),
            keywords: vec!["bridge".into(), "sunrise".into(), "fog".into()],
            category: "Nature".into(),
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
        assert!(args.contains(&"-IPTC:Category=Nature".to_string()));
        assert!(args.contains(&"-XMP-photoshop:Category=Nature".to_string()));
        assert!(!args.iter().any(|arg| arg.contains("+=")));
        assert!(args.contains(&"-overwrite_original".to_string()));
    }

    #[test]
    fn writes_the_configured_rights_fields() {
        let rights = Rights {
            creator: "Jane Doe".into(),
            copyright: "© 2026 Jane Doe".into(),
            credit_line: "Jane Doe".into(),
            usage_terms: "Commercial use only".into(),
        };
        let args = rights_args(&rights);
        assert!(args.contains(&"-XMP-dc:Creator=Jane Doe".to_string()));
        assert!(args.contains(&"-IPTC:By-line=Jane Doe".to_string()));
        assert!(args.contains(&"-XMP-dc:Rights=© 2026 Jane Doe".to_string()));
        assert!(args.contains(&"-IPTC:CopyrightNotice=© 2026 Jane Doe".to_string()));
        assert!(args.contains(&"-XMP-photoshop:Credit=Jane Doe".to_string()));
        assert!(args.contains(&"-IPTC:Credit=Jane Doe".to_string()));
        assert!(args.contains(&"-XMP-xmpRights:UsageTerms=Commercial use only".to_string()));
    }

    #[test]
    fn blank_rights_fields_are_skipped() {
        assert!(rights_args(&Rights::default()).is_empty());
        let rights = Rights {
            creator: "  ".into(),
            copyright: String::new(),
            credit_line: "Jane Doe".into(),
            usage_terms: "  ".into(),
        };
        let args = rights_args(&rights);
        assert_eq!(args.len(), 2);
        assert!(args.contains(&"-XMP-photoshop:Credit=Jane Doe".to_string()));
    }

    #[test]
    fn falls_back_to_title_for_image_description() {
        let md = Metadata {
            title: "Lone Tree".into(),
            caption: String::new(),
            keywords: vec![],
            category: String::new(),
        };
        let args = write_args(&md);
        assert!(args.contains(&"-EXIF:ImageDescription=Lone Tree".to_string()));
        assert!(!args.iter().any(|arg| arg.starts_with("-IPTC:Keywords")));
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
            category: String::new(),
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
            category: String::new(),
        };
        let args = write_args(&md);
        assert!(args.contains(&"-IPTC:Keywords=bridge".to_string()));
        assert!(args.contains(&"-IPTC:Keywords=sunrise".to_string()));
        assert!(args.contains(&"-XMP-dc:Subject=bridge".to_string()));
        assert!(args.contains(&"-XMP-dc:Subject=sunrise".to_string()));
        // "+=" would make ExifTool keep the previous keyword list.
        assert!(!args.iter().any(|arg| arg.contains("+=")));
        assert_eq!(
            args.iter()
                .filter(|arg| arg.starts_with("-IPTC:Keywords"))
                .count(),
            2
        );
        assert_eq!(
            args.iter()
                .filter(|arg| arg.starts_with("-XMP-dc:Subject"))
                .count(),
            2
        );
    }

    #[test]
    fn video_metadata_writes_xmp_and_quicktime_keys() {
        let md = Metadata {
            title: "Harbour at Dawn".into(),
            caption: "Waves break against the stone pier.".into(),
            keywords: vec!["harbour".into(), "dawn".into(), "ocean".into()],
            category: "Nature".into(),
        };
        let rights = Rights {
            creator: "Jane Doe".into(),
            copyright: "Copyright 2026 Jane Doe".into(),
            credit_line: "Jane Doe".into(),
            usage_terms: "Commercial use".into(),
        };
        let args = video_write_args(&md, &rights, false);

        assert!(args.contains(&"-XMP-dc:Title=Harbour at Dawn".to_string()));
        assert!(args.contains(&"-Keys:Title=Harbour at Dawn".to_string()));
        assert!(
            args.contains(&"-XMP-dc:Description=Waves break against the stone pier.".to_string())
        );
        assert!(args.contains(&"-Keys:Description=Waves break against the stone pier.".to_string()));
        assert!(args.contains(&"-XMP-dc:Subject=harbour".to_string()));
        assert!(args.contains(&"-XMP-dc:Subject=ocean".to_string()));
        assert!(args.contains(&"-Keys:Keywords=harbour, dawn, ocean".to_string()));
        assert!(args.contains(&"-XMP-photoshop:Category=Nature".to_string()));
        assert!(args.contains(&"-XMP-dc:Creator=Jane Doe".to_string()));
        assert!(args.contains(&"-Keys:Author=Jane Doe".to_string()));
        assert!(args.contains(&"-XMP-dc:Rights=Copyright 2026 Jane Doe".to_string()));
        assert!(args.contains(&"-Keys:Copyright=Copyright 2026 Jane Doe".to_string()));
        assert!(args.contains(&"-XMP-photoshop:Credit=Jane Doe".to_string()));
        assert!(args.contains(&"-XMP-xmpRights:UsageTerms=Commercial use".to_string()));
        assert!(!args.iter().any(|arg| arg.starts_with("-IPTC:")));
        assert!(!args.iter().any(|arg| arg.contains("+=")));
    }

    #[test]
    fn video_clear_keywords_removes_both_video_lists() {
        let args = video_write_args(&Metadata::default(), &Rights::default(), true);
        assert!(args.contains(&"-XMP-dc:Subject=".to_string()));
        assert!(args.contains(&"-Keys:Keywords=".to_string()));
    }

    #[test]
    fn unicode_values_are_preserved_in_arguments() {
        let md = Metadata {
            title: "Café Terrace at Night".into(),
            caption: "Café tables under warm evening light in an old town square.".into(),
            keywords: vec!["café".into(), "naïve".into(), "北京".into()],
            category: "Travel".into(),
        };
        let args = write_args(&md);
        assert!(args.contains(&"-XMP-dc:Title=Café Terrace at Night".to_string()));
        assert!(args.contains(&"-IPTC:Keywords=café".to_string()));
        assert!(args.contains(&"-IPTC:Keywords=北京".to_string()));
        assert!(args.contains(&"-XMP-dc:Subject=naïve".to_string()));
    }

    #[test]
    fn keywords_are_capped_at_fifty() {
        let keywords: Vec<String> = (0..70).map(|index| format!("word{index}")).collect();
        let md = Metadata {
            title: "Title".into(),
            caption: String::new(),
            keywords,
            category: String::new(),
        };
        let args = write_args(&md);
        assert_eq!(
            args.iter()
                .filter(|arg| arg.starts_with("-IPTC:Keywords="))
                .count(),
            KEYWORD_TARGET
        );
        assert_eq!(
            args.iter()
                .filter(|arg| arg.starts_with("-XMP-dc:Subject="))
                .count(),
            KEYWORD_TARGET
        );
    }
}
