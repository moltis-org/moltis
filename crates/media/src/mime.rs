/// MIME detection via buffer sniffing with header fallback.
pub fn detect_mime(_buffer: &[u8], _headers: Option<&str>) -> String {
    todo!("sniff magic bytes, fall back to content-type header")
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
