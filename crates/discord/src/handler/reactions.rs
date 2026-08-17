//! Inbound Discord reaction handling.
//!
//! Split from the main handler so both files stay inside the file-size limit.

use serenity::all::{Context, Reaction, ReactionType};

use {
    moltis_channels::{ChannelEvent, ChannelType},
    moltis_common::types::ChatType,
};

use {super::implementation::Handler, crate::access};

impl Handler {
    /// Surface a reaction change so the gateway can score it as feedback.
    ///
    /// The bot's own acknowledgement reactions are skipped: it marks incoming
    /// messages with 👀/✅/❌, and treating those as user opinions would score
    /// every turn the bot itself handled.
    pub(super) async fn emit_reaction_change(
        &self,
        ctx: &Context,
        reaction: &Reaction,
        added: bool,
    ) {
        let Some(user_id) = reaction.user_id else {
            return;
        };
        if user_id == ctx.cache.current_user().id {
            return;
        }

        let (config, sink) = {
            let accts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            match accts.get(&self.account_id) {
                Some(state) => (state.config.clone(), state.event_sink.clone()),
                None => return,
            }
        };
        let Some(sink) = sink else {
            return;
        };

        let chat_type = if reaction.guild_id.is_some() {
            ChatType::Group
        } else {
            ChatType::Dm
        };
        let guild_id = reaction.guild_id.map(|id| id.to_string());
        let username = if added {
            reaction.user(&ctx.http).await.ok().map(|user| user.name)
        } else {
            None
        };
        let basic_access = !added
            || access::check_access(
                &config,
                &chat_type,
                &user_id.to_string(),
                username.as_deref(),
                guild_id.as_deref(),
                // A reaction is intentional interaction and does not need a fresh
                // mention, while the account and guild allowlists still apply.
                true,
            )
            .is_ok();
        let filter_active =
            !config.channel_name_patterns.is_empty() || !config.category_allowlist.is_empty();
        let channel_allowed = if !added || !filter_active {
            true
        } else {
            reaction
                .guild_id
                .and_then(|guild_id| {
                    ctx.cache.guild(guild_id).and_then(|guild| {
                        guild.channels.get(&reaction.channel_id).map(|channel| {
                            access::channel_matches_filter(
                                &config,
                                Some(&channel.name),
                                channel.parent_id.map(|id| id.to_string()).as_deref(),
                            )
                        })
                    })
                })
                .unwrap_or(false)
        };
        if !basic_access || !channel_allowed {
            return;
        }

        // Custom guild emoji have no unicode form; their name is what a
        // feedback vocabulary can match against.
        let emoji = match &reaction.emoji {
            ReactionType::Unicode(raw) => raw.clone(),
            ReactionType::Custom { name, .. } => name.clone().unwrap_or_default(),
            _ => return,
        };
        if emoji.is_empty() {
            return;
        }

        sink.emit(ChannelEvent::ReactionChange {
            channel_type: ChannelType::Discord,
            account_id: self.account_id.clone(),
            chat_id: reaction.channel_id.to_string(),
            message_id: reaction.message_id.to_string(),
            user_id: user_id.to_string(),
            emoji,
            added,
        })
        .await;
    }
}
