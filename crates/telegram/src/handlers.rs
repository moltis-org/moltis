use std::sync::Arc;

use {
    teloxide::{
        payloads::SendMessageSetters,
        prelude::*,
        types::{
            CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, MediaKind, MessageKind,
            ParseMode,
        },
    },
    tracing::{debug, warn},
};

use {
    moltis_channels::{
        ChannelEvent, ChannelMessageMeta, ChannelOutbound, ChannelReplyTarget,
        message_log::MessageLogEntry,
    },
    moltis_common::types::ChatType,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, telegram as tg_metrics};

use crate::{
    access::{self, AccessDenied},
    otp::{OtpInitResult, OtpVerifyResult},
    state::AccountStateMap,
};

/// Shared context injected into teloxide's dispatcher.
#[derive(Clone)]
pub struct HandlerContext {
    pub accounts: AccountStateMap,
    pub account_id: String,
}

/// Build the teloxide update handler.
pub fn build_handler() -> Handler<
    'static,
    DependencyMap,
    Result<(), Box<dyn std::error::Error + Send + Sync>>,
    teloxide::dispatching::DpHandlerDescription,
> {
    Update::filter_message().endpoint(handle_message)
}

/// Handle a single inbound Telegram message (called from manual polling loop).
pub async fn handle_message_direct(
    msg: Message,
    _bot: &Bot,
    account_id: &str,
    accounts: &AccountStateMap,
) -> anyhow::Result<()> {
    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    #[cfg(feature = "metrics")]
    counter!(tg_metrics::MESSAGES_RECEIVED_TOTAL).increment(1);

    let text = extract_text(&msg);
    if text.is_none() && !has_media(&msg) {
        debug!(account_id, "ignoring non-text, non-media message");
        return Ok(());
    }

    let (config, bot_username, _outbound, message_log, event_sink) = {
        let accts = accounts.read().unwrap();
        let state = match accts.get(account_id) {
            Some(s) => s,
            None => {
                warn!(account_id, "handler: account not found in state map");
                return Ok(());
            },
        };
        (
            state.config.clone(),
            state.bot_username.clone(),
            Arc::clone(&state.outbound),
            state.message_log.clone(),
            state.event_sink.clone(),
        )
    };

    let (chat_type, group_id) = classify_chat(&msg);
    let peer_id = msg
        .from
        .as_ref()
        .map(|u| u.id.0.to_string())
        .unwrap_or_default();
    let sender_name = msg.from.as_ref().and_then(|u| {
        let first = &u.first_name;
        let last = u.last_name.as_deref().unwrap_or("");
        let name = format!("{first} {last}").trim().to_string();
        if name.is_empty() {
            u.username.clone()
        } else {
            Some(name)
        }
    });

    let bot_mentioned = check_bot_mentioned(&msg, bot_username.as_deref());

    debug!(
        account_id,
        ?chat_type,
        peer_id,
        ?bot_mentioned,
        "checking access"
    );

    let username = msg.from.as_ref().and_then(|u| u.username.clone());

    // Access control
    let access_result = access::check_access(
        &config,
        &chat_type,
        &peer_id,
        username.as_deref(),
        group_id.as_deref(),
        bot_mentioned,
    );
    let access_granted = access_result.is_ok();

    // Log every inbound message (before returning on denial).
    if let Some(ref log) = message_log {
        let chat_type_str = match chat_type {
            ChatType::Dm => "dm",
            ChatType::Group => "group",
            ChatType::Channel => "channel",
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let entry = MessageLogEntry {
            id: 0,
            account_id: account_id.to_string(),
            channel_type: "telegram".into(),
            peer_id: peer_id.clone(),
            username: username.clone(),
            sender_name: sender_name.clone(),
            chat_id: msg.chat.id.0.to_string(),
            chat_type: chat_type_str.into(),
            body: text.clone().unwrap_or_default(),
            access_granted,
            created_at: now,
        };
        if let Err(e) = log.log(entry).await {
            warn!(account_id, "failed to log message: {e}");
        }
    }

    // Emit channel event for real-time UI updates.
    if let Some(ref sink) = event_sink {
        sink.emit(ChannelEvent::InboundMessage {
            channel_type: "telegram".into(),
            account_id: account_id.to_string(),
            peer_id: peer_id.clone(),
            username: username.clone(),
            sender_name: sender_name.clone(),
            message_count: None,
            access_granted,
        })
        .await;
    }

    if let Err(reason) = access_result {
        warn!(account_id, %reason, peer_id, username = ?username, "handler: access denied");
        #[cfg(feature = "metrics")]
        counter!(tg_metrics::ACCESS_CONTROL_DENIALS_TOTAL).increment(1);

        // OTP self-approval for non-allowlisted DM users.
        if reason == AccessDenied::NotOnAllowlist
            && chat_type == ChatType::Dm
            && config.otp_self_approval
        {
            handle_otp_flow(
                accounts,
                account_id,
                &peer_id,
                username.as_deref(),
                sender_name.as_deref(),
                text.as_deref(),
                &msg,
                event_sink.as_deref(),
            )
            .await;
        }

        return Ok(());
    }

    debug!(account_id, "handler: access granted");

    let body = text.unwrap_or_default();

    // Dispatch to the chat session (per-channel session key derived by the sink).
    // The reply target tells the gateway where to send the LLM response back.
    if let Some(ref sink) = event_sink
        && !body.is_empty()
    {
        let reply_target = ChannelReplyTarget {
            channel_type: "telegram".into(),
            account_id: account_id.to_string(),
            chat_id: msg.chat.id.0.to_string(),
        };

        // Intercept slash commands before dispatching to the LLM.
        if body.starts_with('/') {
            let cmd_text = body.trim_start_matches('/');
            let cmd = cmd_text.split_whitespace().next().unwrap_or("");
            if matches!(
                cmd,
                "new" | "clear" | "compact" | "context" | "model" | "sandbox" | "sessions" | "help"
            ) {
                // For /context, send a formatted card with inline keyboard.
                if cmd == "context" {
                    let context_result =
                        sink.dispatch_command("context", reply_target.clone()).await;
                    let bot = {
                        let accts = accounts.read().unwrap();
                        accts.get(account_id).map(|s| s.bot.clone())
                    };
                    if let Some(bot) = bot {
                        match context_result {
                            Ok(text) => {
                                send_context_card(&bot, &reply_target.chat_id, &text).await;
                            },
                            Err(e) => {
                                let _ = bot
                                    .send_message(
                                        ChatId(reply_target.chat_id.parse().unwrap_or(0)),
                                        format!("Error: {e}"),
                                    )
                                    .await;
                            },
                        }
                    }
                    return Ok(());
                }

                // For /model without args, send an inline keyboard to pick a model.
                if cmd == "model" && cmd_text.trim() == "model" {
                    let list_result = sink.dispatch_command("model", reply_target.clone()).await;
                    let bot = {
                        let accts = accounts.read().unwrap();
                        accts.get(account_id).map(|s| s.bot.clone())
                    };
                    if let Some(bot) = bot {
                        match list_result {
                            Ok(text) => {
                                send_model_keyboard(&bot, &reply_target.chat_id, &text).await;
                            },
                            Err(e) => {
                                let _ = bot
                                    .send_message(
                                        ChatId(reply_target.chat_id.parse().unwrap_or(0)),
                                        format!("Error: {e}"),
                                    )
                                    .await;
                            },
                        }
                    }
                    return Ok(());
                }

                // For /sandbox without args, send toggle + image keyboard.
                if cmd == "sandbox" && cmd_text.trim() == "sandbox" {
                    let list_result = sink.dispatch_command("sandbox", reply_target.clone()).await;
                    let bot = {
                        let accts = accounts.read().unwrap();
                        accts.get(account_id).map(|s| s.bot.clone())
                    };
                    if let Some(bot) = bot {
                        match list_result {
                            Ok(text) => {
                                send_sandbox_keyboard(&bot, &reply_target.chat_id, &text).await;
                            },
                            Err(e) => {
                                let _ = bot
                                    .send_message(
                                        ChatId(reply_target.chat_id.parse().unwrap_or(0)),
                                        format!("Error: {e}"),
                                    )
                                    .await;
                            },
                        }
                    }
                    return Ok(());
                }

                // For /sessions without args, send an inline keyboard instead of plain text.
                if cmd == "sessions" && cmd_text.trim() == "sessions" {
                    let list_result = sink
                        .dispatch_command("sessions", reply_target.clone())
                        .await;
                    let bot = {
                        let accts = accounts.read().unwrap();
                        accts.get(account_id).map(|s| s.bot.clone())
                    };
                    if let Some(bot) = bot {
                        match list_result {
                            Ok(text) => {
                                send_sessions_keyboard(&bot, &reply_target.chat_id, &text).await;
                            },
                            Err(e) => {
                                let _ = bot
                                    .send_message(
                                        ChatId(reply_target.chat_id.parse().unwrap_or(0)),
                                        format!("Error: {e}"),
                                    )
                                    .await;
                            },
                        }
                    }
                    return Ok(());
                }

                let response = if cmd == "help" {
                    "Available commands:\n/new — Start a new session\n/sessions — List and switch sessions\n/model — Switch provider/model\n/sandbox — Toggle sandbox and choose image\n/clear — Clear session history\n/compact — Compact session (summarize)\n/context — Show session context info\n/help — Show this help".to_string()
                } else {
                    match sink.dispatch_command(cmd_text, reply_target.clone()).await {
                        Ok(msg) => msg,
                        Err(e) => format!("Error: {e}"),
                    }
                };
                // Get the outbound Arc before awaiting (avoid holding RwLockReadGuard across await).
                let outbound = {
                    let accts = accounts.read().unwrap();
                    accts.get(account_id).map(|s| Arc::clone(&s.outbound))
                };
                if let Some(outbound) = outbound
                    && let Err(e) = outbound
                        .send_text(account_id, &reply_target.chat_id, &response)
                        .await
                {
                    warn!(account_id, "failed to send command response: {e}");
                }
                return Ok(());
            }
        }

        let meta = ChannelMessageMeta {
            channel_type: "telegram".into(),
            sender_name: sender_name.clone(),
            username: username.clone(),
            model: config.model.clone(),
        };
        sink.dispatch_to_chat(&body, reply_target, meta).await;
    }

    #[cfg(feature = "metrics")]
    histogram!(tg_metrics::POLLING_DURATION_SECONDS).record(start.elapsed().as_secs_f64());

    Ok(())
}

/// OTP challenge message sent to the Telegram user.
///
/// **Security invariant:** this message must NEVER contain the actual
/// verification code.  The code is only visible to the bot owner in the
/// web UI (Channels → Senders).  Leaking it here would let any
/// unauthenticated user self-approve without admin awareness.
pub(crate) const OTP_CHALLENGE_MSG: &str = "To use this bot, please enter the verification code.\n\nAsk the bot owner for the code \u{2014} it is visible in the web UI under <b>Channels \u{2192} Senders</b>.\n\nThe code expires in 5 minutes.";

/// Handle OTP challenge/verification flow for a non-allowlisted DM user.
///
/// Called when `dm_policy = Allowlist`, the peer is not on the allowlist, and
/// `otp_self_approval` is enabled. Manages the full lifecycle:
/// - First message: issue a 6-digit OTP challenge
/// - Code reply: verify and auto-approve on match
/// - Non-code messages while pending: silently ignored (flood protection)
#[allow(clippy::too_many_arguments)]
async fn handle_otp_flow(
    accounts: &AccountStateMap,
    account_id: &str,
    peer_id: &str,
    username: Option<&str>,
    sender_name: Option<&str>,
    text: Option<&str>,
    msg: &Message,
    event_sink: Option<&dyn moltis_channels::ChannelEventSink>,
) {
    let chat_id = msg.chat.id;

    // Resolve bot early (needed for sending messages).
    let bot = {
        let accts = accounts.read().unwrap();
        accts.get(account_id).map(|s| s.bot.clone())
    };
    let bot = match bot {
        Some(b) => b,
        None => return,
    };

    // Check current OTP state.
    let has_pending = {
        let accts = accounts.read().unwrap();
        accts
            .get(account_id)
            .map(|s| {
                let otp = s.otp.lock().unwrap();
                otp.has_pending(peer_id)
            })
            .unwrap_or(false)
    };

    if has_pending {
        // A challenge is already pending. Check if the user sent a 6-digit code.
        let body = text.unwrap_or("").trim();
        let is_code = body.len() == 6 && body.chars().all(|c| c.is_ascii_digit());

        if !is_code {
            // Silent ignore — flood protection.
            return;
        }

        // Verify the code.
        let result = {
            let accts = accounts.read().unwrap();
            match accts.get(account_id) {
                Some(s) => {
                    let mut otp = s.otp.lock().unwrap();
                    otp.verify(peer_id, body)
                },
                None => return,
            }
        };

        match result {
            OtpVerifyResult::Approved => {
                // Auto-approve: add to allowlist via the event sink.
                let identifier = username.unwrap_or(peer_id);
                if let Some(sink) = event_sink {
                    sink.request_sender_approval("telegram", account_id, identifier)
                        .await;
                }

                let _ = bot
                    .send_message(chat_id, "Verified! You now have access to this bot.")
                    .await;

                // Emit resolved event.
                if let Some(sink) = event_sink {
                    sink.emit(ChannelEvent::OtpResolved {
                        channel_type: "telegram".into(),
                        account_id: account_id.to_string(),
                        peer_id: peer_id.to_string(),
                        username: username.map(String::from),
                        resolution: "approved".into(),
                    })
                    .await;
                }

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "approved").increment(1);
            },
            OtpVerifyResult::WrongCode { attempts_left } => {
                let _ = bot
                    .send_message(
                        chat_id,
                        format!(
                            "Incorrect code. {attempts_left} attempt{} remaining.",
                            if attempts_left == 1 {
                                ""
                            } else {
                                "s"
                            }
                        ),
                    )
                    .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "wrong_code")
                    .increment(1);
            },
            OtpVerifyResult::LockedOut => {
                let _ = bot
                    .send_message(chat_id, "Too many failed attempts. Please try again later.")
                    .await;

                if let Some(sink) = event_sink {
                    sink.emit(ChannelEvent::OtpResolved {
                        channel_type: "telegram".into(),
                        account_id: account_id.to_string(),
                        peer_id: peer_id.to_string(),
                        username: username.map(String::from),
                        resolution: "locked_out".into(),
                    })
                    .await;
                }

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "locked_out")
                    .increment(1);
            },
            OtpVerifyResult::Expired => {
                let _ = bot
                    .send_message(
                        chat_id,
                        "Your code has expired. Send any message to get a new one.",
                    )
                    .await;

                if let Some(sink) = event_sink {
                    sink.emit(ChannelEvent::OtpResolved {
                        channel_type: "telegram".into(),
                        account_id: account_id.to_string(),
                        peer_id: peer_id.to_string(),
                        username: username.map(String::from),
                        resolution: "expired".into(),
                    })
                    .await;
                }

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "expired").increment(1);
            },
            OtpVerifyResult::NoPending => {
                // Shouldn't happen since we checked has_pending, but handle gracefully.
            },
        }
    } else {
        // No pending challenge — initiate one.
        let init_result = {
            let accts = accounts.read().unwrap();
            match accts.get(account_id) {
                Some(s) => {
                    let mut otp = s.otp.lock().unwrap();
                    otp.initiate(
                        peer_id,
                        username.map(String::from),
                        sender_name.map(String::from),
                    )
                },
                None => return,
            }
        };

        match init_result {
            OtpInitResult::Created(code) => {
                let _ = bot
                    .send_message(chat_id, OTP_CHALLENGE_MSG)
                    .parse_mode(ParseMode::Html)
                    .await;

                // Emit OTP challenge event for the admin UI.
                if let Some(sink) = event_sink {
                    // Compute expires_at epoch.
                    let expires_at = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64
                        + 300;

                    sink.emit(ChannelEvent::OtpChallenge {
                        channel_type: "telegram".into(),
                        account_id: account_id.to_string(),
                        peer_id: peer_id.to_string(),
                        username: username.map(String::from),
                        sender_name: sender_name.map(String::from),
                        code,
                        expires_at,
                    })
                    .await;
                }

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_CHALLENGES_TOTAL).increment(1);
            },
            OtpInitResult::AlreadyPending | OtpInitResult::LockedOut => {
                // Silent ignore.
            },
        }
    }
}

/// Handle a single inbound Telegram message (teloxide dispatcher endpoint).
async fn handle_message(
    msg: Message,
    bot: Bot,
    ctx: Arc<HandlerContext>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    handle_message_direct(msg, &bot, &ctx.account_id, &ctx.accounts)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Send a sessions list as an inline keyboard.
///
/// Parses the text response from `dispatch_command("sessions")` to extract
/// session labels, then sends an inline keyboard with one button per session.
async fn send_sessions_keyboard(bot: &Bot, chat_id: &str, sessions_text: &str) {
    let chat = ChatId(chat_id.parse().unwrap_or(0));

    // Parse numbered lines like "1. Session label (5 msgs) *"
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for line in sessions_text.lines() {
        let trimmed = line.trim();
        // Match lines starting with a number followed by ". "
        if let Some(dot_pos) = trimmed.find(". ")
            && let Ok(n) = trimmed[..dot_pos].parse::<usize>()
        {
            let label_part = &trimmed[dot_pos + 2..];
            let is_active = label_part.ends_with('*');
            let display = if is_active {
                format!("● {}", label_part.trim_end_matches('*').trim())
            } else {
                format!("○ {label_part}")
            };
            buttons.push(vec![InlineKeyboardButton::callback(
                display,
                format!("sessions_switch:{n}"),
            )]);
        }
    }

    if buttons.is_empty() {
        let _ = bot.send_message(chat, sessions_text).await;
        return;
    }

    let keyboard = InlineKeyboardMarkup::new(buttons);
    let _ = bot
        .send_message(chat, "Select a session:")
        .reply_markup(keyboard)
        .await;
}

/// Send context info as a formatted HTML card with blockquote sections.
///
/// Parses the markdown context response from `dispatch_command("context")`
/// and renders it as a structured Telegram HTML message.
async fn send_context_card(bot: &Bot, chat_id: &str, context_text: &str) {
    let chat = ChatId(chat_id.parse().unwrap_or(0));

    // Parse "**Key:** value" lines from the markdown response into a map.
    let mut fields: Vec<(&str, String)> = Vec::new();
    for line in context_text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("**")
            && let Some(end) = rest.find("**")
        {
            let label = &rest[..end];
            let raw_value = rest[end + 2..].trim();
            // Strip markdown backticks from value
            let value = raw_value.replace('`', "");
            fields.push((label, escape_html_simple(&value)));
        }
    }

    let get = |key: &str| -> String {
        fields
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };

    let session = get("Session:");
    let messages = get("Messages:");
    let provider = get("Provider:");
    let model = get("Model:");
    let sandbox = get("Sandbox:");
    let plugins_raw = get("Plugins:");
    let tokens = get("Tokens:");

    // Format plugins as individual lines
    let plugins_section = if plugins_raw == "none" || plugins_raw.is_empty() {
        "  <i>none</i>".to_string()
    } else {
        plugins_raw
            .split(", ")
            .map(|p| format!("  ▸ {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Sandbox indicator
    let sandbox_icon = if sandbox.starts_with("on") {
        "🟢"
    } else {
        "⚫"
    };

    let html = format!(
        "\
<b>📋 Session Context</b>

<blockquote><b>🤖 Model</b>
{provider} · <code>{model}</code>

<b>{sandbox_icon} Sandbox</b>
{sandbox}

<b>🧩 Plugins</b>
{plugins_section}</blockquote>

<code>Session   {session}
Messages  {messages}
Tokens    {tokens}</code>"
    );

    let _ = bot
        .send_message(chat, html)
        .parse_mode(ParseMode::Html)
        .await;
}

/// Send model selection as an inline keyboard.
///
/// If the response starts with `providers:`, show a provider picker first.
/// Otherwise show the model list directly.
async fn send_model_keyboard(bot: &Bot, chat_id: &str, text: &str) {
    let chat = ChatId(chat_id.parse().unwrap_or(0));

    let is_provider_list = text.starts_with("providers:");

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "providers:" {
            continue;
        }
        if let Some(dot_pos) = trimmed.find(". ")
            && let Ok(n) = trimmed[..dot_pos].parse::<usize>()
        {
            let label_part = &trimmed[dot_pos + 2..];
            let is_active = label_part.ends_with('*');
            let clean = label_part.trim_end_matches('*').trim();
            let display = if is_active {
                format!("● {clean}")
            } else {
                format!("○ {clean}")
            };

            if is_provider_list {
                // Extract provider name (before the parenthesized count).
                let provider_name = clean.rfind(" (").map(|i| &clean[..i]).unwrap_or(clean);
                buttons.push(vec![InlineKeyboardButton::callback(
                    display,
                    format!("model_provider:{provider_name}"),
                )]);
            } else {
                buttons.push(vec![InlineKeyboardButton::callback(
                    display,
                    format!("model_switch:{n}"),
                )]);
            }
        }
    }

    if buttons.is_empty() {
        let _ = bot.send_message(chat, "No models available.").await;
        return;
    }

    let heading = if is_provider_list {
        "🤖 Select a provider:"
    } else {
        "🤖 Select a model:"
    };

    let keyboard = InlineKeyboardMarkup::new(buttons);
    let _ = bot.send_message(chat, heading).reply_markup(keyboard).await;
}

/// Send sandbox status with toggle button and image picker.
///
/// First line is `status:on` or `status:off`. Remaining lines are numbered
/// images, with `*` marking the current one.
async fn send_sandbox_keyboard(bot: &Bot, chat_id: &str, text: &str) {
    let chat = ChatId(chat_id.parse().unwrap_or(0));

    let mut is_on = false;
    let mut image_buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(status) = trimmed.strip_prefix("status:") {
            is_on = status == "on";
            continue;
        }
        if let Some(dot_pos) = trimmed.find(". ")
            && let Ok(n) = trimmed[..dot_pos].parse::<usize>()
        {
            let label_part = &trimmed[dot_pos + 2..];
            let is_active = label_part.ends_with('*');
            let clean = label_part.trim_end_matches('*').trim();
            let display = if is_active {
                format!("● {clean}")
            } else {
                format!("○ {clean}")
            };
            image_buttons.push(vec![InlineKeyboardButton::callback(
                display,
                format!("sandbox_image:{n}"),
            )]);
        }
    }

    // Toggle button at the top.
    let toggle_label = if is_on {
        "🟢 Sandbox ON — tap to disable"
    } else {
        "⚫ Sandbox OFF — tap to enable"
    };
    let toggle_action = if is_on {
        "sandbox_toggle:off"
    } else {
        "sandbox_toggle:on"
    };

    let mut buttons = vec![vec![InlineKeyboardButton::callback(
        toggle_label.to_string(),
        toggle_action.to_string(),
    )]];
    buttons.extend(image_buttons);

    let keyboard = InlineKeyboardMarkup::new(buttons);
    let _ = bot
        .send_message(chat, "⚙️ Sandbox settings:")
        .reply_markup(keyboard)
        .await;
}

fn escape_html_simple(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Handle a Telegram callback query (inline keyboard button press).
pub async fn handle_callback_query(
    query: CallbackQuery,
    _bot: &Bot,
    account_id: &str,
    accounts: &AccountStateMap,
) -> anyhow::Result<()> {
    let data = match query.data {
        Some(ref d) => d.as_str(),
        None => return Ok(()),
    };

    // Answer the callback to dismiss the loading spinner.
    let bot = {
        let accts = accounts.read().unwrap();
        accts.get(account_id).map(|s| s.bot.clone())
    };

    // Determine which command this callback is for.
    let cmd_text = if let Some(n_str) = data.strip_prefix("sessions_switch:") {
        Some(format!("sessions {n_str}"))
    } else if let Some(n_str) = data.strip_prefix("model_switch:") {
        Some(format!("model {n_str}"))
    } else if let Some(val) = data.strip_prefix("sandbox_toggle:") {
        Some(format!("sandbox {val}"))
    } else if let Some(n_str) = data.strip_prefix("sandbox_image:") {
        Some(format!("sandbox image {n_str}"))
    } else if data.starts_with("model_provider:") {
        // Handled separately below — no simple cmd_text.
        None
    } else {
        if let Some(ref bot) = bot {
            let _ = bot.answer_callback_query(&query.id).await;
        }
        return Ok(());
    };

    let chat_id = query
        .message
        .as_ref()
        .map(|m| m.chat().id.0.to_string())
        .unwrap_or_default();

    if chat_id.is_empty() {
        return Ok(());
    }

    let (event_sink, outbound) = {
        let accts = accounts.read().unwrap();
        let state = match accts.get(account_id) {
            Some(s) => s,
            None => return Ok(()),
        };
        (state.event_sink.clone(), Arc::clone(&state.outbound))
    };

    let reply_target = moltis_channels::ChannelReplyTarget {
        channel_type: "telegram".into(),
        account_id: account_id.to_string(),
        chat_id: chat_id.clone(),
    };

    // Provider selection → fetch models for that provider and show a new keyboard.
    if let Some(provider_name) = data.strip_prefix("model_provider:") {
        if let Some(ref bot) = bot {
            let _ = bot.answer_callback_query(&query.id).await;
        }
        if let Some(ref sink) = event_sink {
            let cmd = format!("model provider:{provider_name}");
            match sink.dispatch_command(&cmd, reply_target).await {
                Ok(text) => {
                    let b = bot.as_ref().unwrap();
                    send_model_keyboard(b, &chat_id, &text).await;
                },
                Err(e) => {
                    if let Err(err) = outbound
                        .send_text(account_id, &chat_id, &format!("Error: {e}"))
                        .await
                    {
                        warn!(account_id, "failed to send callback response: {err}");
                    }
                },
            }
        }
        return Ok(());
    }

    let cmd_text = cmd_text.unwrap();

    if let Some(ref sink) = event_sink {
        let response = match sink.dispatch_command(&cmd_text, reply_target).await {
            Ok(msg) => msg,
            Err(e) => format!("Error: {e}"),
        };

        // Answer callback query with the response text (shows as toast).
        if let Some(ref bot) = bot {
            let _ = bot.answer_callback_query(&query.id).text(&response).await;
        }

        // Also send as a regular message for visibility.
        if let Err(e) = outbound.send_text(account_id, &chat_id, &response).await {
            warn!(account_id, "failed to send callback response: {e}");
        }
    } else if let Some(ref bot) = bot {
        let _ = bot.answer_callback_query(&query.id).await;
    }

    Ok(())
}

/// Extract text content from a message.
fn extract_text(msg: &Message) -> Option<String> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Text(t) => Some(t.text.clone()),
            MediaKind::Photo(p) => p.caption.clone(),
            MediaKind::Document(d) => d.caption.clone(),
            MediaKind::Audio(a) => a.caption.clone(),
            MediaKind::Voice(v) => v.caption.clone(),
            MediaKind::Video(vid) => vid.caption.clone(),
            MediaKind::Animation(a) => a.caption.clone(),
            _ => None,
        },
        _ => None,
    }
}

/// Check if the message contains media (photo, document, etc.).
fn has_media(msg: &Message) -> bool {
    match &msg.kind {
        MessageKind::Common(common) => !matches!(common.media_kind, MediaKind::Text(_)),
        _ => false,
    }
}

/// Extract a file ID reference from a message for later download.
#[allow(dead_code)]
fn extract_media_url(msg: &Message) -> Option<String> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Photo(p) => p.photo.last().map(|ps| format!("tg://file/{}", ps.file.id)),
            MediaKind::Document(d) => Some(format!("tg://file/{}", d.document.file.id)),
            MediaKind::Audio(a) => Some(format!("tg://file/{}", a.audio.file.id)),
            MediaKind::Voice(v) => Some(format!("tg://file/{}", v.voice.file.id)),
            MediaKind::Sticker(s) => Some(format!("tg://file/{}", s.sticker.file.id)),
            _ => None,
        },
        _ => None,
    }
}

/// Classify the chat type.
fn classify_chat(msg: &Message) -> (ChatType, Option<String>) {
    match msg.chat.kind {
        teloxide::types::ChatKind::Private(_) => (ChatType::Dm, None),
        teloxide::types::ChatKind::Public(ref p) => {
            let group_id = msg.chat.id.0.to_string();
            match p.kind {
                teloxide::types::PublicChatKind::Channel(_) => (ChatType::Channel, Some(group_id)),
                _ => (ChatType::Group, Some(group_id)),
            }
        },
    }
}

/// Check if the bot was @mentioned in the message.
fn check_bot_mentioned(msg: &Message, bot_username: Option<&str>) -> bool {
    let text = extract_text(msg).unwrap_or_default();
    if let Some(username) = bot_username {
        text.contains(&format!("@{username}"))
    } else {
        false
    }
}

/// Build a session key.
#[allow(dead_code)]
fn build_session_key(
    account_id: &str,
    chat_type: &ChatType,
    peer_id: &str,
    group_id: Option<&str>,
) -> String {
    match chat_type {
        ChatType::Dm => format!("telegram:{account_id}:dm:{peer_id}"),
        ChatType::Group | ChatType::Channel => {
            let gid = group_id.unwrap_or("unknown");
            format!("telegram:{account_id}:group:{gid}")
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_key_dm() {
        let key = build_session_key("bot1", &ChatType::Dm, "user123", None);
        assert_eq!(key, "telegram:bot1:dm:user123");
    }

    #[test]
    fn session_key_group() {
        let key = build_session_key("bot1", &ChatType::Group, "user123", Some("-100999"));
        assert_eq!(key, "telegram:bot1:group:-100999");
    }

    /// Security: the OTP challenge message sent to the Telegram user must
    /// NEVER contain the verification code.  The code should only be visible
    /// to the admin in the web UI.  If this test fails, unauthenticated users
    /// can self-approve without admin involvement.
    #[test]
    fn security_otp_challenge_message_does_not_contain_code() {
        let msg = OTP_CHALLENGE_MSG;

        // Must not contain any 6-digit numeric sequences (OTP codes are 6 digits).
        let has_six_digits = msg
            .as_bytes()
            .windows(6)
            .any(|w| w.iter().all(|b| b.is_ascii_digit()));
        assert!(
            !has_six_digits,
            "OTP challenge message must not contain a 6-digit code: {msg}"
        );

        // Must not contain format placeholders that could interpolate a code.
        assert!(
            !msg.contains("{code}") && !msg.contains("{0}"),
            "OTP challenge message must not contain format placeholders: {msg}"
        );

        // Must contain instructions pointing to the web UI.
        assert!(
            msg.contains("Channels") && msg.contains("Senders"),
            "OTP challenge message must tell the user where to find the code"
        );
    }
}
