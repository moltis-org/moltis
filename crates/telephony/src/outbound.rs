//! `ChannelOutbound` and `ChannelStreamOutbound` for telephony.
//!
//! Sends messages to active calls by playing TTS audio.

use {
    async_trait::async_trait,
    moltis_channels::{
        Result,
        plugin::{ChannelOutbound, ChannelStreamOutbound, StreamReceiver},
    },
    moltis_common::types::ReplyPayload,
    std::sync::Arc,
    tokio::sync::RwLock,
    tracing::debug,
};

use crate::manager::CallManager;

/// Outbound adapter that speaks messages into active phone calls.
///
/// `chat_id` (the `to` parameter) maps to the internal `call_id`.
pub struct TelephonyOutbound {
    manager: Arc<RwLock<CallManager>>,
}

impl TelephonyOutbound {
    pub fn new(manager: Arc<RwLock<CallManager>>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl ChannelOutbound for TelephonyOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        let mgr = self.manager.read().await;
        let call = mgr.get_call(to);
        let Some(call) = call else {
            debug!(call_id = %to, "no active call, skipping send_text");
            return Ok(());
        };

        if call.state.is_terminal() {
            debug!(call_id = %to, "call already ended, skipping TTS");
            return Ok(());
        }

        let provider_call_id = match call.provider_call_id.as_deref() {
            Some(pid) => pid.to_string(),
            None => return Ok(()),
        };

        if let Err(e) = mgr
            .provider()
            .read()
            .await
            .play_tts(&provider_call_id, text, None, None)
            .await
        {
            tracing::warn!(call_id = %to, error = %e, "TTS playback failed");
        }

        mgr.record_bot_speech(to, text);
        debug!(call_id = %to, account_id = %account_id, "TTS played");
        Ok(())
    }

    async fn send_media(
        &self,
        _account_id: &str,
        _to: &str,
        _payload: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        // Media attachments are not supported on phone calls.
        Ok(())
    }
}

/// Streaming outbound — phone calls don't support edit-in-place streaming.
pub struct TelephonyStreamOutbound;

#[async_trait]
impl ChannelStreamOutbound for TelephonyStreamOutbound {
    async fn send_stream(
        &self,
        _account_id: &str,
        _to: &str,
        _reply_to: Option<&str>,
        _stream: StreamReceiver,
    ) -> Result<()> {
        // Streaming not supported for telephony; caller falls back to send_text.
        Ok(())
    }

    async fn is_stream_enabled(&self, _account_id: &str) -> bool {
        false
    }
}

/// Routing outbound that dispatches to the correct account's CallManager.
///
/// Used as the shared_outbound() so the agent loop can route TTS replies
/// back to active phone calls across all accounts.
struct AccountOutbound {
    manager: Arc<RwLock<CallManager>>,
    gather_url: String,
}

pub(crate) struct RoutingOutbound {
    accounts: Arc<std::sync::RwLock<std::collections::HashMap<String, AccountOutbound>>>,
}

impl RoutingOutbound {
    pub(crate) fn new() -> Self {
        Self {
            accounts: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub(crate) fn set_manager(
        &self,
        account_id: &str,
        manager: Arc<RwLock<CallManager>>,
        gather_url: String,
    ) {
        if let Ok(mut map) = self.accounts.write() {
            map.insert(account_id.to_string(), AccountOutbound {
                manager,
                gather_url,
            });
        }
    }

    pub(crate) fn remove_manager(&self, account_id: &str) {
        if let Ok(mut map) = self.accounts.write() {
            map.remove(account_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn gather_url(&self, account_id: &str) -> Option<String> {
        self.accounts.read().ok().and_then(|map| {
            map.get(account_id)
                .map(|account| account.gather_url.clone())
        })
    }
}

#[async_trait]
impl ChannelOutbound for RoutingOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        let (mgr, gather_url) = {
            let map = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            match map.get(account_id) {
                Some(a) => (Arc::clone(&a.manager), a.gather_url.clone()),
                None => {
                    tracing::debug!(account_id = %account_id, "no call manager for account, skipping TTS");
                    return Ok(());
                },
            }
        };

        let manager = mgr.read().await;
        let call = manager.get_call(to);
        let Some(call) = call else {
            tracing::debug!(call_id = %to, "no active call, skipping TTS");
            return Ok(());
        };
        if call.state.is_terminal() {
            return Ok(());
        }

        let provider_call_id = match call.provider_call_id.as_deref() {
            Some(pid) => pid.to_string(),
            None => return Ok(()),
        };

        if let Err(e) = manager
            .provider()
            .read()
            .await
            .play_tts(&provider_call_id, text, None, Some(&gather_url))
            .await
        {
            tracing::warn!(call_id = %to, error = %e, "TTS playback failed");
        }
        manager.record_bot_speech(to, text);
        Ok(())
    }

    async fn send_media(
        &self,
        _account_id: &str,
        _to: &str,
        _payload: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }
}

/// Minimal HTML to plain text for TTS.
#[allow(dead_code)]
pub(crate) fn html2text(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {},
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html2text_strips_tags() {
        assert_eq!(html2text("<b>hello</b> world"), "hello world");
        assert_eq!(html2text("no tags here"), "no tags here");
        assert_eq!(html2text("<p>foo &amp; bar</p>"), "foo & bar");
    }
}
