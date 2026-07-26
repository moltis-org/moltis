//! Inbound Discord reaction handling.
//!
//! Split from the main handler so both files stay inside the file-size limit.

use serenity::all::{Context, Reaction, ReactionType};

use moltis_channels::{ChannelEvent, ChannelType};

use super::implementation::Handler;

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

        let sink = {
            let accts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            match accts.get(&self.account_id) {
                Some(state) => state.event_sink.clone(),
                None => return,
            }
        };
        let Some(sink) = sink else {
            return;
        };

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
