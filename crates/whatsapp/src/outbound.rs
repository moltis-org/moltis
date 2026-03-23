use {
    async_trait::async_trait,
    base64::Engine,
    tracing::{debug, info},
};

use {
    wacore::download::MediaType,
    wacore_binary::jid::Jid,
    waproto::whatsapp as wa,
    whatsapp_rust::{ChatStateType, upload::UploadResponse},
};

use {
    moltis_channels::{
        Result as ChannelResult,
        plugin::{ChannelOutbound, ChannelStreamOutbound, StreamEvent, StreamReceiver},
    },
    moltis_common::types::ReplyPayload,
};

use crate::state::{AccountStateMap, BOT_WATERMARK};

// ── Media helpers ────────────────────────────────────────────────────

/// Decode a `data:<mime>;base64,<payload>` URI into raw bytes.
fn decode_data_url(url: &str) -> ChannelResult<Vec<u8>> {
    let comma_pos = url
        .find(',')
        .ok_or_else(|| moltis_channels::Error::invalid_input("invalid data URI: no comma"))?;
    let base64_data = &url[comma_pos + 1..];
    base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| moltis_channels::Error::invalid_input(format!("base64 decode: {e}")))
}

/// Map a MIME type to the WhatsApp `MediaType` used for encryption/upload.
fn mime_to_media_type(mime: &str) -> MediaType {
    if mime.starts_with("image/") {
        MediaType::Image
    } else if mime.starts_with("video/") {
        MediaType::Video
    } else if mime.starts_with("audio/") {
        MediaType::Audio
    } else {
        MediaType::Document
    }
}

/// Build the `wa::Message` for a successfully uploaded media file.
fn build_media_message(
    mime: &str,
    caption: Option<String>,
    upload: &UploadResponse,
) -> wa::Message {
    if mime.starts_with("image/") {
        wa::Message {
            image_message: Some(Box::new(wa::message::ImageMessage {
                mimetype: Some(mime.to_string()),
                caption,
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key.clone()),
                file_sha256: Some(upload.file_sha256.clone()),
                file_enc_sha256: Some(upload.file_enc_sha256.clone()),
                file_length: Some(upload.file_length),
                ..Default::default()
            })),
            ..Default::default()
        }
    } else if mime.starts_with("video/") {
        wa::Message {
            video_message: Some(Box::new(wa::message::VideoMessage {
                mimetype: Some(mime.to_string()),
                caption,
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key.clone()),
                file_sha256: Some(upload.file_sha256.clone()),
                file_enc_sha256: Some(upload.file_enc_sha256.clone()),
                file_length: Some(upload.file_length),
                ..Default::default()
            })),
            ..Default::default()
        }
    } else if mime.starts_with("audio/") {
        wa::Message {
            audio_message: Some(Box::new(wa::message::AudioMessage {
                mimetype: Some(mime.to_string()),
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key.clone()),
                file_sha256: Some(upload.file_sha256.clone()),
                file_enc_sha256: Some(upload.file_enc_sha256.clone()),
                file_length: Some(upload.file_length),
                ..Default::default()
            })),
            ..Default::default()
        }
    } else {
        wa::Message {
            document_message: Some(Box::new(wa::message::DocumentMessage {
                mimetype: Some(mime.to_string()),
                title: caption,
                url: Some(upload.url.clone()),
                direct_path: Some(upload.direct_path.clone()),
                media_key: Some(upload.media_key.clone()),
                file_sha256: Some(upload.file_sha256.clone()),
                file_enc_sha256: Some(upload.file_enc_sha256.clone()),
                file_length: Some(upload.file_length),
                ..Default::default()
            })),
            ..Default::default()
        }
    }
}

/// Outbound message sender for WhatsApp.
pub struct WhatsAppOutbound {
    pub(crate) accounts: AccountStateMap,
}

impl WhatsAppOutbound {
    fn get_client(
        &self,
        account_id: &str,
    ) -> ChannelResult<std::sync::Arc<whatsapp_rust::client::Client>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| std::sync::Arc::clone(&s.client))
            .ok_or_else(|| moltis_channels::Error::unknown_account(account_id))
    }

    /// Record a sent message ID for self-chat loop detection.
    fn record_sent_id(&self, account_id: &str, msg_id: &str) {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get(account_id) {
            state.record_sent_id(msg_id);
        }
    }
}

#[async_trait]
impl ChannelOutbound for WhatsAppOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let client = self.get_client(account_id)?;
        let jid: Jid = to
            .parse()
            .map_err(|e| moltis_channels::Error::invalid_input(format!("invalid JID: {e:?}")))?;

        debug!(
            account_id,
            to,
            text_len = text.len(),
            "sending WhatsApp text"
        );

        let mut watermarked = text.to_string();
        watermarked.push_str(BOT_WATERMARK);
        let msg = wa::Message {
            conversation: Some(watermarked),
            ..Default::default()
        };
        let msg_id = client
            .send_message(jid, msg)
            .await
            .map_err(|e| moltis_channels::Error::unavailable(format!("whatsapp send_text: {e}")))?;
        self.record_sent_id(account_id, &msg_id);

        #[cfg(feature = "metrics")]
        moltis_metrics::counter!(
            moltis_metrics::channels::MESSAGES_SENT_TOTAL,
            moltis_metrics::labels::CHANNEL => "whatsapp"
        )
        .increment(1);

        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let Some(media) = payload.media.as_ref() else {
            return self
                .send_text(account_id, to, &payload.text, reply_to)
                .await;
        };

        // Non-data: URLs — send as text (WhatsApp auto-previews links).
        if !media.url.starts_with("data:") {
            let mut text = payload.text.clone();
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&media.url);
            return self.send_text(account_id, to, &text, reply_to).await;
        }

        // Decode base64 data: URI.
        let bytes = decode_data_url(&media.url)?;
        let media_type = mime_to_media_type(&media.mime_type);
        let caption = if payload.text.is_empty() {
            None
        } else {
            Some(payload.text.clone())
        };

        info!(
            account_id,
            to,
            mime = %media.mime_type,
            bytes = bytes.len(),
            media_type = ?media_type,
            "uploading WhatsApp media"
        );

        let client = self.get_client(account_id)?;
        let jid: Jid = to
            .parse()
            .map_err(|e| moltis_channels::Error::invalid_input(format!("invalid JID: {e:?}")))?;

        let upload = client.upload(bytes, media_type).await.map_err(|e| {
            moltis_channels::Error::unavailable(format!("whatsapp media upload: {e}"))
        })?;

        let msg = build_media_message(&media.mime_type, caption, &upload);

        let msg_id = client.send_message(jid, msg).await.map_err(|e| {
            moltis_channels::Error::unavailable(format!("whatsapp send_media: {e}"))
        })?;
        self.record_sent_id(account_id, &msg_id);

        #[cfg(feature = "metrics")]
        moltis_metrics::counter!(
            moltis_metrics::channels::MESSAGES_SENT_TOTAL,
            moltis_metrics::labels::CHANNEL => "whatsapp"
        )
        .increment(1);

        info!(account_id, to, "WhatsApp media sent");
        Ok(())
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        let client = self.get_client(account_id)?;
        let jid: Jid = to
            .parse()
            .map_err(|e| moltis_channels::Error::invalid_input(format!("invalid JID: {e:?}")))?;
        client
            .chatstate()
            .send(&jid, ChatStateType::Composing)
            .await
            .map_err(|e| moltis_channels::Error::unavailable(format!("whatsapp chatstate: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl ChannelStreamOutbound for WhatsAppOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        // WhatsApp doesn't support message editing, so collect all deltas
        // and send the final text as a single message.
        let mut text = String::new();
        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(delta) => text.push_str(&delta),
                StreamEvent::Done => break,
                StreamEvent::Error(err) => {
                    debug!(account_id, chat_id = to, "WhatsApp stream error: {err}");
                    if text.is_empty() {
                        text = err;
                    }
                    break;
                },
            }
        }
        if text.is_empty() {
            return Ok(());
        }
        self.send_text(account_id, to, &text, reply_to).await
    }

    async fn is_stream_enabled(&self, _account_id: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_data_url_valid() {
        let b64 = base64::engine::general_purpose::STANDARD.encode([0xAB, 0xCD]);
        let url = format!("data:image/png;base64,{b64}");
        let bytes = decode_data_url(&url).unwrap_or_else(|e| panic!("decode failed: {e}"));
        assert_eq!(bytes, vec![0xAB, 0xCD]);
    }

    #[test]
    fn decode_data_url_no_comma_fails() {
        assert!(decode_data_url("data:image/png;base64").is_err());
    }

    #[test]
    fn mime_to_media_type_mapping() {
        assert!(matches!(mime_to_media_type("image/png"), MediaType::Image));
        assert!(matches!(mime_to_media_type("image/jpeg"), MediaType::Image));
        assert!(matches!(mime_to_media_type("video/mp4"), MediaType::Video));
        assert!(matches!(mime_to_media_type("audio/ogg"), MediaType::Audio));
        assert!(matches!(
            mime_to_media_type("application/pdf"),
            MediaType::Document
        ));
        assert!(matches!(
            mime_to_media_type("application/octet-stream"),
            MediaType::Document
        ));
    }

    #[test]
    fn build_media_message_image() {
        let upload = UploadResponse {
            url: "https://example.com/img".into(),
            direct_path: "/path".into(),
            media_key: vec![1, 2, 3],
            file_sha256: vec![4, 5, 6],
            file_enc_sha256: vec![7, 8, 9],
            file_length: 1024,
        };
        let msg = build_media_message("image/png", Some("caption".into()), &upload);
        let img = msg
            .image_message
            .unwrap_or_else(|| panic!("expected image_message"));
        assert_eq!(img.mimetype.as_deref(), Some("image/png"));
        assert_eq!(img.caption.as_deref(), Some("caption"));
        assert_eq!(img.url.as_deref(), Some("https://example.com/img"));
        assert_eq!(img.file_length, Some(1024));
    }

    #[test]
    fn build_media_message_video() {
        let upload = UploadResponse {
            url: "https://example.com/vid".into(),
            direct_path: "/path".into(),
            media_key: vec![],
            file_sha256: vec![],
            file_enc_sha256: vec![],
            file_length: 2048,
        };
        let msg = build_media_message("video/mp4", None, &upload);
        let vid = msg
            .video_message
            .unwrap_or_else(|| panic!("expected video_message"));
        assert_eq!(vid.mimetype.as_deref(), Some("video/mp4"));
        assert!(vid.caption.is_none());
    }

    #[test]
    fn build_media_message_audio() {
        let upload = UploadResponse {
            url: "https://example.com/aud".into(),
            direct_path: "/path".into(),
            media_key: vec![],
            file_sha256: vec![],
            file_enc_sha256: vec![],
            file_length: 512,
        };
        let msg = build_media_message("audio/ogg", None, &upload);
        assert!(msg.audio_message.is_some());
    }

    #[test]
    fn build_media_message_document_fallback() {
        let upload = UploadResponse {
            url: "https://example.com/doc".into(),
            direct_path: "/path".into(),
            media_key: vec![],
            file_sha256: vec![],
            file_enc_sha256: vec![],
            file_length: 4096,
        };
        let msg = build_media_message("application/pdf", Some("report.pdf".into()), &upload);
        let doc = msg
            .document_message
            .unwrap_or_else(|| panic!("expected document_message"));
        assert_eq!(doc.mimetype.as_deref(), Some("application/pdf"));
        assert_eq!(doc.title.as_deref(), Some("report.pdf"));
    }
}
