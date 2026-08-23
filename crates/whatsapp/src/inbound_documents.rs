use std::{
    io::{Cursor, Error as IoError, ErrorKind, Seek, SeekFrom, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use {
    async_trait::async_trait,
    moltis_channels::{
        ChannelDocumentFile, ChannelEventSink, ChannelReplyTarget, SavedChannelFile,
    },
    whatsapp_rust::client::Client,
};

pub(super) const MAX_INBOUND_DOCUMENT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DocumentReception {
    Disabled,
    Empty,
    TooLarge,
    Download,
}

pub(super) fn reception_for(enabled: bool, reported_size: Option<u64>) -> DocumentReception {
    if !enabled {
        return DocumentReception::Disabled;
    }
    match reported_size {
        Some(0) => DocumentReception::Empty,
        Some(size) if size > MAX_INBOUND_DOCUMENT_BYTES as u64 => DocumentReception::TooLarge,
        _ => DocumentReception::Download,
    }
}

fn safe_display_name(name: Option<&str>) -> String {
    let basename = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .and_then(|name| name.rsplit(['/', '\\']).next())
        .unwrap_or("document");
    let mut sanitized = String::with_capacity(basename.len().min(255));
    for character in basename.chars().filter(|character| !character.is_control()) {
        if sanitized.len() + character.len_utf8() > 255 {
            break;
        }
        sanitized.push(character);
    }
    if sanitized.trim().is_empty() {
        "document".to_string()
    } else {
        sanitized
    }
}

fn safe_media_type(media_type: Option<&str>) -> String {
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

fn safe_storage_name(media_type: &str) -> String {
    let extension = moltis_media::mime::extension_for_mime(media_type);
    let prefix = format!("{:032x}", rand::random::<u128>());
    format!("{prefix}_document.{extension}")
}

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
            .ok_or_else(|| IoError::new(ErrorKind::FileTooLarge, "document size overflow"))?;
        if end > self.limit {
            self.exceeded.store(true, Ordering::Release);
            return Err(IoError::new(
                ErrorKind::FileTooLarge,
                "inbound document exceeds size limit",
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
                "inbound document exceeds size limit",
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
    #[error("document exceeds the {MAX_INBOUND_DOCUMENT_BYTES}-byte limit")]
    TooLarge,
    #[error("downloaded document is empty")]
    Empty,
    #[error("WhatsApp document download failed")]
    Download,
}

#[async_trait]
pub(super) trait InboundDocumentDownloader: Send + Sync {
    async fn download_document(
        &self,
        document: &waproto::whatsapp::message::DocumentMessage,
    ) -> Result<Vec<u8>, DownloadError>;
}

#[async_trait]
impl InboundDocumentDownloader for Client {
    async fn download_document(
        &self,
        document: &waproto::whatsapp::message::DocumentMessage,
    ) -> Result<Vec<u8>, DownloadError> {
        let exceeded = Arc::new(AtomicBool::new(false));
        let writer = BoundedBuffer::new(MAX_INBOUND_DOCUMENT_BYTES, Arc::clone(&exceeded));
        let writer = self
            .download_to_writer(document, writer)
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
}

pub(super) struct InboundDocumentMetadata {
    pub display_name: String,
    pub media_type: String,
}

pub(super) fn metadata(name: Option<&str>, media_type: Option<&str>) -> InboundDocumentMetadata {
    InboundDocumentMetadata {
        display_name: safe_display_name(name),
        media_type: safe_media_type(media_type),
    }
}

pub(super) async fn save(
    sink: &Arc<dyn ChannelEventSink>,
    reply_to: &ChannelReplyTarget,
    data: &[u8],
    metadata: &InboundDocumentMetadata,
) -> Option<ChannelDocumentFile> {
    let filename = safe_storage_name(&metadata.media_type);
    let saved = sink
        .save_channel_attachment(data, &filename, reply_to)
        .await?;
    Some(document_file(&saved, metadata, data.len()))
}

fn document_file(
    saved: &SavedChannelFile,
    metadata: &InboundDocumentMetadata,
    size_bytes: usize,
) -> ChannelDocumentFile {
    ChannelDocumentFile {
        display_name: metadata.display_name.clone(),
        stored_filename: saved.filename.clone(),
        mime_type: metadata.media_type.clone(),
        size_bytes: u64::try_from(size_bytes).ok(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Mutex;

    use {
        async_trait::async_trait,
        moltis_channels::{ChannelEvent, ChannelType, Result},
    };

    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        saved: Mutex<Option<(Vec<u8>, String)>>,
    }

    #[async_trait]
    impl ChannelEventSink for RecordingSink {
        async fn emit(&self, _event: ChannelEvent) {}

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
        ) -> Result<String> {
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
            data: &[u8],
            filename: &str,
            _reply_to: &ChannelReplyTarget,
        ) -> Option<SavedChannelFile> {
            self.saved
                .lock()
                .unwrap()
                .replace((data.to_vec(), filename.to_string()));
            Some(SavedChannelFile {
                filename: filename.to_string(),
                media_ref: format!("media/test/{filename}"),
                absolute_path: format!("/tmp/test/{filename}"),
            })
        }
    }

    fn reply_target() -> ChannelReplyTarget {
        ChannelReplyTarget {
            channel_type: ChannelType::Whatsapp,
            account_id: "test".to_string(),
            chat_id: "test-chat".to_string(),
            message_id: None,
            thread_id: None,
            ack_message_id: None,
        }
    }

    #[test]
    fn reception_is_disabled_by_default_and_checks_reported_size() {
        assert_eq!(reception_for(false, Some(10)), DocumentReception::Disabled);
        assert_eq!(reception_for(true, Some(0)), DocumentReception::Empty);
        assert_eq!(
            reception_for(true, Some(MAX_INBOUND_DOCUMENT_BYTES as u64)),
            DocumentReception::Download
        );
        assert_eq!(
            reception_for(true, Some(MAX_INBOUND_DOCUMENT_BYTES as u64 + 1)),
            DocumentReception::TooLarge
        );
        assert_eq!(reception_for(true, None), DocumentReception::Download);
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
    fn metadata_removes_paths_controls_and_invalid_mime_values() {
        let sanitized = metadata(
            Some("../../folder\\report\nname.pdf"),
            Some("application/pdf\r\nmalicious: value"),
        );

        assert_eq!(sanitized.display_name, "reportname.pdf");
        assert_eq!(sanitized.media_type, "application/octet-stream");

        assert_eq!(
            metadata(Some("file.txt"), Some("text//plain")).media_type,
            "application/octet-stream"
        );
    }

    #[test]
    fn metadata_bounds_multibyte_display_names_on_a_character_boundary() {
        let long_name = "é".repeat(200);
        let metadata = metadata(Some(&long_name), Some("text/plain"));

        assert!(metadata.display_name.len() <= 255);
        assert!(
            metadata
                .display_name
                .chars()
                .all(|character| character == 'é')
        );
        assert_eq!(metadata.media_type, "text/plain");
    }

    #[tokio::test]
    async fn save_persists_exact_bytes_with_safe_metadata() {
        let sink = Arc::new(RecordingSink::default());
        let metadata = metadata(
            Some("../../Quarterly report.exe"),
            Some("Application/PDF; version=1.7"),
        );
        let document = save(
            &(Arc::clone(&sink) as Arc<dyn ChannelEventSink>),
            &reply_target(),
            b"%PDF-test",
            &metadata,
        )
        .await
        .unwrap();
        let saved = sink.saved.lock().unwrap().clone().unwrap();

        assert_eq!(saved.0, b"%PDF-test");
        assert_eq!(saved.1.len(), 45);
        assert!(saved.1.ends_with("_document.pdf"));
        assert!(
            saved.1[..32]
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        );
        assert_eq!(document.display_name, "Quarterly report.exe");
        assert_eq!(document.stored_filename, saved.1);
        assert_eq!(document.mime_type, "application/pdf");
        assert_eq!(document.size_bytes, Some(9));
        assert!(!document.stored_filename.contains('/'));
        assert!(!document.stored_filename.contains(".."));
        assert!(!document.stored_filename.contains("Quarterly"));
    }
}
