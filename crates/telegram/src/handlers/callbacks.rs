use {
    teloxide::{
        prelude::*,
        types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
    },
    tracing::warn,
};

use std::sync::Arc;

use moltis_channels::{ChannelOutbound, ChannelReplyTarget, ChannelType};

use crate::state::AccountStateMap;

use super::parse_chat_target_lossy;

pub(super) async fn send_sessions_keyboard(bot: &Bot, to: &str, sessions_text: &str) {
    let (chat, thread_id) = parse_chat_target_lossy(to);

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for line in sessions_text.lines() {
        let trimmed = line.trim();
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
        let mut req = bot.send_message(chat, sessions_text);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.await;
        return;
    }

    let keyboard = InlineKeyboardMarkup::new(buttons);
    let mut req = bot
        .send_message(chat, "Select a session:")
        .reply_markup(keyboard);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

pub(super) async fn send_agent_keyboard(bot: &Bot, to: &str, agents_text: &str) {
    let (chat, thread_id) = parse_chat_target_lossy(to);
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for line in agents_text.lines() {
        let trimmed = line.trim();
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
                format!("agent_switch:{n}"),
            )]);
        }
    }

    if buttons.is_empty() {
        let mut req = bot.send_message(chat, agents_text);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.await;
        return;
    }

    let keyboard = InlineKeyboardMarkup::new(buttons);
    let mut req = bot
        .send_message(chat, "Select an agent:")
        .reply_markup(keyboard);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

pub(super) async fn send_context_card(bot: &Bot, to: &str, context_text: &str) {
    let (chat, thread_id) = parse_chat_target_lossy(to);

    let mut fields: Vec<(&str, String)> = Vec::new();
    for line in context_text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("**")
            && let Some(end) = rest.find("**")
        {
            let label = &rest[..end];
            let raw_value = rest[end + 2..].trim();
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

    let plugins_section = if plugins_raw == "none" || plugins_raw.is_empty() {
        "  <i>none</i>".to_string()
    } else {
        plugins_raw
            .split(", ")
            .map(|p| format!("  ▸ {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

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

    let mut req = bot.send_message(chat, html).parse_mode(ParseMode::Html);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

pub(super) async fn send_model_keyboard(bot: &Bot, to: &str, text: &str) {
    let (chat, thread_id) = parse_chat_target_lossy(to);

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
        let mut req = bot.send_message(chat, "No models available.");
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.await;
        return;
    }

    let heading = if is_provider_list {
        "🤖 Select a provider:"
    } else {
        "🤖 Select a model:"
    };

    let keyboard = InlineKeyboardMarkup::new(buttons);
    let mut req = bot.send_message(chat, heading).reply_markup(keyboard);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

pub(super) async fn send_sandbox_keyboard(bot: &Bot, to: &str, text: &str) {
    let (chat, thread_id) = parse_chat_target_lossy(to);

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
    let mut req = bot
        .send_message(chat, "⚙️ Sandbox settings:")
        .reply_markup(keyboard);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

fn escape_html_simple(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[allow(dead_code, unused)]
pub(super) async fn handle_callback_query(
    query: CallbackQuery,
    _bot: &Bot,
    account_id: &str,
    accounts: &AccountStateMap,
) -> crate::Result<()> {
    let data = match query.data {
        Some(ref d) => d.as_str(),
        None => return Ok(()),
    };

    let bot = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).map(|s| s.bot.clone())
    };

    let cmd_text = if let Some(n_str) = data.strip_prefix("sessions_switch:") {
        Some(format!("sessions {n_str}"))
    } else if let Some(n_str) = data.strip_prefix("agent_switch:") {
        Some(format!("agent {n_str}"))
    } else if let Some(n_str) = data.strip_prefix("model_switch:") {
        Some(format!("model {n_str}"))
    } else if let Some(val) = data.strip_prefix("sandbox_toggle:") {
        Some(format!("sandbox {val}"))
    } else if let Some(n_str) = data.strip_prefix("sandbox_image:") {
        Some(format!("sandbox image {n_str}"))
    } else if data.starts_with("model_provider:") {
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
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = match accts.get(account_id) {
            Some(s) => s,
            None => return Ok(()),
        };
        (state.event_sink.clone(), Arc::clone(&state.outbound))
    };

    let callback_thread_id = query
        .message
        .as_ref()
        .and_then(|m| m.regular_message())
        .and_then(|m| m.thread_id)
        .map(|tid| tid.0.0.to_string());
    let sender_id = query.from.id.0.to_string();
    let reply_target = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: account_id.to_string(),
        chat_id: chat_id.clone(),
        message_id: None,
        thread_id: callback_thread_id,
    };
    let outbound_to = reply_target.outbound_to().into_owned();

    if let Some(provider_name) = data.strip_prefix("model_provider:") {
        if let Some(ref bot) = bot {
            let _ = bot.answer_callback_query(&query.id).await;
        }
        if let Some(ref sink) = event_sink {
            let cmd = format!("model provider:{provider_name}");
            match sink
                .dispatch_command(&cmd, reply_target, Some(&sender_id))
                .await
            {
                Ok(text) => {
                    if let Some(ref b) = bot {
                        send_model_keyboard(b, &outbound_to, &text).await;
                    }
                },
                Err(e) => {
                    if let Err(err) = outbound
                        .send_text(account_id, &outbound_to, &format!("Error: {e}"), None)
                        .await
                    {
                        warn!(account_id, "failed to send callback response: {err}");
                    }
                },
            }
        }
        return Ok(());
    }

    let Some(cmd_text) = cmd_text else {
        return Ok(());
    };

    if let Some(ref sink) = event_sink {
        let response = match sink
            .dispatch_command(&cmd_text, reply_target, Some(&sender_id))
            .await
        {
            Ok(msg) => msg,
            Err(e) => format!("Error: {e}"),
        };

        if let Some(ref bot) = bot {
            let _ = bot.answer_callback_query(&query.id).text(&response).await;
        }

        if let Err(e) = outbound
            .send_text(account_id, &outbound_to, &response, None)
            .await
        {
            warn!(account_id, "failed to send callback response: {e}");
        }
    } else if let Some(ref bot) = bot {
        let _ = bot.answer_callback_query(&query.id).await;
    }

    Ok(())
}
