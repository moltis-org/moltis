//! Outbound message sending for Signal channels.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use {
    async_trait::async_trait,
    moltis_channels::{
        ChannelOutbound, ChannelStreamOutbound, Result as ChannelResult, StreamReceiver,
        plugin::StreamEvent,
    },
    moltis_common::types::ReplyPayload,
    serde_json::{Value, json},
};

use crate::state::AccountState;

pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

#[derive(Clone)]
pub struct SignalOutbound {
    pub accounts: AccountStateMap,
}

#[derive(Debug, PartialEq, Eq)]
enum SignalTarget {
    Recipient(String),
    Group(String),
    Username(String),
}

impl SignalOutbound {
    fn resolve(
        &self,
        account_id: &str,
    ) -> ChannelResult<(
        crate::client::SignalClient,
        crate::config::SignalAccountConfig,
    )> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts.get(account_id).ok_or_else(|| {
            moltis_channels::Error::unavailable(format!("signal account not found: {account_id}"))
        })?;
        let config = state
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        Ok((state.client.clone(), config))
    }
}

fn parse_target(raw: &str) -> ChannelResult<SignalTarget> {
    let mut value = raw.trim();
    if value.is_empty() {
        return Err(moltis_channels::Error::invalid_input(
            "Signal recipient is required",
        ));
    }
    if let Some(stripped) = value.strip_prefix("signal:") {
        value = stripped.trim();
    }
    let lower = value.to_ascii_lowercase();
    if let Some(group_id) = lower
        .starts_with("group:")
        .then(|| value["group:".len()..].trim())
    {
        if group_id.is_empty() {
            return Err(moltis_channels::Error::invalid_input(
                "Signal group ID is required",
            ));
        }
        return Ok(SignalTarget::Group(group_id.to_string()));
    }
    if let Some(username) = lower
        .starts_with("username:")
        .then(|| value["username:".len()..].trim())
    {
        if username.is_empty() {
            return Err(moltis_channels::Error::invalid_input(
                "Signal username is required",
            ));
        }
        return Ok(SignalTarget::Username(username.to_string()));
    }
    if let Some(username) = lower.starts_with("u:").then(|| value["u:".len()..].trim()) {
        if username.is_empty() {
            return Err(moltis_channels::Error::invalid_input(
                "Signal username is required",
            ));
        }
        return Ok(SignalTarget::Username(username.to_string()));
    }
    Ok(SignalTarget::Recipient(value.to_string()))
}

fn target_params(target: SignalTarget) -> Value {
    match target {
        SignalTarget::Recipient(recipient) => json!({ "recipient": [recipient] }),
        SignalTarget::Group(group_id) => json!({ "groupId": group_id }),
        SignalTarget::Username(username) => json!({ "username": [username] }),
    }
}

fn chunk_text(text: &str, limit: usize) -> Vec<&str> {
    if text.is_empty() || text.len() <= limit {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + limit).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(&text[start..end]);
        start = end;
    }
    chunks
}

async fn send_text_once(
    client: &crate::client::SignalClient,
    config: &crate::config::SignalAccountConfig,
    to: &str,
    text: &str,
) -> ChannelResult<()> {
    let target = parse_target(to)?;
    let mut params = target_params(target);
    let Some(obj) = params.as_object_mut() else {
        return Err(moltis_channels::Error::invalid_input(
            "invalid Signal target parameters",
        ));
    };
    obj.insert("message".to_string(), json!(text));
    if let Some(account) = config.account() {
        obj.insert("account".to_string(), json!(account));
    }

    let _ = client
        .rpc_value(&config.http_url, "send", Value::Object(obj.clone()))
        .await?;
    Ok(())
}

#[async_trait]
impl ChannelOutbound for SignalOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let (client, config) = self.resolve(account_id)?;
        for chunk in chunk_text(text, config.text_chunk_limit) {
            send_text_once(&client, &config, to, chunk).await?;
        }
        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.send_text(account_id, to, &payload.text, reply_to)
            .await
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        let (client, config) = self.resolve(account_id)?;
        let target = parse_target(to)?;
        let mut params = target_params(target);
        let Some(obj) = params.as_object_mut() else {
            return Ok(());
        };
        if let Some(account) = config.account() {
            obj.insert("account".to_string(), json!(account));
        }
        let _ = client
            .rpc_value(&config.http_url, "sendTyping", Value::Object(obj.clone()))
            .await;
        Ok(())
    }
}

#[async_trait]
impl ChannelStreamOutbound for SignalOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        let mut buffer = String::new();
        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(chunk) => buffer.push_str(&chunk),
                StreamEvent::Done => break,
                StreamEvent::Error(e) => {
                    tracing::warn!(account_id, "Signal stream error: {e}");
                    if buffer.is_empty() {
                        buffer.push_str("[Error generating response]");
                    }
                    break;
                },
            }
        }

        if !buffer.is_empty() {
            self.send_text(account_id, to, &buffer, reply_to).await?;
        }
        Ok(())
    }

    async fn is_stream_enabled(&self, _account_id: &str) -> bool {
        false
    }
}

impl std::fmt::Debug for SignalOutbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalOutbound").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use crate::outbound::{SignalTarget, chunk_text, parse_target};

    #[test]
    fn parses_signal_targets() {
        assert_eq!(
            parse_target("signal:+15551234567").ok(),
            Some(SignalTarget::Recipient("+15551234567".to_string()))
        );
        assert_eq!(
            parse_target("group:abc").ok(),
            Some(SignalTarget::Group("abc".to_string()))
        );
        assert_eq!(
            parse_target("u:alice.01").ok(),
            Some(SignalTarget::Username("alice.01".to_string()))
        );
    }

    #[test]
    fn chunks_at_utf8_boundaries() {
        let chunks = chunk_text("aébc", 2);
        assert_eq!(chunks, vec!["a", "é", "bc"]);
    }
}
