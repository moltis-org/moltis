use {
    moltis_channels::{ChannelReplyTarget, ChannelType},
    serenity::all::{ChannelId, GuildId, MessageId},
};

/// Whether a gateway message arrived in the bot's own one-to-one DM.
///
/// Guild messages always carry a `guild_id`, and bot users cannot be members
/// of group DMs, so a message without one is a direct conversation between
/// its author and the bot.
fn is_bot_direct_message(guild_id: Option<GuildId>) -> bool {
    guild_id.is_none()
}

/// Reply target for an inbound gateway message, forwarding whether the
/// conversation is a DM so the gateway can classify it as a direct chat.
pub(super) fn message_reply_target(
    account_id: &str,
    channel_id: ChannelId,
    message_id: MessageId,
    guild_id: Option<GuildId>,
) -> ChannelReplyTarget {
    ChannelReplyTarget {
        direct_chat: is_bot_direct_message(guild_id),
        ack_message_id: None,
        channel_type: ChannelType::Discord,
        account_id: account_id.to_string(),
        chat_id: channel_id.to_string(),
        message_id: Some(message_id.to_string()),
        thread_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dm_message_reply_target_forwards_direct_chat() {
        let target = message_reply_target("bot", ChannelId::new(42), MessageId::new(7), None);
        assert!(target.direct_chat);
        assert_eq!(target.channel_type, ChannelType::Discord);
        assert_eq!(target.chat_id, "42");
        assert_eq!(target.message_id.as_deref(), Some("7"));
        assert!(!target.is_shared_chat());
        assert_eq!(target.classify_chat().as_deref(), Some("direct"));
    }

    #[test]
    fn guild_message_reply_target_stays_shared() {
        let target = message_reply_target(
            "bot",
            ChannelId::new(42),
            MessageId::new(7),
            Some(GuildId::new(9)),
        );
        assert!(!target.direct_chat);
        assert!(target.is_shared_chat());
        assert!(target.classify_chat().is_none());
    }
}
