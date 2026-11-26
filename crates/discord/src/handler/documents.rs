//! Inbound Discord documents: attachments that are neither audio nor a still
//! image are downloaded, saved to the session media directory and handed to
//! the agent as `ChannelDocumentFile`s.

use {
    moltis_channels::{
        ChannelDocumentFile, ChannelEventSink, ChannelReplyTarget, InboundMediaDownloader,
        InboundMediaSource,
    },
    serenity::all::Attachment,
    tracing::{debug, warn},
};

use super::attachments::{MAX_ATTACHMENT_BYTES, attachment_is_document};

const OCTET_STREAM: &str = "application/octet-stream";
const MAX_STORED_NAME_CHARS: usize = 160;
const MAX_EXTENSION_CHARS: usize = 16;
const MAX_DISPLAY_NAME_CHARS: usize = 120;

/// Why a document attachment could not be handed to the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DocumentFailure {
    TooLarge,
    DownloadFailed,
    SaveFailed,
}

impl DocumentFailure {
    fn reason(self) -> &'static str {
        match self {
            Self::TooLarge => "the file exceeds the 25 MB limit",
            Self::DownloadFailed => "the download failed",
            Self::SaveFailed => "the file could not be saved",
        }
    }
}

#[derive(Debug)]
struct DocumentOutcome {
    display_name: String,
    mime_type: String,
    result: Result<ChannelDocumentFile, DocumentFailure>,
}

/// Result of ingesting every document attachment of one Discord message.
#[derive(Debug, Default)]
pub(super) struct DocumentIngestion {
    outcomes: Vec<DocumentOutcome>,
}

impl DocumentIngestion {
    #[must_use]
    pub(super) fn has_saved(&self) -> bool {
        self.outcomes.iter().any(|outcome| outcome.result.is_ok())
    }

    #[must_use]
    pub(super) fn has_failures(&self) -> bool {
        self.outcomes.iter().any(|outcome| outcome.result.is_err())
    }

    /// Whether the message should stop after the user-facing notices: a voice
    /// note that could not be transcribed, or documents that all failed with
    /// no text or media beside them. Saved documents always go to the agent.
    #[must_use]
    pub(super) fn nothing_to_dispatch(
        &self,
        body: &str,
        has_media_attachments: bool,
        voice_untranscribed: bool,
    ) -> bool {
        if self.has_saved() {
            return false;
        }
        voice_untranscribed || (body.is_empty() && !has_media_attachments && self.has_failures())
    }

    /// Saved documents for `ChannelMessageMeta::documents`.
    #[must_use]
    pub(super) fn documents(&self) -> Option<Vec<ChannelDocumentFile>> {
        let documents: Vec<_> = self
            .outcomes
            .iter()
            .filter_map(|outcome| outcome.result.as_ref().ok().cloned())
            .collect();
        (!documents.is_empty()).then_some(documents)
    }

    /// Append one label per document to the message body so the agent knows
    /// what was attached, including the files that could not be ingested.
    #[must_use]
    pub(super) fn append_labels(&self, body: &str) -> String {
        let labels = self.outcomes.iter().map(|outcome| {
            let base = format!("{} ({})", outcome.display_name, outcome.mime_type);
            match outcome.result {
                Ok(_) => format!("[Document: {base}]"),
                Err(failure) => format!("[Document: {base} - not received: {}]", failure.reason()),
            }
        });
        let sections: Vec<String> = std::iter::once(body.to_string())
            .filter(|body| !body.is_empty())
            .chain(labels)
            .collect();
        sections.join("\n\n")
    }

    /// Message telling the sender which files the agent will not see.
    #[must_use]
    pub(super) fn user_notice(&self) -> Option<String> {
        let failures: Vec<String> = self
            .outcomes
            .iter()
            .filter_map(|outcome| {
                outcome
                    .result
                    .as_ref()
                    .err()
                    .map(|failure| format!("`{}`: {}", outcome.display_name, failure.reason()))
            })
            .collect();
        match failures.as_slice() {
            [] => None,
            [single] => Some(format!("I couldn't read {single}.")),
            many => Some(format!(
                "I couldn't read some of your files:\n- {}",
                many.join("\n- ")
            )),
        }
    }
}

/// Download and save every document attachment, concurrently.
pub(super) async fn ingest_discord_documents(
    attachments: &[Attachment],
    sink: &dyn ChannelEventSink,
    reply_to: &ChannelReplyTarget,
    account_id: &str,
    downloader: &dyn InboundMediaDownloader,
) -> DocumentIngestion {
    let pending = attachments
        .iter()
        .filter(|attachment| attachment_is_document(attachment))
        .map(|attachment| ingest_document(attachment, sink, reply_to, account_id, downloader));
    DocumentIngestion {
        outcomes: futures::future::join_all(pending).await,
    }
}

async fn ingest_document(
    attachment: &Attachment,
    sink: &dyn ChannelEventSink,
    reply_to: &ChannelReplyTarget,
    account_id: &str,
    downloader: &dyn InboundMediaDownloader,
) -> DocumentOutcome {
    let display_name = display_name(&attachment.filename);
    let mime_type = document_media_type(attachment.content_type.as_deref(), &attachment.filename);
    let result = download_and_save(
        attachment, &mime_type, sink, reply_to, account_id, downloader,
    )
    .await
    .map(|(stored_filename, size)| ChannelDocumentFile {
        display_name: display_name.clone(),
        stored_filename,
        mime_type: mime_type.clone(),
        size_bytes: u64::try_from(size).ok(),
    });
    DocumentOutcome {
        display_name,
        mime_type,
        result,
    }
}

/// Returns the stored filename and the byte size of the saved document.
async fn download_and_save(
    attachment: &Attachment,
    mime_type: &str,
    sink: &dyn ChannelEventSink,
    reply_to: &ChannelReplyTarget,
    account_id: &str,
    downloader: &dyn InboundMediaDownloader,
) -> Result<(String, usize), DocumentFailure> {
    // Discord reports the size up front, so oversized (Nitro) uploads are
    // rejected without spending bandwidth on them.
    if usize::try_from(attachment.size).map_or(true, |size| size > MAX_ATTACHMENT_BYTES) {
        warn!(
            account_id,
            attachment_id = %attachment.id,
            size = attachment.size,
            limit = MAX_ATTACHMENT_BYTES,
            "skipping oversized discord document"
        );
        return Err(DocumentFailure::TooLarge);
    }

    let source = InboundMediaSource::RemoteUrl {
        url: attachment.url.clone(),
    };
    let data = downloader
        .download_media(&source, MAX_ATTACHMENT_BYTES)
        .await
        .map_err(|error| {
            warn!(
                account_id,
                attachment_id = %attachment.id,
                %error,
                "failed to download discord document"
            );
            DocumentFailure::DownloadFailed
        })?;

    let filename = stored_filename(&attachment.id.to_string(), &attachment.filename, mime_type);
    let Some(saved) = sink
        .save_channel_attachment(&data, &filename, reply_to)
        .await
    else {
        warn!(
            account_id,
            attachment_id = %attachment.id,
            filename,
            "failed to save discord document"
        );
        return Err(DocumentFailure::SaveFailed);
    };
    debug!(
        account_id,
        attachment_id = %attachment.id,
        stored_filename = %saved.filename,
        mime_type,
        size = data.len(),
        "saved discord document"
    );
    Ok((saved.filename, data.len()))
}

/// Normalize the reported MIME type to a bare `type/subtype`, falling back to
/// the filename extension when Discord sends none or a generic binary type.
///
/// The value reaches the LLM context, so anything malformed is replaced
/// rather than passed through.
pub(super) fn document_media_type(content_type: Option<&str>, filename: &str) -> String {
    let reported = content_type
        .and_then(|raw| raw.split(';').next())
        .map(|raw| raw.trim().to_ascii_lowercase())
        .filter(|mime| is_well_formed_mime(mime));
    if let Some(mime) = reported.filter(|mime| mime != OCTET_STREAM) {
        return mime;
    }
    filename
        .rsplit_once('.')
        .and_then(|(_, extension)| moltis_media::mime::mime_from_extension(extension))
        .unwrap_or(OCTET_STREAM)
        .to_string()
}

fn is_well_formed_mime(mime: &str) -> bool {
    mime.len() <= 127
        && mime.split_once('/').is_some_and(|(kind, subtype)| {
            !kind.is_empty() && !subtype.is_empty() && !subtype.contains('/')
        })
        && mime
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "/+-.".contains(character))
}

/// Sender-controlled filename made safe for a body label and a Discord reply:
/// no control characters, no brackets or backticks, bounded length.
pub(super) fn display_name(filename: &str) -> String {
    let cleaned: String = filename
        .chars()
        .filter(|character| !character.is_control() && !"[]`".contains(*character))
        .take(MAX_DISPLAY_NAME_CHARS)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Session media filename: the attachment id keeps it unique, the rest is the
/// sanitized original name so the agent still sees a meaningful extension.
pub(super) fn stored_filename(attachment_id: &str, filename: &str, mime_type: &str) -> String {
    let sanitized: String = filename
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "-_.".contains(*character))
        .collect();
    let sanitized = sanitized.trim_start_matches('.');
    let (stem, extension) = match sanitized.rsplit_once('.') {
        Some((stem, extension)) if !extension.is_empty() => (stem, extension),
        _ => (sanitized, moltis_media::mime::extension_for_mime(mime_type)),
    };
    let stem: String = stem.chars().take(MAX_STORED_NAME_CHARS).collect();
    let stem = if stem.is_empty() {
        "discord-file"
    } else {
        &stem
    };
    let extension: String = extension.chars().take(MAX_EXTENSION_CHARS).collect();
    format!("{attachment_id}_{stem}.{extension}")
}
