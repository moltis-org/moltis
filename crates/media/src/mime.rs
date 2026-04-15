use image::ImageFormat;

/// MIME detection via buffer sniffing with header fallback.
pub fn detect_mime(buffer: &[u8], headers: Option<&str>) -> String {
    if let Some(mime) = sniff_image_mime(buffer) {
        return mime.to_string();
    }

    if buffer.starts_with(b"%PDF-") {
        return "application/pdf".to_string();
    }
    if buffer.starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
        return "application/zip".to_string();
    }
    if buffer.starts_with(&[0x1F, 0x8B]) {
        return "application/gzip".to_string();
    }

    let trimmed = trim_ascii_start(buffer);
    if trimmed.starts_with(b"{") || trimmed.starts_with(b"[") {
        return "application/json".to_string();
    }
    if trimmed.starts_with(b"<!DOCTYPE html") || trimmed.starts_with(b"<html") {
        return "text/html".to_string();
    }
    if trimmed.starts_with(b"<?xml") {
        return "application/xml".to_string();
    }
    if let Some(header) = headers.and_then(parse_content_type_header) {
        return header;
    }
    if std::str::from_utf8(buffer).is_ok() {
        return "text/plain".to_string();
    }

    "application/octet-stream".to_string()
}

fn sniff_image_mime(buffer: &[u8]) -> Option<&'static str> {
    let format = image::guess_format(buffer).ok()?;
    Some(match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Png => "image/png",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        ImageFormat::Bmp => "image/bmp",
        ImageFormat::Pnm => "image/x-portable-pixmap",
        ImageFormat::Tiff => "image/tiff",
        ImageFormat::Ico => "image/x-icon",
        ImageFormat::Avif => "image/avif",
        _ => return None,
    })
}

fn trim_ascii_start(buffer: &[u8]) -> &[u8] {
    let first_non_ws = buffer
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(buffer.len());
    &buffer[first_non_ws..]
}

fn parse_content_type_header(header: &str) -> Option<String> {
    let raw = header.trim();
    let value = raw
        .split_once(':')
        .map(|(name, value)| {
            if name.trim().eq_ignore_ascii_case("content-type") {
                value
            } else {
                raw
            }
        })
        .unwrap_or(raw)
        .trim();
    let mime = value.split(';').next()?.trim().to_ascii_lowercase();
    if mime.is_empty() {
        None
    } else {
        Some(mime)
    }
}

/// Map a MIME type to its canonical file extension.
pub fn extension_for_mime(mime: &str) -> &str {
    match mime {
        // Images
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/x-portable-pixmap" => "ppm",
        // Audio / Video
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/webm" => "webm",
        "video/mp4" => "mp4",
        // Documents
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "application/json" => "json",
        "application/zip" => "zip",
        "application/gzip" => "gz",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.ms-excel" => "xls",
        "application/msword" => "doc",
        "text/html" => "html",
        "text/xml" | "application/xml" => "xml",
        "application/rtf" => "rtf",
        "text/markdown" => "md",
        _ => "bin",
    }
}

/// Map a file extension (without leading dot) to its MIME type.
///
/// Delegates to `mime_guess` with manual overrides for extensions it
/// doesn't cover (e.g. `text`, `log`, `ppm`).
pub fn mime_from_extension(ext: &str) -> Option<&'static str> {
    let lower = ext.to_ascii_lowercase();
    match lower.as_str() {
        "text" | "log" => return Some("text/plain"),
        "ppm" => return Some("image/x-portable-pixmap"),
        _ => {},
    }
    mime_guess::from_ext(&lower).first_raw()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_for_mime_covers_images() {
        assert_eq!(extension_for_mime("image/png"), "png");
        assert_eq!(extension_for_mime("image/jpeg"), "jpg");
        assert_eq!(extension_for_mime("image/gif"), "gif");
        assert_eq!(extension_for_mime("image/webp"), "webp");
        assert_eq!(extension_for_mime("image/x-portable-pixmap"), "ppm");
    }

    #[test]
    fn extension_for_mime_covers_documents() {
        assert_eq!(extension_for_mime("application/pdf"), "pdf");
        assert_eq!(extension_for_mime("text/plain"), "txt");
        assert_eq!(extension_for_mime("text/csv"), "csv");
        assert_eq!(extension_for_mime("application/json"), "json");
        assert_eq!(extension_for_mime("application/zip"), "zip");
        assert_eq!(
            extension_for_mime(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            ),
            "docx"
        );
        assert_eq!(
            extension_for_mime("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            "xlsx"
        );
    }

    #[test]
    fn extension_for_mime_unknown_returns_bin() {
        assert_eq!(extension_for_mime("application/octet-stream"), "bin");
        assert_eq!(extension_for_mime("something/unknown"), "bin");
    }

    #[test]
    fn detect_mime_sniffs_png_bytes() {
        let png = [
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(detect_mime(&png, None), "image/png");
    }

    #[test]
    fn detect_mime_prefers_sniffed_bytes_over_header() {
        let pdf = b"%PDF-1.7\n";
        assert_eq!(detect_mime(pdf, Some("image/png")), "application/pdf");
    }

    #[test]
    fn detect_mime_falls_back_to_header() {
        let data = [0x00, 0x01, 0x02, 0x03];
        assert_eq!(
            detect_mime(&data, Some("Content-Type: image/webp; charset=binary")),
            "image/webp"
        );
    }

    #[test]
    fn mime_from_extension_covers_images() {
        assert_eq!(mime_from_extension("png"), Some("image/png"));
        assert_eq!(mime_from_extension("PNG"), Some("image/png"));
        assert_eq!(mime_from_extension("jpg"), Some("image/jpeg"));
        assert_eq!(mime_from_extension("jpeg"), Some("image/jpeg"));
        assert_eq!(mime_from_extension("gif"), Some("image/gif"));
        assert_eq!(mime_from_extension("webp"), Some("image/webp"));
        assert_eq!(mime_from_extension("ppm"), Some("image/x-portable-pixmap"));
    }

    #[test]
    fn mime_from_extension_covers_documents() {
        assert_eq!(mime_from_extension("pdf"), Some("application/pdf"));
        assert_eq!(mime_from_extension("txt"), Some("text/plain"));
        assert_eq!(mime_from_extension("csv"), Some("text/csv"));
        assert_eq!(mime_from_extension("json"), Some("application/json"));
        assert_eq!(mime_from_extension("zip"), Some("application/zip"));
        assert_eq!(
            mime_from_extension("docx"),
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        );
        assert_eq!(
            mime_from_extension("xlsx"),
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
        );
    }

    #[test]
    fn mime_from_extension_unknown_returns_none() {
        assert_eq!(mime_from_extension("zzz_nope"), None);
        assert_eq!(mime_from_extension("qqqq"), None);
    }

    #[test]
    fn mime_from_extension_extras_from_mime_guess() {
        // mime_guess covers formats our old manual table didn't.
        assert_eq!(mime_from_extension("bmp"), Some("image/bmp"));
        assert_eq!(mime_from_extension("svg"), Some("image/svg+xml"));
        assert_eq!(mime_from_extension("tar"), Some("application/x-tar"));
    }

    #[test]
    fn round_trip_image_types() {
        for ext in &["png", "jpg", "gif", "webp", "ppm"] {
            let mime = mime_from_extension(ext).unwrap();
            let back = extension_for_mime(mime);
            // jpg -> image/jpeg -> jpg (not jpeg)
            if *ext == "jpg" {
                assert_eq!(back, "jpg");
            } else {
                assert_eq!(back, *ext);
            }
        }
    }

    #[test]
    fn round_trip_document_types() {
        for ext in &["pdf", "txt", "csv", "json", "zip", "docx", "xlsx"] {
            let mime = mime_from_extension(ext).unwrap();
            let back = extension_for_mime(mime);
            assert_eq!(back, *ext);
        }
    }
}
