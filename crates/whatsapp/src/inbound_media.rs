use {
    base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD},
    moltis_channels::{
        ChannelDocumentFile, ChannelEventSink, ChannelReplyTarget, SavedChannelFile,
    },
    std::{
        io::{Cursor, Error as IoError, ErrorKind, Seek, SeekFrom, Write},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    },
    wacore::download::Downloadable,
    whatsapp_rust::client::Client,
};

pub(super) const MAX_SAVED_INBOUND_FILE_BYTES: usize = 20 * 1024 * 1024;

/// A `Write + Seek` sink that refuses to buffer more than `limit` bytes.
///
/// `Client::download` buffers the whole response before any size check can run,
/// so a sender who understates `file_length` in the protobuf can force an
/// arbitrarily large allocation. Streaming into this buffer instead makes the
/// limit authoritative: the write that would cross it fails, aborting the
/// download mid-flight.
///
/// `download_to_writer` rewinds and replays the writer when it retries a failed
/// media connection, so seeking back to 0 must discard whatever was already
/// buffered rather than leaving it to be overwritten in place.
struct BoundedBuffer {
    inner: Cursor<Vec<u8>>,
    limit: u64,
    exceeded: Arc<AtomicBool>,
    replace_on_write: bool,
    empty_result: bool,
}

impl BoundedBuffer {
    fn new(limit: usize, exceeded: Arc<AtomicBool>) -> Self {
        Self {
            inner: Cursor::new(Vec::new()),
            limit: limit as u64,
            exceeded,
            replace_on_write: false,
            empty_result: false,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        if self.empty_result {
            Vec::new()
        } else {
            self.inner.into_inner()
        }
    }
}

impl Write for BoundedBuffer {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.replace_on_write {
            self.inner.get_mut().clear();
            self.inner.set_position(0);
            self.replace_on_write = false;
        }
        self.empty_result = false;
        let end = self
            .inner
            .position()
            .checked_add(buffer.len() as u64)
            .ok_or_else(|| IoError::new(ErrorKind::FileTooLarge, "inbound file size overflow"))?;
        if end > self.limit {
            self.exceeded.store(true, Ordering::Release);
            return Err(IoError::new(
                ErrorKind::FileTooLarge,
                "inbound file exceeds size limit",
            ));
        }
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for BoundedBuffer {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let previous = self.inner.position();
        let next = self.inner.seek(position)?;
        if next > self.limit {
            self.exceeded.store(true, Ordering::Release);
            return Err(IoError::new(
                ErrorKind::FileTooLarge,
                "inbound file exceeds size limit",
            ));
        }
        if next == 0 {
            if previous == 0 && self.replace_on_write {
                self.empty_result = true;
            } else if previous > 0 && !self.inner.get_ref().is_empty() {
                self.replace_on_write = true;
            }
        }
        Ok(next)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum DownloadError {
    #[error("inbound file exceeds the {MAX_SAVED_INBOUND_FILE_BYTES}-byte limit")]
    TooLarge,
    #[error("downloaded file is empty")]
    Empty,
    #[error("WhatsApp media download failed")]
    Download,
}

/// Download WhatsApp media, aborting once `MAX_SAVED_INBOUND_FILE_BYTES` is
/// crossed instead of buffering the whole (sender-controlled) payload first.
pub(super) async fn download_bounded(
    client: &Client,
    downloadable: &dyn Downloadable,
) -> Result<Vec<u8>, DownloadError> {
    let exceeded = Arc::new(AtomicBool::new(false));
    let writer = BoundedBuffer::new(MAX_SAVED_INBOUND_FILE_BYTES, Arc::clone(&exceeded));
    let writer = client
        .download_to_writer(downloadable, writer)
        .await
        .map_err(|_| {
            if exceeded.load(Ordering::Acquire) {
                DownloadError::TooLarge
            } else {
                DownloadError::Download
            }
        })?;
    if exceeded.load(Ordering::Acquire) {
        return Err(DownloadError::TooLarge);
    }
    let data = writer.into_inner();
    if data.is_empty() {
        Err(DownloadError::Empty)
    } else {
        Ok(data)
    }
}

/// Normalize a sender-supplied MIME type down to a bare, well-formed `type/subtype`.
///
/// The raw value reaches the LLM context and the stored file's metadata, so
/// anything with parameters, control characters, or unexpected punctuation is
/// replaced with the generic binary type rather than passed through.
pub(super) fn safe_media_type(media_type: Option<&str>) -> String {
    let raw = media_type
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if raw.len() > 127 {
        return "application/octet-stream".to_string();
    }
    let normalized = raw.to_ascii_lowercase();
    let valid = normalized.split_once('/').is_some_and(|(kind, subtype)| {
        !kind.is_empty()
            && !subtype.is_empty()
            && !subtype.contains('/')
            && normalized.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '/' | '+' | '-' | '.')
            })
    });
    if valid {
        normalized
    } else {
        "application/octet-stream".to_string()
    }
}

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
    // Idempotent: callers already normalize, but the stored metadata must never
    // carry a raw sender-supplied MIME type even if a future caller forgets.
    let mime = safe_media_type(Some(mime));
    let filename = saved_filename(display_name, &mime, file_sha256);
    let saved = sink
        .save_channel_attachment(data, &filename, reply_to)
        .await?;
    Some(document_metadata(&saved, display_name, &mime, data.len()))
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

    #[test]
    fn bounded_buffer_accepts_the_limit_and_rejects_one_more_byte() {
        let exceeded = Arc::new(AtomicBool::new(false));
        let mut buffer = BoundedBuffer::new(4, Arc::clone(&exceeded));

        buffer.write_all(b"test").unwrap();
        assert_eq!(buffer.inner.get_ref(), b"test");
        assert!(buffer.write_all(b"!").is_err());
        assert!(exceeded.load(Ordering::Acquire));
        assert_eq!(buffer.inner.get_ref(), b"test");
    }

    #[test]
    fn bounded_buffer_replaces_bytes_when_a_download_is_retried() {
        let exceeded = Arc::new(AtomicBool::new(false));
        let mut buffer = BoundedBuffer::new(8, exceeded);

        buffer.write_all(b"stale").unwrap();
        buffer.seek(SeekFrom::Start(0)).unwrap();
        buffer.write_all(b"new").unwrap();
        buffer.seek(SeekFrom::Start(0)).unwrap();

        assert_eq!(buffer.into_inner(), b"new");
    }

    #[test]
    fn bounded_buffer_does_not_return_stale_bytes_for_an_empty_retry() {
        let exceeded = Arc::new(AtomicBool::new(false));
        let mut buffer = BoundedBuffer::new(8, exceeded);

        buffer.write_all(b"stale").unwrap();
        buffer.seek(SeekFrom::Start(0)).unwrap();
        buffer.seek(SeekFrom::Start(0)).unwrap();

        assert!(buffer.into_inner().is_empty());
    }

    #[test]
    fn safe_media_type_strips_parameters_and_rejects_malformed_values() {
        assert_eq!(
            safe_media_type(Some("Application/PDF; version=1.7")),
            "application/pdf"
        );
        assert_eq!(
            safe_media_type(Some("application/pdf\r\nmalicious: value")),
            "application/octet-stream"
        );
        assert_eq!(
            safe_media_type(Some("text//plain")),
            "application/octet-stream"
        );
        assert_eq!(
            safe_media_type(Some("notamime")),
            "application/octet-stream"
        );
        assert_eq!(safe_media_type(None), "application/octet-stream");
        assert_eq!(
            safe_media_type(Some(&"a".repeat(200))),
            "application/octet-stream"
        );
    }

    #[tokio::test]
    async fn save_inbound_file_never_stores_a_raw_sender_mime_type() {
        struct CapturingSink(std::sync::Mutex<Option<String>>);

        #[async_trait::async_trait]
        impl ChannelEventSink for CapturingSink {
            async fn emit(&self, _event: moltis_channels::ChannelEvent) {}

            async fn dispatch_to_chat(
                &self,
                _text: &str,
                _reply_to: ChannelReplyTarget,
                _meta: moltis_channels::ChannelMessageMeta,
            ) {
            }

            async fn dispatch_command(
                &self,
                _command: &str,
                _reply_to: ChannelReplyTarget,
                _sender_id: Option<&str>,
            ) -> moltis_channels::Result<String> {
                Ok(String::new())
            }

            async fn request_disable_account(
                &self,
                _channel_type: &str,
                _account_id: &str,
                _reason: &str,
            ) {
            }

            async fn save_channel_attachment(
                &self,
                _data: &[u8],
                filename: &str,
                _reply_to: &ChannelReplyTarget,
            ) -> Option<SavedChannelFile> {
                self.0.lock().unwrap().replace(filename.to_string());
                Some(SavedChannelFile {
                    filename: filename.to_string(),
                    media_ref: format!("media/test/{filename}"),
                    absolute_path: format!("/tmp/test/{filename}"),
                })
            }
        }

        let sink = Arc::new(CapturingSink(std::sync::Mutex::new(None)));
        let reply_to = ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Whatsapp,
            account_id: "test".to_string(),
            chat_id: "test-chat".to_string(),
            message_id: None,
            thread_id: None,
            ack_message_id: None,
        };

        let document = save_inbound_file(
            &(Arc::clone(&sink) as Arc<dyn ChannelEventSink>),
            &reply_to,
            b"%PDF-test",
            Some("report.pdf"),
            "Application/PDF; version=1.7",
            Some(&[1, 2, 3, 4]),
        )
        .await
        .unwrap();

        assert_eq!(document.mime_type, "application/pdf");
        assert_eq!(document.display_name, "report.pdf");
        assert_eq!(document.size_bytes, Some(9));
    }
}
