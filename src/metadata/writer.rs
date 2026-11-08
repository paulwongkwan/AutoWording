use crate::ai::prompt::KEYWORD_TARGET;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    pub title: String,
    pub caption: String,
    pub keywords: Vec<String>,
}

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
    fn keywords_are_capped_at_fifty() {
        let keywords: Vec<String> = (0..70).map(|index| format!("word{index}")).collect();
        let md = Metadata {
            title: "Title".into(),
            caption: String::new(),
            keywords,
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
