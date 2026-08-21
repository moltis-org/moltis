use {
    base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD},
    moltis_channels::{
        ChannelDocumentFile, ChannelEventSink, ChannelReplyTarget, SavedChannelFile,
    },
    std::sync::Arc,
};

pub(super) const MAX_SAVED_INBOUND_FILE_BYTES: usize = 20 * 1024 * 1024;

fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        .take(160)
        .collect();
    sanitized.trim_start_matches('.').to_string()
}

fn saved_filename(display_name: Option<&str>, mime: &str, file_sha256: Option<&[u8]>) -> String {
    let extension = moltis_media::mime::extension_for_mime(mime);
    let base = display_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(sanitize_filename)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("whatsapp-file.{extension}"));
    let base = if extension != "bin"
        && !base
            .to_ascii_lowercase()
            .ends_with(&format!(".{extension}"))
    {
        format!("{base}.{extension}")
    } else {
        base
    };
    let prefix = file_sha256
        .map(|hash| URL_SAFE_NO_PAD.encode(hash))
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(16).collect::<String>())
        .unwrap_or_else(|| {
            time::OffsetDateTime::now_utc()
                .unix_timestamp_nanos()
                .to_string()
        });
    format!("{prefix}_{base}")
}

fn document_metadata(
    saved: &SavedChannelFile,
    display_name: Option<&str>,
    mime: &str,
    size_bytes: usize,
) -> ChannelDocumentFile {
    ChannelDocumentFile {
        display_name: display_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&saved.filename)
            .to_string(),
        stored_filename: saved.filename.clone(),
        mime_type: mime.to_string(),
        size_bytes: u64::try_from(size_bytes).ok(),
    }
}

pub(super) async fn save_inbound_file(
    sink: &Arc<dyn ChannelEventSink>,
    reply_to: &ChannelReplyTarget,
    data: &[u8],
    display_name: Option<&str>,
    mime: &str,
    file_sha256: Option<&[u8]>,
) -> Option<ChannelDocumentFile> {
    let filename = saved_filename(display_name, mime, file_sha256);
    let saved = sink
        .save_channel_attachment(data, &filename, reply_to)
        .await?;
    Some(document_metadata(&saved, display_name, mime, data.len()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn saved_filename_is_unique_and_sanitized() {
        let filename = saved_filename(
            Some("../../blood result 2026"),
            "application/pdf",
            Some(&[1, 2, 3, 4]),
        );

        assert_eq!(filename, "AQIDBA_bloodresult2026.pdf");
        assert!(!filename.contains('/'));
    }

    #[test]
    fn saved_filename_is_bounded() {
        let long_name = format!("{}.pdf", "a".repeat(300));
        let filename = saved_filename(Some(&long_name), "application/pdf", Some(&[1; 32]));

        assert!(filename.len() <= 181);
        assert!(filename.ends_with(".pdf"));
    }

    #[test]
    fn document_metadata_preserves_display_name() {
        let saved = SavedChannelFile {
            filename: "AQIDBA_result.pdf".to_string(),
            media_ref: "media/main/AQIDBA_result.pdf".to_string(),
            absolute_path: "/tmp/AQIDBA_result.pdf".to_string(),
        };

        let document = document_metadata(&saved, Some("Blood result.pdf"), "application/pdf", 42);

        assert_eq!(document.display_name, "Blood result.pdf");
        assert_eq!(document.stored_filename, saved.filename);
        assert_eq!(document.mime_type, "application/pdf");
        assert_eq!(document.size_bytes, Some(42));
    }
}
