use {
    serenity::{
        all::{Context, EventHandler, GatewayIntents, Message, Ready},
        async_trait,
    },
    tracing::{debug, info, warn},
};

use moltis_channels::{
    ChannelEvent, ChannelType,
    gating::{DmPolicy, GroupPolicy, MentionMode, is_allowed},
    message_log::MessageLogEntry,
    plugin::{ChannelMessageKind, ChannelMessageMeta, ChannelReplyTarget},
};

use crate::state::AccountStateMap;

/// Required gateway intents for the Discord bot.
pub fn required_intents() -> GatewayIntents {
    GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILDS
}

/// Serenity event handler for a Discord bot account.
pub struct Handler {
    pub account_id: String,
    pub accounts: AccountStateMap,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Strip the bot mention (e.g. `<@123456789>`) from the beginning of a message.
pub fn strip_bot_mention(text: &str, bot_id: u64) -> String {
    let mention = format!("<@{bot_id}>");
    let mention_nick = format!("<@!{bot_id}>");
    let stripped = text
        .trim()
        .strip_prefix(&mention)
        .or_else(|| text.trim().strip_prefix(&mention_nick))
        .unwrap_or(text);
    stripped.trim().to_string()
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, _ctx: Context, msg: Message) {
        // Ignore messages from bots (including ourselves).
        if msg.author.bot {
            return;
        }

        let (config, event_sink, message_log, bot_user_id) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let Some(state) = accounts.get(&self.account_id) else {
                warn!(account_id = %self.account_id, "Discord handler: unknown account");
                return;
            };
            (
                state.config.clone(),
                state.event_sink.clone(),
                state.message_log.clone(),
                state.bot_user_id,
            )
        };

        let is_guild = msg.guild_id.is_some();
        let peer_id = msg.author.id.to_string();
        let username = Some(msg.author.name.clone());
        let sender_name = msg
            .author
            .global_name
            .clone()
            .or_else(|| Some(msg.author.name.clone()));
        let chat_id = msg.channel_id.to_string();

        // Check if the bot is mentioned in a guild message.
        let bot_mentioned = if let Some(bot_id) = bot_user_id {
            msg.mentions.iter().any(|u| u.id == bot_id)
        } else {
            false
        };

        // Extract and clean message text.
        let text = if let Some(bot_id) = bot_user_id
            && bot_mentioned
        {
            strip_bot_mention(&msg.content, bot_id.get())
        } else {
            msg.content.clone()
        };

        if text.is_empty() {
            return;
        }

        // Apply guild allowlist.
        if is_guild
            && let Some(guild_id) = msg.guild_id
            && !config.guild_allowlist.is_empty()
            && !is_allowed(&guild_id.to_string(), &config.guild_allowlist)
        {
            debug!(
                account_id = %self.account_id,
                guild_id = %guild_id,
                "Discord message from non-allowlisted guild"
            );
            return;
        }

        // Apply DM/group policy and mention mode.
        let policy_allowed = if is_guild {
            match config.group_policy {
                GroupPolicy::Open => true,
                GroupPolicy::Allowlist => is_allowed(&chat_id, &config.guild_allowlist),
                GroupPolicy::Disabled => false,
            }
        } else {
            match config.dm_policy {
                DmPolicy::Open => true,
                DmPolicy::Allowlist => is_allowed(&peer_id, &config.allowlist),
                DmPolicy::Disabled => false,
            }
        };
        let mention_allowed = if is_guild {
            match config.mention_mode {
                MentionMode::Always => true,
                MentionMode::Mention => bot_mentioned,
                MentionMode::None => false,
            }
        } else {
            true // DMs always pass mention check
        };
        let access_granted = policy_allowed && mention_allowed;

        // Log the message.
        if let Some(log) = message_log {
            let _ = log
                .log(MessageLogEntry {
                    id: 0,
                    account_id: self.account_id.clone(),
                    channel_type: "discord".into(),
                    peer_id: peer_id.clone(),
                    username: username.clone(),
                    sender_name: sender_name.clone(),
                    chat_id: chat_id.clone(),
                    chat_type: if is_guild {
                        "group".into()
                    } else {
                        "private".into()
                    },
                    body: text.clone(),
                    access_granted,
                    created_at: unix_now(),
                })
                .await;
        }

        // Emit inbound message event.
        if let Some(sink) = event_sink.as_ref() {
            sink.emit(ChannelEvent::InboundMessage {
                channel_type: ChannelType::Discord,
                account_id: self.account_id.clone(),
                peer_id: peer_id.clone(),
                username: username.clone(),
                sender_name: sender_name.clone(),
                message_count: None,
                access_granted,
            })
            .await;
        }

        if !access_granted {
            return;
        }

        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::Discord,
            account_id: self.account_id.clone(),
            chat_id: chat_id.clone(),
            message_id: Some(msg.id.to_string()),
        };

        let Some(sink) = event_sink else {
            warn!(
                account_id = %self.account_id,
                "Discord inbound message ignored: no channel event sink"
            );
            return;
        };

        // Handle slash commands.
        if let Some(command) = text.strip_prefix('/') {
            match sink
                .dispatch_command(command.trim(), reply_to.clone())
                .await
            {
                Ok(response) => {
                    // Read the http handle from state to send the response.
                    let http = {
                        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
                        accounts.get(&self.account_id).and_then(|s| s.http.clone())
                    };
                    if let Some(http) = http
                        && let Err(e) = send_discord_message(&http, msg.channel_id, &response).await
                    {
                        warn!(
                            account_id = %self.account_id,
                            chat_id, "failed to send Discord command response: {e}"
                        );
                    }
                },
                Err(e) => {
                    let error_msg = format!("Command failed: {e}");
                    let http = {
                        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
                        accounts.get(&self.account_id).and_then(|s| s.http.clone())
                    };
                    if let Some(http) = http
                        && let Err(send_err) =
                            send_discord_message(&http, msg.channel_id, &error_msg).await
                    {
                        warn!(
                            account_id = %self.account_id,
                            chat_id,
                            "failed to send Discord command error: {send_err}"
                        );
                    }
                },
            }
            return;
        }

        // Dispatch to chat.
        sink.dispatch_to_chat(&text, reply_to, ChannelMessageMeta {
            channel_type: ChannelType::Discord,
            sender_name,
            username,
            message_kind: Some(ChannelMessageKind::Text),
            model: config.model.clone(),
            audio_filename: None,
        })
        .await;
    }

    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!(
            account_id = %self.account_id,
            bot_user = %ready.user.name,
            "Discord bot connected as {}",
            ready.user.name,
        );
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get_mut(&self.account_id) {
            state.bot_user_id = Some(ready.user.id);
        }
    }
}

/// Send a text message to a Discord channel, chunking at the 2000-character limit.
pub async fn send_discord_message(
    http: &serenity::http::Http,
    channel_id: serenity::all::ChannelId,
    text: &str,
) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }

    for chunk in chunk_message(text, 2000) {
        channel_id
            .say(http, chunk)
            .await
            .map_err(|e| format!("Discord send: {e}"))?;
    }
    Ok(())
}

/// Split a message into chunks of at most `max_len` characters.
fn chunk_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining);
            break;
        }
        // Try to split at a newline within the limit.
        let split_at = remaining[..max_len]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(max_len);
        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk);
        remaining = rest;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_mention_at_start() {
        assert_eq!(strip_bot_mention("<@123> hello world", 123), "hello world");
        assert_eq!(strip_bot_mention("<@!123> hello world", 123), "hello world");
    }

    #[test]
    fn strip_mention_no_match() {
        assert_eq!(
            strip_bot_mention("hello <@123> world", 123),
            "hello <@123> world"
        );
        assert_eq!(strip_bot_mention("hello world", 123), "hello world");
    }

    #[test]
    fn strip_mention_different_bot() {
        assert_eq!(strip_bot_mention("<@999> hello", 123), "<@999> hello");
    }

    #[test]
    fn chunk_short_message() {
        let chunks = chunk_message("hello", 2000);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn chunk_long_message() {
        let text = "a".repeat(4500);
        let chunks = chunk_message(&text, 2000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 2000);
        assert_eq!(chunks[1].len(), 2000);
        assert_eq!(chunks[2].len(), 500);
    }

    #[test]
    fn chunk_splits_at_newline() {
        let mut text = String::new();
        text.push_str(&"a".repeat(1500));
        text.push('\n');
        text.push_str(&"b".repeat(1000));
        let chunks = chunk_message(&text, 2000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 1501); // 1500 + newline
        assert_eq!(chunks[1].len(), 1000);
    }
}
