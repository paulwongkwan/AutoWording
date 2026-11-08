//! JPEG orientation.
//!
//! Photos from phones and many cameras store their pixels unrotated and add an
//! EXIF `Orientation` tag. Some tools strip EXIF but keep the same value in the
//! XMP packet, so XMP `tiff:Orientation` / `exif:Orientation` is read as a
//! fallback. The `image` crate does not apply orientation, so without this
//! module portrait shots are displayed and sent to the vision model sideways.
//! Tags are parsed directly from APP1 segments: no extra dependency and no
//! ExifTool process per thumbnail.

use image::DynamicImage;

const TAG_ORIENTATION: u16 = 0x0112;
const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// Orientation tag from JPEG bytes, if the file carries a valid 1..=8 value.
/// EXIF wins; XMP is consulted only when EXIF has no orientation tag.
pub fn from_jpeg(bytes: &[u8]) -> Option<u8> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }

    let mut exif = None;
    let mut xmp = None;
    let mut offset = 2usize;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xFF {
            offset += 1;
            continue;
        }
        let marker = bytes[offset + 1];
        // Standalone markers carry no length field.
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            offset += 2;
            continue;
        }
        // Metadata segments precede the scan data.
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        if length < 2 || offset + 2 + length > bytes.len() {
            return None;
        }
        let payload = &bytes[offset + 4..offset + 2 + length];
        if marker == 0xE1 {
            if payload.starts_with(b"Exif\0\0") {
                if exif.is_none() {
                    exif = tiff_orientation(&payload[6..]);
                }
            } else if payload.starts_with(XMP_HEADER) && xmp.is_none() {
                xmp = xmp_orientation(&payload[XMP_HEADER.len()..]);
            }
        }
        offset += 2 + length;
    }
    exif.or(xmp)
}

fn read_u16(bytes: &[u8], at: usize, little: bool) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    let pair = [slice[0], slice[1]];
    Some(if little {
        u16::from_le_bytes(pair)
    } else {
        u16::from_be_bytes(pair)
    })
}

fn read_u32(bytes: &[u8], at: usize, little: bool) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    let quad = [slice[0], slice[1], slice[2], slice[3]];
    Some(if little {
        u32::from_le_bytes(quad)
    } else {
        u32::from_be_bytes(quad)
    })
}

fn tiff_orientation(tiff: &[u8]) -> Option<u8> {
    if tiff.len() < 8 {
        return None;
    }
    let little = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    if read_u16(tiff, 2, little)? != 42 {
        return None;
    }
    let ifd = read_u32(tiff, 4, little)? as usize;
    let count = read_u16(tiff, ifd, little)? as usize;
    for index in 0..count {
        let entry = ifd + 2 + index * 12;
        if entry + 12 > tiff.len() {
            return None;
        }
        if read_u16(tiff, entry, little)? == TAG_ORIENTATION {
            let value = read_u16(tiff, entry + 8, little)?;
            return (1..=8).contains(&value).then_some(value as u8);
        }
    }
    None
}

/// Read `tiff:Orientation`, `exif:Orientation` or `xmp:Orientation` from an XMP
/// packet, as an element or as an attribute on `rdf:Description`.
fn xmp_orientation(xml: &[u8]) -> Option<u8> {
    for tag in [
        &b"tiff:Orientation"[..],
        &b"exif:Orientation"[..],
        &b"xmp:Orientation"[..],
    ] {
        if let Some(value) = property_value(xml, tag) {
            return Some(value);
        }
    }
    None
}

fn property_value(xml: &[u8], tag: &[u8]) -> Option<u8> {
    let mut from = 0usize;
    while let Some(found) = find(&xml[from..], tag) {
        let start = from + found;
        from = start + tag.len();
        // Skip the closing `</tiff:Orientation>` tag.
        if start > 0 && xml[start - 1] == b'/' {
            continue;
        }
        let rest = &xml[from..];
        let rest = &rest[rest
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(rest.len())..];
        match rest.first().copied() {
            // Attribute form: `tiff:Orientation="6"`.
            Some(b'=') => {
                let rest = &rest[1..];
                let rest = &rest[rest
                    .iter()
                    .position(|b| !b.is_ascii_whitespace())
                    .unwrap_or(rest.len())..];
                let quote = *rest.first()?;
                if quote != b'"' && quote != b'\'' {
                    continue;
                }
                let body = &rest[1..];
                let end = body.iter().position(|&b| b == quote)?;
                if let Some(value) = parse_orientation(&body[..end]) {
                    return Some(value);
                }
            }
            // Element form: `>6</tiff:Orientation>`.
            Some(b'>') => {
                let body = &rest[1..];
                let end = body.iter().position(|&b| b == b'<')?;
                if let Some(value) = parse_orientation(&body[..end]) {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_orientation(raw: &[u8]) -> Option<u8> {
    let value: u8 = std::str::from_utf8(raw).ok()?.trim().parse().ok()?;
    (1..=8).contains(&value).then_some(value)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Rotate/mirror decoded pixels into the orientation the camera intended.
pub fn apply(image: DynamicImage, orientation: u8) -> DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate90().flipv(),
        8 => image.rotate270(),
        _ => image,
    }
}

/// Build a JPEG carrying an orientation tag. Test-only: the encoder writes no
/// EXIF, so the APP1 segment is inserted right after SOI.
#[cfg(test)]
fn plain_jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([10, 120, 200]));
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90);
    encoder
        .encode(
            image.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgb8,
        )
        .expect("encode jpeg");
    bytes
}

#[cfg(test)]
fn insert_app1(jpeg: &[u8], payload: &[u8]) -> Vec<u8> {
    let length = 2 + payload.len();
    assert!(length <= u16::MAX as usize);
    let mut segment = Vec::with_capacity(2 + length);
    segment.extend_from_slice(&[0xFF, 0xE1]);
    segment.extend_from_slice(&(length as u16).to_be_bytes());
    segment.extend_from_slice(payload);

    let mut out = Vec::with_capacity(jpeg.len() + segment.len());
    out.extend_from_slice(&jpeg[..2]);
    out.extend_from_slice(&segment);
    out.extend_from_slice(&jpeg[2..]);
    out
}

#[cfg(test)]
fn exif_payload(tiff: &[u8]) -> Vec<u8> {
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(tiff);
    payload
}

#[cfg(test)]
fn xmp_payload(xml: &str) -> Vec<u8> {
    let mut payload = XMP_HEADER.to_vec();
    payload.extend_from_slice(xml.as_bytes());
    payload
}

#[cfg(test)]
pub(crate) fn jpeg_with_orientation(width: u32, height: u32, orientation: u16) -> Vec<u8> {
    let base = plain_jpeg(width, height);

    let mut tiff: Vec<u8> = Vec::new();
    tiff.extend_from_slice(b"II");
    tiff.extend_from_slice(&42u16.to_le_bytes());
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&1u16.to_le_bytes());
    tiff.extend_from_slice(&TAG_ORIENTATION.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_le_bytes());

    insert_app1(&base, &exif_payload(&tiff))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tag_from_app1_segment() {
        for orientation in [1u16, 3, 6, 8] {
            let jpeg = jpeg_with_orientation(8, 4, orientation);
            assert_eq!(from_jpeg(&jpeg), Some(orientation as u8));
        }
    }

    #[test]
    fn rejects_non_jpeg_and_out_of_range_tags() {
        assert_eq!(from_jpeg(&[]), None);
        assert_eq!(from_jpeg(b"not a jpeg"), None);
        assert_eq!(from_jpeg(&jpeg_with_orientation(4, 4, 9)), None);
        assert_eq!(from_jpeg(&jpeg_with_orientation(4, 4, 0)), None);
    }

    #[test]
    fn reads_big_endian_tiff_orientation() {
        let mut tiff: Vec<u8> = Vec::new();
        tiff.extend_from_slice(b"MM");
        tiff.extend_from_slice(&42u16.to_be_bytes());
        tiff.extend_from_slice(&8u32.to_be_bytes());
        tiff.extend_from_slice(&1u16.to_be_bytes());
        tiff.extend_from_slice(&TAG_ORIENTATION.to_be_bytes());
        tiff.extend_from_slice(&3u16.to_be_bytes());
        tiff.extend_from_slice(&1u32.to_be_bytes());
        tiff.extend_from_slice(&6u16.to_be_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(tiff_orientation(&tiff), Some(6));
    }

    #[test]
    fn rotation_and_transpose_swap_dimensions() {
        let base = DynamicImage::ImageRgb8(image::RgbImage::from_fn(3, 2, |x, y| {
            image::Rgb([x as u8, y as u8, 0])
        }));
        for orientation in [5u8, 6, 7, 8] {
            let rotated = apply(base.clone(), orientation);
            assert_eq!(
                (rotated.width(), rotated.height()),
                (2, 3),
                "orientation {orientation} should swap width and height"
            );
        }
        let upright = apply(base.clone(), 1);
        assert_eq!((upright.width(), upright.height()), (3, 2));
    }

    #[test]
    fn rotation_6_turns_the_top_row_into_the_right_column() {
        // 2x1 with distinct pixels: rotating clockwise puts the first pixel
        // at the top of a 1x2 image.
        let base = DynamicImage::ImageRgb8(image::RgbImage::from_fn(2, 1, |x, _| {
            image::Rgb([x as u8 * 100, 0, 0])
        }));
        let rotated = apply(base, 6).to_rgb8();
        assert_eq!((rotated.width(), rotated.height()), (1, 2));
        assert_eq!(
            rotated.get_pixel(0, 0).0[0],
            0,
            "left pixel moves to the top"
        );
        assert_eq!(rotated.get_pixel(0, 1).0[0], 100);
    }

    #[test]
    fn reads_xmp_orientation_when_exif_has_no_orientation_tag() {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&0u16.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());

        let jpeg = insert_app1(&plain_jpeg(8, 4), &exif_payload(&tiff));
        let jpeg = insert_app1(
            &jpeg,
            &xmp_payload(
                r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:tiff="http://ns.adobe.com/tiff/1.0/"><tiff:Orientation>6</tiff:Orientation></rdf:Description></rdf:RDF></x:xmpmeta>"#,
            ),
        );
        assert_eq!(from_jpeg(&jpeg), Some(6));
    }

    #[test]
    fn reads_xmp_orientation_attribute_and_exif_prefix() {
        let attribute = xmp_payload(
            r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:tiff="http://ns.adobe.com/tiff/1.0/" tiff:Orientation="8"/>"#,
        );
        assert_eq!(
            from_jpeg(&insert_app1(&plain_jpeg(8, 4), &attribute)),
            Some(8)
        );

        let exif_prefix = xmp_payload(
            r#"<rdf:Description xmlns:exif="http://ns.adobe.com/exif/1.0/"><exif:Orientation> 7 </exif:Orientation></rdf:Description>"#,
        );
        assert_eq!(
            from_jpeg(&insert_app1(&plain_jpeg(8, 4), &exif_prefix)),
            Some(7)
        );
    }

    #[test]
    fn exif_orientation_wins_over_xmp() {
        let jpeg = insert_app1(
            &jpeg_with_orientation(8, 4, 3),
            &xmp_payload(
                r#"<rdf:Description xmlns:tiff="http://ns.adobe.com/tiff/1.0/"><tiff:Orientation>6</tiff:Orientation></rdf:Description>"#,
            ),
        );
        assert_eq!(from_jpeg(&jpeg), Some(3));
    }

    #[test]
    fn ignores_invalid_or_missing_xmp_orientation() {
        for value in ["0", "9", "six", ""] {
            let xml = format!(
                r#"<rdf:Description xmlns:tiff="http://ns.adobe.com/tiff/1.0/"><tiff:Orientation>{value}</tiff:Orientation></rdf:Description>"#
            );
            assert_eq!(
                from_jpeg(&insert_app1(&plain_jpeg(8, 4), &xmp_payload(&xml))),
                None,
                "value {value:?} is not a usable orientation"
            );
        }

        let no_tag = xmp_payload(r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"/>"#);
        assert_eq!(from_jpeg(&insert_app1(&plain_jpeg(8, 4), &no_tag)), None);
    }
}
