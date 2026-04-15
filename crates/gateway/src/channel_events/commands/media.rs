use std::sync::Arc;

use tracing::{debug, info, warn};

use moltis_channels::{
    ChannelReplyTarget, Error as ChannelError, Result as ChannelResult, SavedChannelFile,
};

use crate::state::GatewayState;

use super::super::{default_channel_session_key, resolve_channel_session};

pub(in crate::channel_events) async fn request_sender_approval(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    channel_type: &str,
    account_id: &str,
    identifier: &str,
) {
    if let Some(state) = state.get() {
        let params = serde_json::json!({
            "type": channel_type,
            "account_id": account_id,
            "identifier": identifier,
        });
        match state.services.channel.sender_approve(params).await {
            Ok(_) => {
                info!(account_id, identifier, "OTP self-approval: sender approved");
            },
            Err(e) => {
                warn!(
                    account_id,
                    identifier,
                    error = %e,
                    "OTP self-approval: failed to approve sender"
                );
            },
        }
    } else {
        warn!("request_sender_approval: gateway not ready");
    }
}

pub(in crate::channel_events) async fn save_channel_voice(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    audio_data: &[u8],
    filename: &str,
    reply_to: &ChannelReplyTarget,
) -> Option<String> {
    let state = state.get()?;
    let session_key = if let Some(ref sm) = state.services.session_metadata {
        resolve_channel_session(reply_to, sm).await
    } else {
        default_channel_session_key(reply_to)
    };
    let store = state.services.session_store.as_ref()?;
    match store.save_media(&session_key, filename, audio_data).await {
        Ok(_) => {
            debug!(
                session_key,
                filename, "saved channel voice audio to session media"
            );
            Some(filename.to_string())
        },
        Err(e) => {
            warn!(session_key, filename, error = %e, "failed to save channel voice audio");
            None
        },
    }
}

pub(in crate::channel_events) async fn save_channel_attachment(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    file_data: &[u8],
    filename: &str,
    reply_to: &ChannelReplyTarget,
) -> Option<SavedChannelFile> {
    let state = state.get()?;
    let session_key = if let Some(ref sm) = state.services.session_metadata {
        resolve_channel_session(reply_to, sm).await
    } else {
        default_channel_session_key(reply_to)
    };
    let store = state.services.session_store.as_ref()?;
    match store.save_media(&session_key, filename, file_data).await {
        Ok(media_ref) => {
            let absolute_path = store
                .media_path_for(&session_key, filename)
                .to_string_lossy()
                .to_string();
            debug!(
                session_key,
                filename, media_ref, absolute_path, "saved channel attachment to session media"
            );
            Some(SavedChannelFile {
                filename: filename.to_string(),
                media_ref,
                absolute_path,
            })
        },
        Err(e) => {
            warn!(
                session_key,
                filename,
                error = %e,
                "failed to save channel attachment"
            );
            None
        },
    }
}

pub(in crate::channel_events) async fn transcribe_voice(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    audio_data: &[u8],
    format: &str,
) -> ChannelResult<String> {
    let state = state
        .get()
        .ok_or_else(|| ChannelError::unavailable("gateway not ready"))?;

    let result = state
        .services
        .stt
        .transcribe_bytes(
            bytes::Bytes::copy_from_slice(audio_data),
            format,
            None,
            None,
            None,
        )
        .await
        .map_err(|e| ChannelError::unavailable(format!("transcription failed: {e}")))?;

    let text = result
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::invalid_input("transcription result missing text"))?;

    Ok(text.to_string())
}

pub(in crate::channel_events) async fn voice_stt_available(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
) -> bool {
    let Some(state) = state.get() else {
        return false;
    };

    match state.services.stt.status().await {
        Ok(status) => status
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        Err(_) => false,
    }
}
