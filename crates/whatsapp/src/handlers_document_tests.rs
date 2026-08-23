use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
};

use {
    async_trait::async_trait,
    moltis_channels::{
        ChannelDocumentFile, ChannelEvent, ChannelEventSink, Result as ChannelResult,
        SavedChannelFile,
    },
};

use super::*;

#[derive(Clone)]
enum StubDownload {
    Data(Vec<u8>),
    Error(DownloadError),
}

struct StubDocumentDownloader {
    result: StubDownload,
    calls: AtomicUsize,
}

impl StubDocumentDownloader {
    fn data(data: &[u8]) -> Self {
        Self {
            result: StubDownload::Data(data.to_vec()),
            calls: AtomicUsize::new(0),
        }
    }

    fn error(error: DownloadError) -> Self {
        Self {
            result: StubDownload::Error(error),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(AtomicOrdering::Acquire)
    }
}

#[async_trait]
impl InboundDocumentDownloader for StubDocumentDownloader {
    async fn download_document(
        &self,
        _document: &wa::message::DocumentMessage,
    ) -> Result<Vec<u8>, DownloadError> {
        self.calls.fetch_add(1, AtomicOrdering::AcqRel);
        match &self.result {
            StubDownload::Data(data) => Ok(data.clone()),
            StubDownload::Error(error) => Err(*error),
        }
    }
}

#[derive(Default)]
struct RecordingDocumentSink {
    dispatched: Mutex<Vec<(String, Option<Vec<ChannelDocumentFile>>)>>,
    saved: Mutex<Vec<(Vec<u8>, String)>>,
    fail_save: bool,
}

#[async_trait]
impl ChannelEventSink for RecordingDocumentSink {
    async fn emit(&self, _event: ChannelEvent) {}

    async fn dispatch_to_chat(
        &self,
        text: &str,
        _reply_to: ChannelReplyTarget,
        meta: ChannelMessageMeta,
    ) {
        self.dispatched
            .lock()
            .unwrap()
            .push((text.to_string(), meta.documents));
    }

    async fn dispatch_command(
        &self,
        _command: &str,
        _reply_to: ChannelReplyTarget,
        _sender_id: Option<&str>,
    ) -> ChannelResult<String> {
        Ok(String::new())
    }

    async fn request_disable_account(&self, _channel_type: &str, _account_id: &str, _reason: &str) {
    }

    async fn save_channel_attachment(
        &self,
        data: &[u8],
        filename: &str,
        _reply_to: &ChannelReplyTarget,
    ) -> Option<SavedChannelFile> {
        if self.fail_save {
            return None;
        }
        self.saved
            .lock()
            .unwrap()
            .push((data.to_vec(), filename.to_string()));
        Some(SavedChannelFile {
            filename: filename.to_string(),
            media_ref: format!("media/test/{filename}"),
            absolute_path: format!("/tmp/test/{filename}"),
        })
    }
}

fn document_reply_target() -> ChannelReplyTarget {
    ChannelReplyTarget {
        channel_type: ChannelType::Whatsapp,
        account_id: "test-account".to_string(),
        chat_id: "test-chat".to_string(),
        message_id: Some("test-message".to_string()),
        thread_id: None,
        ack_message_id: None,
    }
}

fn document_meta() -> ChannelMessageMeta {
    ChannelMessageMeta {
        channel_type: ChannelType::Whatsapp,
        sender_name: Some("Test Sender".to_string()),
        username: Some("test-sender".to_string()),
        sender_id: Some("test-sender-id".to_string()),
        message_kind: Some(ChannelMessageKind::Document),
        model: None,
        agent_id: None,
        audio_filename: None,
        documents: None,
    }
}

fn synthetic_document(reported_size: Option<u64>) -> wa::message::DocumentMessage {
    wa::message::DocumentMessage {
        caption: Some("Please inspect this synthetic document".to_string()),
        file_name: Some("synthetic-note.txt".to_string()),
        mimetype: Some("text/plain".to_string()),
        file_length: reported_size,
        ..Default::default()
    }
}

async fn run_document_flow(
    document: &wa::message::DocumentMessage,
    downloader: &StubDocumentDownloader,
    download_enabled: bool,
    sink: Arc<RecordingDocumentSink>,
) {
    let sink: Arc<dyn ChannelEventSink> = sink;
    dispatch_document(
        document,
        downloader,
        download_enabled,
        "test-account",
        document_reply_target(),
        document_meta(),
        &sink,
    )
    .await;
}

#[tokio::test]
async fn document_download_opt_out_skips_network_and_storage() {
    let downloader = StubDocumentDownloader::data(b"synthetic content");
    let sink = Arc::new(RecordingDocumentSink::default());

    run_document_flow(
        &synthetic_document(Some(17)),
        &downloader,
        false,
        Arc::clone(&sink),
    )
    .await;

    assert_eq!(downloader.calls(), 0);
    assert!(sink.saved.lock().unwrap().is_empty());
    let dispatched = sink.dispatched.lock().unwrap();
    assert_eq!(dispatched.len(), 1);
    assert!(dispatched[0].0.contains("downloads are disabled"));
    assert!(dispatched[0].1.is_none());
}

#[tokio::test]
async fn document_download_success_persists_bytes_and_dispatches_metadata() {
    let downloader = StubDocumentDownloader::data(b"synthetic content");
    let sink = Arc::new(RecordingDocumentSink::default());

    run_document_flow(
        &synthetic_document(Some(17)),
        &downloader,
        true,
        Arc::clone(&sink),
    )
    .await;

    assert_eq!(downloader.calls(), 1);
    let saved = sink.saved.lock().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].0, b"synthetic content");
    assert!(saved[0].1.ends_with("_document.txt"));
    assert!(!saved[0].1.contains("synthetic-note"));

    let dispatched = sink.dispatched.lock().unwrap();
    assert_eq!(dispatched.len(), 1);
    assert!(!dispatched[0].0.contains("failed"));
    let documents = dispatched[0].1.as_ref().unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].display_name, "synthetic-note.txt");
    assert_eq!(documents[0].stored_filename, saved[0].1);
    assert_eq!(documents[0].mime_type, "text/plain");
    assert_eq!(documents[0].size_bytes, Some(17));
}

#[tokio::test]
async fn oversized_reported_document_is_rejected_before_download() {
    let downloader = StubDocumentDownloader::data(b"not downloaded");
    let sink = Arc::new(RecordingDocumentSink::default());

    run_document_flow(
        &synthetic_document(Some(
            inbound_documents::MAX_INBOUND_DOCUMENT_BYTES as u64 + 1,
        )),
        &downloader,
        true,
        Arc::clone(&sink),
    )
    .await;

    assert_eq!(downloader.calls(), 0);
    assert!(sink.saved.lock().unwrap().is_empty());
    let dispatched = sink.dispatched.lock().unwrap();
    assert!(dispatched[0].0.contains("exceeds the 20 MB limit"));
    assert!(dispatched[0].1.is_none());
}

#[tokio::test]
async fn oversized_stream_is_rejected_when_reported_size_is_missing() {
    let downloader = StubDocumentDownloader::error(DownloadError::TooLarge);
    let sink = Arc::new(RecordingDocumentSink::default());

    run_document_flow(
        &synthetic_document(None),
        &downloader,
        true,
        Arc::clone(&sink),
    )
    .await;

    assert_eq!(downloader.calls(), 1);
    assert!(sink.saved.lock().unwrap().is_empty());
    let dispatched = sink.dispatched.lock().unwrap();
    assert!(dispatched[0].0.contains("exceeds the 20 MB limit"));
    assert!(dispatched[0].1.is_none());
}

#[tokio::test]
async fn persistence_failure_never_exposes_nonexistent_document_metadata() {
    let downloader = StubDocumentDownloader::data(b"synthetic content");
    let sink = Arc::new(RecordingDocumentSink {
        fail_save: true,
        ..Default::default()
    });

    run_document_flow(
        &synthetic_document(Some(17)),
        &downloader,
        true,
        Arc::clone(&sink),
    )
    .await;

    assert_eq!(downloader.calls(), 1);
    assert!(sink.saved.lock().unwrap().is_empty());
    let dispatched = sink.dispatched.lock().unwrap();
    assert!(dispatched[0].0.contains("could not be saved"));
    assert!(dispatched[0].1.is_none());
}
