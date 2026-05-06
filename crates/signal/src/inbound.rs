//! Inbound Signal event handling.

use std::sync::{Arc, Mutex, RwLock};

use {
    moltis_channels::{
        ChannelEvent, ChannelEventSink, ChannelMessageKind, ChannelMessageMeta, ChannelReplyTarget,
        ChannelType,
        commands::{help_text, is_channel_command},
        gating::{DmPolicy, GroupPolicy, MentionMode, is_allowed},
        otp::{
            OtpInitResult, OtpVerifyResult, approve_sender_via_otp, emit_otp_challenge,
            emit_otp_resolution,
        },
    },
    serde_json::Value,
};

use crate::{client::SignalClient, config::SignalAccountConfig};

const OTP_CHALLENGE_PREFIX: &str =
    "This Signal sender is not approved yet. Reply with this PIN to approve access:";

pub async fn handle_event(
    raw: &Value,
    client: &SignalClient,
    config: &Arc<RwLock<SignalAccountConfig>>,
    otp: &Arc<Mutex<moltis_channels::otp::OtpState>>,
    account_id: &str,
    event_sink: &Arc<dyn ChannelEventSink>,
) {
    let Some(envelope) = extract_envelope(raw) else {
        return;
    };

    let cfg = config.read().unwrap_or_else(|e| e.into_inner()).clone();
    if cfg.ignore_stories && envelope.get("storyMessage").is_some() {
        return;
    }
    if envelope.get("syncMessage").is_some() {
        return;
    }

    let data_message = envelope.get("dataMessage").or_else(|| {
        envelope
            .get("editMessage")
            .and_then(|v| v.get("dataMessage"))
    });
    let Some(data_message) = data_message else {
        return;
    };

    let sender = first_string(envelope, &["sourceNumber", "sourceUuid", "source"]);
    let Some(sender) = sender else {
        return;
    };
    if is_self_sender(&sender, envelope, &cfg) {
        return;
    }

    let sender_uuid = string_field(envelope, "sourceUuid");
    let sender_name = string_field(envelope, "sourceName");
    let text = render_mentions(
        string_field(data_message, "message").unwrap_or_default(),
        data_message.get("mentions"),
    );
    let group_info = data_message.get("groupInfo");
    let group_id = group_info.and_then(|v| string_field(v, "groupId"));
    let group_name = group_info.and_then(|v| string_field(v, "groupName"));
    let is_group = group_id.is_some();
    let chat_id = group_id
        .as_ref()
        .map_or_else(|| sender.clone(), |id| format!("group:{id}"));
    let message_id = envelope
        .get("timestamp")
        .and_then(Value::as_i64)
        .map(|v| v.to_string());

    if !is_group {
        if handle_pending_otp(
            &text,
            &sender,
            &sender_uuid,
            otp,
            client,
            &cfg,
            account_id,
            event_sink,
        )
        .await
        {
            return;
        }

        if !dm_access_allowed(&sender, sender_uuid.as_deref(), &cfg) {
            if cfg.dm_policy == DmPolicy::Allowlist && cfg.otp_self_approval {
                issue_otp_challenge(
                    &sender,
                    sender_uuid.as_deref(),
                    sender_name.as_deref(),
                    otp,
                    client,
                    &cfg,
                    account_id,
                    event_sink,
                )
                .await;
            }
            return;
        }
    } else if !group_access_allowed(
        group_id.as_deref(),
        &text,
        data_message.get("mentions"),
        &cfg,
    ) {
        return;
    }

    let reply_to = ChannelReplyTarget {
        channel_type: ChannelType::Signal,
        account_id: account_id.to_string(),
        chat_id: chat_id.clone(),
        message_id,
        thread_id: None,
    };

    if let Some(cmd_text) = text.strip_prefix('/') {
        let cmd_name = cmd_text.split_whitespace().next().unwrap_or("");
        if is_channel_command(cmd_name, cmd_text) {
            let response = if cmd_name == "help" {
                Ok(help_text())
            } else {
                event_sink
                    .dispatch_command(cmd_text, reply_to, Some(&sender))
                    .await
            };
            let reply_text = response.unwrap_or_else(|e| format!("Error: {e}"));
            if let Err(e) = send_direct_text(client, &cfg, &chat_id, &reply_text).await {
                tracing::warn!(
                    account_id,
                    chat_id,
                    "failed to send Signal command response: {e}"
                );
            }
            return;
        }
    }

    event_sink
        .emit(ChannelEvent::InboundMessage {
            channel_type: ChannelType::Signal,
            account_id: account_id.to_string(),
            peer_id: chat_id.clone(),
            username: sender_uuid.clone(),
            sender_name: sender_name.clone(),
            message_count: None,
            access_granted: true,
        })
        .await;

    let meta = ChannelMessageMeta {
        channel_type: ChannelType::Signal,
        sender_name,
        username: sender_uuid,
        sender_id: Some(sender.clone()),
        message_kind: Some(ChannelMessageKind::Text),
        model: cfg.model.clone(),
        agent_id: cfg.agent_id.clone(),
        audio_filename: None,
        documents: None,
    };

    let body = if text.is_empty() && data_message.get("attachments").is_some() {
        "[Signal attachment received. Attachment ingestion is not enabled in this build.]"
    } else {
        text.as_str()
    };
    let display_body = if is_group {
        match (group_name.as_deref(), group_id.as_deref()) {
            (Some(name), _) => format!("[Signal group: {name}]\n{body}"),
            (None, Some(id)) => format!("[Signal group: {id}]\n{body}"),
            _ => body.to_string(),
        }
    } else {
        body.to_string()
    };

    event_sink
        .dispatch_to_chat(&display_body, reply_to, meta)
        .await;
}

fn extract_envelope(raw: &Value) -> Option<&Value> {
    if raw.get("method").and_then(Value::as_str) == Some("receive") {
        let params = raw.get("params")?;
        if let Some(envelope) = params.get("envelope") {
            return Some(envelope);
        }
        return params.get("result").and_then(|v| v.get("envelope"));
    }
    raw.get("envelope")
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn first_string(value: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| string_field(value, field))
}

fn is_self_sender(sender: &str, envelope: &Value, cfg: &SignalAccountConfig) -> bool {
    let account_matches = cfg.account().is_some_and(|account| account == sender);
    let uuid_matches = cfg.account_uuid.as_deref().is_some_and(|uuid| {
        sender == uuid || string_field(envelope, "sourceUuid").as_deref() == Some(uuid)
    });
    account_matches || uuid_matches
}

/// Convert a UTF-16 code-unit offset to a UTF-8 byte offset within `text`.
fn utf16_to_utf8_offset(text: &str, utf16_offset: usize) -> Option<usize> {
    let mut utf16_pos = 0;
    for (byte_pos, ch) in text.char_indices() {
        if utf16_pos == utf16_offset {
            return Some(byte_pos);
        }
        utf16_pos += ch.len_utf16();
        if utf16_pos > utf16_offset {
            return None; // offset falls inside a surrogate pair
        }
    }
    // offset at the very end of the string
    if utf16_pos == utf16_offset {
        return Some(text.len());
    }
    None
}

fn render_mentions(mut text: String, mentions: Option<&Value>) -> String {
    let Some(mentions) = mentions.and_then(Value::as_array) else {
        return text;
    };
    if !text.contains('\u{fffc}') {
        return text;
    }

    // Signal provides mention positions as UTF-16 code-unit indices.
    // Convert to UTF-8 byte offsets for Rust string operations.
    let mut replacements: Vec<(usize, usize, String)> = mentions
        .iter()
        .filter_map(|mention| {
            let utf16_start = mention.get("start")?.as_u64()? as usize;
            let utf16_len = mention.get("length").and_then(Value::as_u64).unwrap_or(1) as usize;
            let byte_start = utf16_to_utf8_offset(&text, utf16_start)?;
            let byte_end = utf16_to_utf8_offset(&text, utf16_start + utf16_len)?;
            let identifier = string_field(mention, "number")
                .or_else(|| string_field(mention, "uuid"))
                .unwrap_or_else(|| "user".to_string());
            Some((byte_start, byte_end - byte_start, format!("@{identifier}")))
        })
        .collect();
    replacements.sort_by_key(|item| std::cmp::Reverse(item.0));

    for (start, length, replacement) in replacements {
        if start + length <= text.len() {
            text.replace_range(start..start + length, &replacement);
        }
    }
    text
}

fn dm_access_allowed(sender: &str, sender_uuid: Option<&str>, cfg: &SignalAccountConfig) -> bool {
    match cfg.dm_policy {
        DmPolicy::Open => true,
        DmPolicy::Disabled => false,
        DmPolicy::Allowlist => identity_allowed(sender, sender_uuid, &cfg.allowlist),
    }
}

fn identity_allowed(sender: &str, sender_uuid: Option<&str>, allowlist: &[String]) -> bool {
    if allowlist.is_empty() {
        return false;
    }
    if is_allowed(sender, allowlist) {
        return true;
    }
    sender_uuid.is_some_and(|uuid| {
        is_allowed(uuid, allowlist) || is_allowed(&format!("uuid:{uuid}"), allowlist)
    })
}

fn group_access_allowed(
    group_id: Option<&str>,
    text: &str,
    mentions: Option<&Value>,
    cfg: &SignalAccountConfig,
) -> bool {
    let Some(group_id) = group_id else {
        return false;
    };
    let policy_allows = match cfg.group_policy {
        GroupPolicy::Open => true,
        GroupPolicy::Allowlist => is_allowed(group_id, &cfg.group_allowlist),
        GroupPolicy::Disabled => false,
    };
    if !policy_allows {
        return false;
    }

    match cfg.mention_mode {
        MentionMode::Always => true,
        MentionMode::None => false,
        MentionMode::Mention => {
            let account_mentioned = cfg.account().is_some_and(|account| text.contains(account));
            let uuid_mentioned = cfg
                .account_uuid
                .as_deref()
                .is_some_and(|uuid| text.contains(uuid));
            let bot_mentioned_in_array = mentions.and_then(Value::as_array).is_some_and(|arr| {
                arr.iter().any(|m| {
                    let num = string_field(m, "number");
                    let uuid = string_field(m, "uuid");
                    let matches_account = cfg.account().is_some_and(|a| num.as_deref() == Some(a));
                    let matches_uuid = cfg
                        .account_uuid
                        .as_deref()
                        .is_some_and(|u| uuid.as_deref() == Some(u));
                    matches_account || matches_uuid
                })
            });
            account_mentioned || uuid_mentioned || bot_mentioned_in_array
        },
    }
}

async fn handle_pending_otp(
    text: &str,
    sender: &str,
    sender_uuid: &Option<String>,
    otp: &Arc<Mutex<moltis_channels::otp::OtpState>>,
    client: &SignalClient,
    cfg: &SignalAccountConfig,
    account_id: &str,
    event_sink: &Arc<dyn ChannelEventSink>,
) -> bool {
    let has_pending = {
        let guard = otp.lock().unwrap_or_else(|e| e.into_inner());
        guard.has_pending(sender)
    };
    if !has_pending {
        return false;
    }

    let code = text.trim();
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }

    let result = {
        let mut guard = otp.lock().unwrap_or_else(|e| e.into_inner());
        guard.verify(sender, code)
    };
    let (reply, resolution) = match result {
        OtpVerifyResult::Approved => {
            approve_sender_via_otp(
                Some(event_sink.as_ref()),
                ChannelType::Signal,
                account_id,
                sender,
                sender,
                sender_uuid.as_deref(),
            )
            .await;
            ("Access granted. You can now send messages.", "approved")
        },
        OtpVerifyResult::WrongCode { attempts_left } => (
            if attempts_left > 0 {
                "Wrong PIN. Please try again."
            } else {
                "Too many failed attempts. You are temporarily locked out."
            },
            "wrong_code",
        ),
        OtpVerifyResult::LockedOut => (
            "Too many failed attempts. You are temporarily locked out.",
            "locked_out",
        ),
        OtpVerifyResult::NoPending => ("No pending challenge.", "no_pending"),
        OtpVerifyResult::Expired => (
            "PIN expired. Send another message to get a new PIN.",
            "expired",
        ),
    };

    if resolution != "approved" {
        emit_otp_resolution(
            Some(event_sink.as_ref()),
            ChannelType::Signal,
            account_id,
            sender,
            sender_uuid.as_deref(),
            resolution,
        )
        .await;
    }
    if let Err(e) = send_direct_text(client, cfg, sender, reply).await {
        tracing::warn!(account_id, sender, "failed to send Signal OTP result: {e}");
    }
    true
}

async fn issue_otp_challenge(
    sender: &str,
    sender_uuid: Option<&str>,
    sender_name: Option<&str>,
    otp: &Arc<Mutex<moltis_channels::otp::OtpState>>,
    client: &SignalClient,
    cfg: &SignalAccountConfig,
    account_id: &str,
    event_sink: &Arc<dyn ChannelEventSink>,
) {
    let init = {
        let mut guard = otp.lock().unwrap_or_else(|e| e.into_inner());
        guard.initiate(
            sender,
            sender_uuid.map(ToString::to_string),
            sender_name.map(ToString::to_string),
        )
    };

    match init {
        OtpInitResult::Created(code) => {
            let expires_at = time::OffsetDateTime::now_utc().unix_timestamp() + 300;
            emit_otp_challenge(
                Some(event_sink.as_ref()),
                ChannelType::Signal,
                account_id,
                sender,
                sender_uuid,
                sender_name,
                code.clone(),
                expires_at,
            )
            .await;
            let message = format!("{OTP_CHALLENGE_PREFIX} {code}");
            if let Err(e) = send_direct_text(client, cfg, sender, &message).await {
                tracing::warn!(
                    account_id,
                    sender,
                    "failed to send Signal OTP challenge: {e}"
                );
            }
        },
        OtpInitResult::AlreadyPending => {},
        OtpInitResult::LockedOut => {
            if let Err(e) = send_direct_text(
                client,
                cfg,
                sender,
                "Too many failed approval attempts. Try again later.",
            )
            .await
            {
                tracing::warn!(account_id, sender, "failed to send Signal OTP lockout: {e}");
            }
        },
    }
}

async fn send_direct_text(
    client: &SignalClient,
    cfg: &SignalAccountConfig,
    to: &str,
    text: &str,
) -> moltis_channels::Result<()> {
    let mut params = serde_json::json!({
        "message": text,
    });
    let Some(obj) = params.as_object_mut() else {
        return Ok(());
    };
    if let Some(group_id) = to.strip_prefix("group:") {
        obj.insert("groupId".to_string(), serde_json::json!(group_id));
    } else {
        obj.insert("recipient".to_string(), serde_json::json!([to]));
    }
    if let Some(account) = cfg.account() {
        obj.insert("account".to_string(), serde_json::json!(account));
    }
    let _ = client.rpc_value(&cfg.http_url, "send", params).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use {
        crate::config::SignalAccountConfig,
        moltis_channels::gating::{DmPolicy, GroupPolicy, MentionMode},
    };

    #[test]
    fn dm_allowlist_matches_uuid_variants() {
        let cfg = SignalAccountConfig {
            dm_policy: DmPolicy::Allowlist,
            allowlist: vec!["uuid:abc".to_string()],
            ..Default::default()
        };
        assert!(crate::inbound::dm_access_allowed(
            "+1555",
            Some("abc"),
            &cfg
        ));
    }

    #[test]
    fn groups_are_disabled_by_default() {
        let cfg = SignalAccountConfig::default();
        assert!(!crate::inbound::group_access_allowed(
            Some("group-id"),
            "hello",
            None,
            &cfg
        ));
    }

    #[test]
    fn group_mentions_gate_open_groups() {
        let cfg = SignalAccountConfig {
            group_policy: GroupPolicy::Open,
            mention_mode: MentionMode::Mention,
            account: Some("+15551234567".to_string()),
            ..Default::default()
        };
        assert!(!crate::inbound::group_access_allowed(
            Some("group-id"),
            "hello",
            None,
            &cfg
        ));
        assert!(crate::inbound::group_access_allowed(
            Some("group-id"),
            "hello +15551234567",
            None,
            &cfg
        ));
    }

    #[test]
    fn group_mention_mode_rejects_non_bot_mentions() {
        let cfg = SignalAccountConfig {
            group_policy: GroupPolicy::Open,
            mention_mode: MentionMode::Mention,
            account: Some("+15551234567".to_string()),
            account_uuid: Some("bot-uuid".to_string()),
            ..Default::default()
        };
        // A mention of someone else should not trigger the bot.
        let mentions = serde_json::json!([
            {"start": 0, "length": 1, "number": "+19998887777", "uuid": "other-uuid"}
        ]);
        assert!(!crate::inbound::group_access_allowed(
            Some("group-id"),
            "hello",
            Some(&mentions),
            &cfg
        ));
        // A mention of the bot's UUID should trigger.
        let bot_mention = serde_json::json!([
            {"start": 0, "length": 1, "uuid": "bot-uuid"}
        ]);
        assert!(crate::inbound::group_access_allowed(
            Some("group-id"),
            "hello",
            Some(&bot_mention),
            &cfg
        ));
    }

    #[test]
    fn render_mentions_handles_utf16_offsets() {
        // "Hey 😀 \u{FFFC}" — emoji is 2 UTF-16 code units, 4 UTF-8 bytes.
        // UTF-16 positions: H=0, e=1, y=2, ' '=3, 😀=4-5, ' '=6, \u{FFFC}=7
        let text = "Hey \u{1F600} \u{FFFC}".to_string();
        let mentions = serde_json::json!([
            {"start": 7, "length": 1, "number": "+15551234567"}
        ]);
        let result = crate::inbound::render_mentions(text, Some(&mentions));
        assert_eq!(result, "Hey \u{1F600} @+15551234567");
    }

    #[test]
    fn utf16_to_utf8_offset_basic() {
        // ASCII only
        assert_eq!(crate::inbound::utf16_to_utf8_offset("hello", 0), Some(0));
        assert_eq!(crate::inbound::utf16_to_utf8_offset("hello", 5), Some(5));
        // With emoji (U+1F600 = 2 UTF-16 units, 4 UTF-8 bytes)
        let s = "a\u{1F600}b";
        assert_eq!(crate::inbound::utf16_to_utf8_offset(s, 0), Some(0)); // 'a'
        assert_eq!(crate::inbound::utf16_to_utf8_offset(s, 1), Some(1)); // start of emoji
        assert_eq!(crate::inbound::utf16_to_utf8_offset(s, 2), None); // inside surrogate pair
        assert_eq!(crate::inbound::utf16_to_utf8_offset(s, 3), Some(5)); // 'b'
    }
}
