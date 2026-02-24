//! Discord event handler for serenity.
//!
//! Implements the EventHandler trait to receive and process Discord events.

use std::sync::Arc;

use {
    serenity::{
        all::{Context, EventHandler, GatewayIntents, GuildId, Message, Ready},
        async_trait,
    },
    tracing::{debug, info, warn},
};

use moltis_channels::{
    ChannelEventSink, ChannelMessageMeta, ChannelReplyTarget, ChannelType,
    gating::{DmPolicy, GroupPolicy, MentionMode, is_allowed},
    message_log::{MessageLog, MessageLogEntry},
    plugin::ChannelEvent,
};

use crate::{config::DiscordAccountConfig, markdown::strip_mentions, state::AccountStateMap};

/// Handler for Discord gateway events.
pub struct DiscordHandler {
    pub account_id: String,
    pub config: DiscordAccountConfig,
    pub accounts: AccountStateMap,
    pub message_log: Option<Arc<dyn MessageLog>>,
    pub event_sink: Option<Arc<dyn ChannelEventSink>>,
}

impl DiscordHandler {
    /// Required gateway intents for the bot.
    pub fn intents() -> GatewayIntents {
        GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
    }

    fn check_access(
        &self,
        user_id: &str,
        guild_id: Option<&str>,
        channel_id: &str,
        is_dm: bool,
        member_roles: &[String],
    ) -> bool {
        if is_dm {
            match self.config.dm_policy {
                DmPolicy::Open => true,
                DmPolicy::Allowlist => is_allowed(user_id, &self.config.user_allowlist),
                DmPolicy::Disabled => false,
            }
        } else {
            // Check guild policy first
            let guild_allowed = match self.config.guild_policy {
                GroupPolicy::Open => true,
                GroupPolicy::Allowlist => {
                    guild_id.is_some_and(|gid| is_allowed(gid, &self.config.guild_allowlist))
                },
                GroupPolicy::Disabled => false,
            };

            if !guild_allowed {
                return false;
            }

            // Check channel allowlist (empty = all channels allowed)
            if !self.config.channel_allowlist.is_empty()
                && !is_allowed(channel_id, &self.config.channel_allowlist)
            {
                return false;
            }

            // Check role allowlist (empty = all roles allowed)
            if !self.config.role_allowlist.is_empty() {
                return member_roles
                    .iter()
                    .any(|role| is_allowed(role, &self.config.role_allowlist));
            }

            true
        }
    }
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(
            account_id = %self.account_id,
            bot_name = %ready.user.name,
            guilds = ready.guilds.len(),
            "discord bot ready"
        );

        // Store bot user ID
        {
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = accounts.get_mut(&self.account_id) {
                state.bot_user_id = Some(ready.user.id.get());
                state.http = ctx.http.clone();
            }
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Skip bot messages to prevent loops
        if msg.author.bot {
            return;
        }

        let channel_id = msg.channel_id.to_string();
        let user_id = msg.author.id.to_string();
        let guild_id = msg.guild_id.map(|g| g.to_string());
        let text = &msg.content;

        // Determine if DM or guild message
        let is_dm = msg.guild_id.is_none();

        // Get bot user ID from cache
        let bot_user_id = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get(&self.account_id).and_then(|s| s.bot_user_id)
        };

        // Check if bot is mentioned
        let is_mention =
            bot_user_id.is_some_and(|bot_id| msg.mentions.iter().any(|u| u.id.get() == bot_id));

        // Get member roles for access control
        let member_roles: Vec<String> = if let Some(ref member) = msg.member {
            member.roles.iter().map(|r| r.to_string()).collect()
        } else {
            Vec::new()
        };

        // Access control
        let access_granted = self.check_access(
            &user_id,
            guild_id.as_deref(),
            &channel_id,
            is_dm,
            &member_roles,
        );

        // Log message
        if let Some(log) = &self.message_log {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let entry = MessageLogEntry {
                id: 0,
                account_id: self.account_id.clone(),
                channel_type: ChannelType::Discord.to_string(),
                peer_id: user_id.clone(),
                username: Some(msg.author.name.clone()),
                sender_name: msg.author.global_name.clone(),
                chat_id: channel_id.clone(),
                chat_type: if is_dm {
                    "dm"
                } else {
                    "guild"
                }
                .into(),
                body: text.clone(),
                access_granted,
                created_at: now,
            };
            let _ = log.log(entry).await;
        }

        // Emit channel event
        if let Some(sink) = &self.event_sink {
            sink.emit(ChannelEvent::InboundMessage {
                channel_type: ChannelType::Discord,
                account_id: self.account_id.clone(),
                peer_id: user_id.clone(),
                username: Some(msg.author.name.clone()),
                sender_name: msg.author.global_name.clone(),
                message_count: None,
                access_granted,
            })
            .await;
        }

        if !access_granted {
            return;
        }

        // Check activation mode
        let should_respond = match self.config.mention_mode {
            MentionMode::Mention => is_dm || is_mention,
            MentionMode::Always => true,
            MentionMode::None => is_dm,
        };

        if !should_respond {
            return;
        }

        // Strip mentions and clean up text
        let clean_text = strip_mentions(text, bot_user_id);

        if clean_text.is_empty() {
            return;
        }

        // Handle moltis commands
        if clean_text.starts_with('/') {
            let command = clean_text
                .trim_start_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or("");

            if matches!(
                command,
                "new" | "clear" | "compact" | "context" | "model" | "sessions"
            ) {
                if let Some(sink) = &self.event_sink {
                    let reply_to = ChannelReplyTarget {
                        channel_type: ChannelType::Discord,
                        account_id: self.account_id.clone(),
                        chat_id: channel_id.clone(),
                        message_id: None,
                    };

                    match sink.dispatch_command(command, reply_to).await {
                        Ok(response) => {
                            if let Err(e) = msg.reply(&ctx.http, &response).await {
                                warn!(error = %e, "failed to send command response");
                            }
                        },
                        Err(e) => {
                            warn!(error = %e, "command dispatch failed");
                        },
                    }
                }
                return;
            }
        }

        // Store message ID for replies
        {
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = accounts.get_mut(&self.account_id) {
                state
                    .pending_replies
                    .insert(channel_id.clone(), msg.id.get());
            }
        }

        // Build reply target
        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::Discord,
            account_id: self.account_id.clone(),
            chat_id: channel_id,
            message_id: None,
        };

        let meta = ChannelMessageMeta {
            channel_type: ChannelType::Discord,
            sender_name: msg.author.global_name.clone(),
            username: Some(msg.author.name.clone()),
            message_kind: None,
            model: self.config.model.clone(),
            audio_filename: None,
        };

        // Dispatch to chat
        if let Some(sink) = &self.event_sink {
            sink.dispatch_to_chat(&clean_text, reply_to, meta).await;
        }
    }

    async fn cache_ready(&self, _ctx: Context, guilds: Vec<GuildId>) {
        debug!(
            account_id = %self.account_id,
            guild_count = guilds.len(),
            "discord cache ready"
        );
    }
}
