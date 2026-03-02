use {async_trait::async_trait, secrecy::ExposeSecret, tracing::debug};

use {
    moltis_channels::{
        Error as ChannelError, Result as ChannelResult,
        plugin::{ChannelOutbound, ChannelStreamOutbound, StreamEvent, StreamReceiver},
    },
    moltis_common::types::ReplyPayload,
};

use crate::{auth::get_access_token, config::MsTeamsAccountConfig, state::AccountStateMap};

/// Outbound sender for Microsoft Teams channel accounts.
pub struct MsTeamsOutbound {
    pub(crate) accounts: AccountStateMap,
}

struct AccountSnapshot {
    config: MsTeamsAccountConfig,
    http: reqwest::Client,
    token_cache: std::sync::Arc<tokio::sync::Mutex<Option<crate::auth::CachedAccessToken>>>,
    service_url: String,
}

impl MsTeamsOutbound {
    fn account_snapshot(
        &self,
        account_id: &str,
        conversation_id: &str,
    ) -> ChannelResult<AccountSnapshot> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;
        let service_url = {
            let service_urls = state.service_urls.read().unwrap_or_else(|e| e.into_inner());
            service_urls
                .get(conversation_id)
                .cloned()
                .ok_or_else(|| {
                    ChannelError::unavailable(format!(
                        "missing Teams service URL for account '{account_id}' and conversation '{conversation_id}'"
                    ))
                })?
        };

        Ok(AccountSnapshot {
            config: state.config.clone(),
            http: state.http.clone(),
            token_cache: std::sync::Arc::clone(&state.token_cache),
            service_url,
        })
    }

    async fn send_activity(
        &self,
        account_id: &str,
        conversation_id: &str,
        activity: serde_json::Value,
    ) -> ChannelResult<()> {
        let snapshot = self.account_snapshot(account_id, conversation_id)?;
        let token = get_access_token(&snapshot.http, &snapshot.config, &snapshot.token_cache)
            .await
            .map_err(|e| ChannelError::unavailable(format!("Teams token acquisition: {e}")))?;

        let url = format!(
            "{}/v3/conversations/{}/activities",
            snapshot.service_url.trim_end_matches('/'),
            urlencoding::encode(conversation_id)
        );
        let resp = snapshot
            .http
            .post(url)
            .bearer_auth(token.expose_secret())
            .json(&activity)
            .send()
            .await
            .map_err(|e| ChannelError::external("Teams HTTP send", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::external(
                "Teams send failed",
                std::io::Error::other(format!("{status}: {body}")),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl ChannelOutbound for MsTeamsOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let mut payload = serde_json::json!({
            "type": "message",
            "text": text,
        });
        if let Some(reply_to) = reply_to
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert(
                "replyToId".into(),
                serde_json::Value::String(reply_to.to_string()),
            );
        }
        self.send_activity(account_id, to, payload).await
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let mut text = payload.text.clone();
        if let Some(media) = payload.media.as_ref() {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            if media.url.starts_with("data:") {
                text.push_str(
                    "[media omitted: Teams channel currently supports URL attachments only]",
                );
            } else {
                text.push_str(&media.url);
            }
        }
        self.send_text(account_id, to, &text, reply_to).await
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        self.send_activity(account_id, to, serde_json::json!({ "type": "typing" }))
            .await
    }

    async fn send_text_with_suffix(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let mut merged = text.to_string();
        if !suffix_html.is_empty() {
            merged.push_str("\n\n");
            merged.push_str(suffix_html);
        }
        self.send_text(account_id, to, &merged, reply_to).await
    }

    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.send_text(account_id, to, html, reply_to).await
    }

    async fn send_location(
        &self,
        account_id: &str,
        to: &str,
        latitude: f64,
        longitude: f64,
        title: Option<&str>,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let mut text = String::new();
        if let Some(title) = title {
            text.push_str(title);
            text.push('\n');
        }
        text.push_str(&format!(
            "https://www.google.com/maps?q={latitude:.6},{longitude:.6}"
        ));
        self.send_text(account_id, to, &text, reply_to).await
    }
}

#[async_trait]
impl ChannelStreamOutbound for MsTeamsOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        let mut text = String::new();
        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(delta) => text.push_str(&delta),
                StreamEvent::Done => break,
                StreamEvent::Error(err) => {
                    debug!(account_id, chat_id = to, "Teams stream error: {err}");
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
