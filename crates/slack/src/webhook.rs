//! Slack Events API webhook handler.
//!
//! Receives HTTP POST requests from Slack's Events API, verifies request
//! signatures using HMAC-SHA256, and dispatches events to the same handlers
//! used by Socket Mode.

use std::sync::Arc;

use {
    hmac::{Hmac, Mac},
    secrecy::ExposeSecret,
    sha2::Sha256,
    slack_morphism::prelude::*,
    tracing::{debug, info},
};

use moltis_channels::{
    ChannelEventSink,
    message_log::MessageLog,
    plugin::{ChannelReplyTarget, ChannelType},
};

use crate::{
    config::SlackAccountConfig,
    state::{AccountState, AccountStateMap},
};

type HmacSha256 = Hmac<Sha256>;

/// Register an Events API account.
///
/// Unlike Socket Mode, this only authenticates the bot token and registers
/// account state. The actual event receiving is done by the HTTP webhook
/// handler in the gateway server.
pub async fn register_events_api_account(
    account_id: &str,
    config: SlackAccountConfig,
    accounts: AccountStateMap,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
) -> moltis_channels::Result<()> {
    let bot_token_str = config.bot_token.expose_secret().clone();

    if bot_token_str.is_empty() {
        return Err(moltis_channels::Error::invalid_input(
            "Slack bot_token is required",
        ));
    }

    let client = Arc::new(SlackClient::new(SlackClientHyperConnector::new().map_err(
        |e| moltis_channels::Error::unavailable(format!("hyper connector: {e}")),
    )?));

    // Verify the bot token and get the bot user ID.
    let bot_token = SlackApiToken::new(SlackApiTokenValue::from(bot_token_str));
    let session = client.open_session(&bot_token);
    let auth_response = session
        .auth_test()
        .await
        .map_err(|e| moltis_channels::Error::unavailable(format!("auth.test failed: {e}")))?;

    let bot_user_id = auth_response.user_id.to_string();
    info!(
        account_id,
        bot_user_id, "slack bot authenticated (events api)"
    );

    let cancel = tokio_util::sync::CancellationToken::new();

    {
        let mut accts = accounts.write().unwrap_or_else(|e| e.into_inner());
        accts.insert(account_id.to_string(), AccountState {
            account_id: account_id.to_string(),
            config,
            message_log,
            event_sink,
            cancel,
            bot_user_id: Some(bot_user_id),
            pending_threads: std::collections::HashMap::new(),
        });
    }

    Ok(())
}

/// Verify the Slack request signature (HMAC-SHA256).
///
/// Slack sends:
/// - `X-Slack-Signature`: `v0=<hex-hmac>`
/// - `X-Slack-Request-Timestamp`: epoch seconds
///
/// The HMAC base string is `v0:{timestamp}:{body}`.
pub fn verify_signature(
    signing_secret: &str,
    timestamp: &str,
    body: &[u8],
    signature: &str,
) -> bool {
    let sig_hex = match signature.strip_prefix("v0=") {
        Some(hex) => hex,
        None => return false,
    };

    let Ok(sig_bytes) = hex_decode(sig_hex) else {
        return false;
    };

    // Use the hmac crate's verify_slice for constant-time comparison.
    let Ok(mut mac) = HmacSha256::new_from_slice(signing_secret.as_bytes()) else {
        return false;
    };
    mac.update(b"v0:");
    mac.update(timestamp.as_bytes());
    mac.update(b":");
    mac.update(body);
    mac.verify_slice(&sig_bytes).is_ok()
}

/// Simple hex decoding (avoids adding another dependency).
fn hex_decode(hex: &str) -> Result<Vec<u8>, ()> {
    if !hex.len().is_multiple_of(2) {
        return Err(());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| ()))
        .collect()
}

/// Handle an Events API webhook request.
///
/// Returns `Ok(Some(challenge))` for URL verification requests,
/// `Ok(None)` for normal event dispatches.
pub async fn handle_webhook(
    account_id: &str,
    body: &[u8],
    timestamp: &str,
    signature: &str,
    accounts: &AccountStateMap,
) -> moltis_channels::Result<Option<String>> {
    // Look up the account and verify the signature.
    let signing_secret = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accts
            .get(account_id)
            .ok_or_else(|| moltis_channels::Error::unknown_account(account_id))?;
        state
            .config
            .signing_secret
            .as_ref()
            .map(|s| s.expose_secret().clone())
            .ok_or_else(|| moltis_channels::Error::invalid_input("signing_secret not configured"))?
    };

    if !verify_signature(&signing_secret, timestamp, body, signature) {
        return Err(moltis_channels::Error::invalid_input(
            "invalid Slack webhook signature",
        ));
    }

    let payload: serde_json::Value = serde_json::from_slice(body)?;

    // URL verification challenge.
    if payload.get("type").and_then(|v| v.as_str()) == Some("url_verification") {
        let challenge = payload
            .get("challenge")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        return Ok(Some(challenge));
    }

    // Event callback.
    if payload.get("type").and_then(|v| v.as_str()) == Some("event_callback") {
        dispatch_event_callback(account_id, &payload, accounts).await;
    }

    Ok(None)
}

/// Handle an already-verified Events API webhook request.
///
/// The caller (channel webhook middleware) has already verified the signature,
/// checked timestamp staleness, and performed idempotency dedup.
///
/// Returns `Ok(Some(challenge))` for URL verification, `Ok(None)` for events.
pub async fn handle_verified_webhook(
    account_id: &str,
    body: &[u8],
    accounts: &AccountStateMap,
) -> moltis_channels::Result<Option<String>> {
    let payload: serde_json::Value = serde_json::from_slice(body)?;

    // URL verification challenge.
    if payload.get("type").and_then(|v| v.as_str()) == Some("url_verification") {
        let challenge = payload
            .get("challenge")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        return Ok(Some(challenge));
    }

    // Event callback.
    if payload.get("type").and_then(|v| v.as_str()) == Some("event_callback") {
        dispatch_event_callback(account_id, &payload, accounts).await;
    }

    Ok(None)
}

/// Handle an already-verified interaction webhook request.
///
/// The caller (channel webhook middleware) has already verified the signature.
pub async fn handle_verified_interaction_webhook(
    account_id: &str,
    body: &[u8],
    accounts: &AccountStateMap,
) -> moltis_channels::Result<()> {
    // Parse form-encoded body to extract `payload` field.
    let body_str = std::str::from_utf8(body)
        .map_err(|e| moltis_channels::Error::invalid_input(format!("invalid utf-8: {e}")))?;

    let payload_json = extract_form_payload(body_str).ok_or_else(|| {
        moltis_channels::Error::invalid_input("missing payload field in interaction")
    })?;

    let payload: serde_json::Value = serde_json::from_str(&payload_json)?;

    let interaction_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");

    if interaction_type != "block_actions" {
        debug!(account_id, interaction_type, "unhandled interaction type");
        return Ok(());
    }

    let action_id = payload
        .get("actions")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|a| a.get("action_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let channel_id = payload
        .get("channel")
        .and_then(|c| c.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if action_id.is_empty() || channel_id.is_empty() {
        debug!(account_id, "interaction missing action_id or channel");
        return Ok(());
    }

    let event_sink = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.event_sink.clone())
    };

    if let Some(sink) = event_sink {
        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::Slack,
            account_id: account_id.to_string(),
            chat_id: channel_id.to_string(),
            message_id: None,
        };
        match sink.dispatch_interaction(action_id, reply_to).await {
            Ok(_) => {},
            Err(e) => {
                debug!(account_id, action_id, "interaction dispatch failed: {e}");
            },
        }
    }

    Ok(())
}

/// Dispatch an event_callback payload to the appropriate handler.
async fn dispatch_event_callback(
    account_id: &str,
    payload: &serde_json::Value,
    accounts: &AccountStateMap,
) {
    let Some(event) = payload.get("event") else {
        debug!("event_callback missing event field");
        return;
    };

    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "message" => {
            // Parse as SlackMessageEvent via serde.
            match serde_json::from_value::<SlackMessageEvent>(event.clone()) {
                Ok(msg_event) => {
                    crate::socket::handle_message_event(account_id, msg_event, accounts).await;
                },
                Err(e) => {
                    debug!(account_id, "failed to parse message event: {e}");
                },
            }
        },
        "app_mention" => {
            // Parse app_mention event manually since the type may differ.
            let channel = event.get("channel").and_then(|v| v.as_str()).unwrap_or("");
            let user = event.get("user").and_then(|v| v.as_str()).unwrap_or("");
            let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let thread_ts = event
                .get("thread_ts")
                .and_then(|v| v.as_str())
                .map(String::from);

            if !channel.is_empty() && !user.is_empty() {
                crate::socket::handle_inbound(
                    account_id, channel, user, text, thread_ts, None, true, // is_mention
                    accounts,
                )
                .await;
            }
        },
        "reaction_added" | "reaction_removed" => {
            let user = event.get("user").and_then(|v| v.as_str()).unwrap_or("");
            let reaction = event.get("reaction").and_then(|v| v.as_str()).unwrap_or("");
            let item = event.get("item");

            if let Some(item) = item {
                // Extract channel and message_ts from the item.
                let item_channel = item.get("channel").and_then(|v| v.as_str()).unwrap_or("");
                let message_ts = item.get("ts").and_then(|v| v.as_str()).unwrap_or("");

                if !user.is_empty() && !reaction.is_empty() && !item_channel.is_empty() {
                    let added = event_type == "reaction_added";

                    // Dispatch reaction via the event sink directly since
                    // handle_reaction_event expects a SlackReactionsItem.
                    let event_sink = {
                        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
                        accts.get(account_id).and_then(|s| s.event_sink.clone())
                    };
                    if let Some(sink) = event_sink {
                        sink.emit(moltis_channels::ChannelEvent::ReactionChange {
                            channel_type: ChannelType::Slack,
                            account_id: account_id.to_string(),
                            chat_id: item_channel.to_string(),
                            message_id: message_ts.to_string(),
                            user_id: user.to_string(),
                            emoji: reaction.to_string(),
                            added,
                        })
                        .await;
                    }
                }
            }
        },
        _ => {
            debug!(account_id, event_type, "unhandled events api event type");
        },
    }
}

/// Handle an interaction payload from the Events API.
///
/// Slack sends interaction payloads as `application/x-www-form-urlencoded`
/// with a `payload` field containing JSON.
pub async fn handle_interaction_webhook(
    account_id: &str,
    body: &[u8],
    timestamp: &str,
    signature: &str,
    accounts: &AccountStateMap,
) -> moltis_channels::Result<()> {
    // Verify signature.
    let signing_secret = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accts
            .get(account_id)
            .ok_or_else(|| moltis_channels::Error::unknown_account(account_id))?;
        state
            .config
            .signing_secret
            .as_ref()
            .map(|s| s.expose_secret().clone())
            .ok_or_else(|| moltis_channels::Error::invalid_input("signing_secret not configured"))?
    };

    if !verify_signature(&signing_secret, timestamp, body, signature) {
        return Err(moltis_channels::Error::invalid_input(
            "invalid Slack webhook signature",
        ));
    }

    // Parse form-encoded body to extract `payload` field.
    let body_str = std::str::from_utf8(body)
        .map_err(|e| moltis_channels::Error::invalid_input(format!("invalid utf-8: {e}")))?;

    let payload_json = extract_form_payload(body_str).ok_or_else(|| {
        moltis_channels::Error::invalid_input("missing payload field in interaction")
    })?;

    let payload: serde_json::Value = serde_json::from_str(&payload_json)?;

    // Extract action from block_actions.
    let interaction_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");

    if interaction_type != "block_actions" {
        debug!(account_id, interaction_type, "unhandled interaction type");
        return Ok(());
    }

    let action_id = payload
        .get("actions")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|a| a.get("action_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let channel_id = payload
        .get("channel")
        .and_then(|c| c.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if action_id.is_empty() || channel_id.is_empty() {
        debug!(account_id, "interaction missing action_id or channel");
        return Ok(());
    }

    let event_sink = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.event_sink.clone())
    };

    if let Some(sink) = event_sink {
        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::Slack,
            account_id: account_id.to_string(),
            chat_id: channel_id.to_string(),
            message_id: None,
        };
        match sink.dispatch_interaction(action_id, reply_to).await {
            Ok(_) => {},
            Err(e) => {
                debug!(account_id, action_id, "interaction dispatch failed: {e}");
            },
        }
    }

    Ok(())
}

/// Extract the `payload` field from a `application/x-www-form-urlencoded` body.
fn extract_form_payload(body: &str) -> Option<String> {
    for pair in body.split('&') {
        if let Some(value) = pair.strip_prefix("payload=") {
            // URL-decode the value.
            return url_decode(value);
        }
    }
    None
}

/// Simple percent-decoding for URL-encoded strings.
fn url_decode(input: &str) -> Option<String> {
    let mut result = Vec::with_capacity(input.len());
    let mut chars = input.bytes();

    while let Some(b) = chars.next() {
        match b {
            b'%' => {
                let hi = chars.next()?;
                let lo = chars.next()?;
                let byte = u8::from_str_radix(&format!("{}{}", hi as char, lo as char), 16).ok()?;
                result.push(byte);
            },
            b'+' => result.push(b' '),
            _ => result.push(b),
        }
    }

    String::from_utf8(result).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn verify_valid_signature() {
        let secret = "8f742231b10e8888abcd99yez67543";
        let timestamp = "1531420618";
        let body = b"token=xyzz0WbapA4vBCDEFasx0YGkhA&team_id=T1DC2JH3J&team_domain=testteamnow&channel_id=G8LX6B8TU&channel_name=mpmulti&user_id=U2CERLKJA&user_name=roadrunner&command=%2Fwebhook-collect&text=&response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands%2FT1DC2JH3J%2F397700885554%2F96rGlfmibIGlgcZRskXaIFfN&trigger_id=398738663015.47445629121.803a0bc887a14d10d2c659f2945b6e92";

        // Compute expected signature.
        use hmac::Mac;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(body);
        let result = mac.finalize().into_bytes();
        let hex: String = result.iter().map(|b| format!("{b:02x}")).collect();
        let signature = format!("v0={hex}");

        assert!(verify_signature(secret, timestamp, body, &signature));
    }

    #[test]
    fn verify_rejects_bad_signature() {
        assert!(!verify_signature(
            "secret",
            "12345",
            b"body",
            "v0=0000000000000000000000000000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn verify_rejects_missing_prefix() {
        assert!(!verify_signature("secret", "12345", b"body", "bad"));
    }

    #[test]
    fn hex_decode_valid() {
        assert_eq!(hex_decode("48656c6c6f"), Ok(b"Hello".to_vec()));
    }

    #[test]
    fn hex_decode_odd_length() {
        assert!(hex_decode("abc").is_err());
    }

    #[test]
    fn url_decode_basic() {
        assert_eq!(
            url_decode("hello+world%21"),
            Some("hello world!".to_string())
        );
    }

    #[test]
    fn url_decode_json_payload() {
        let encoded = "%7B%22type%22%3A%22block_actions%22%7D";
        assert_eq!(
            url_decode(encoded),
            Some(r#"{"type":"block_actions"}"#.to_string())
        );
    }

    #[test]
    fn extract_form_payload_finds_payload() {
        let body = "token=abc&payload=%7B%22type%22%3A%22test%22%7D&other=val";
        let result = extract_form_payload(body);
        assert_eq!(result, Some(r#"{"type":"test"}"#.to_string()));
    }

    #[test]
    fn extract_form_payload_missing() {
        let body = "token=abc&other=val";
        assert!(extract_form_payload(body).is_none());
    }
}
