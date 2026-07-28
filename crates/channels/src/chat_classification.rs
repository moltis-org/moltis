use crate::plugin::ChannelType;

pub(crate) fn classify_chat(channel_type: ChannelType, chat_id: &str) -> Option<String> {
    match channel_type {
        ChannelType::Telegram => {
            if chat_id.starts_with("-100") {
                Some("channel_or_supergroup".to_string())
            } else if chat_id.starts_with('-') {
                Some("group".to_string())
            } else {
                Some("private".to_string())
            }
        },
        ChannelType::Signal => Some(
            if chat_id.starts_with("group:") {
                "group"
            } else {
                "direct"
            }
            .to_string(),
        ),
        ChannelType::Slack => Some(
            if chat_id.starts_with('D') {
                "direct"
            } else {
                "channel"
            }
            .to_string(),
        ),
        ChannelType::Whatsapp => Some(
            if chat_id.ends_with("@g.us") {
                "group"
            } else {
                "direct"
            }
            .to_string(),
        ),
        ChannelType::Nostr => Some("dm".to_string()),
        ChannelType::Telephony => Some("call".to_string()),
        ChannelType::MsTeams | ChannelType::Discord | ChannelType::Matrix => None,
    }
}

pub(crate) fn is_shared_chat(channel_type: ChannelType, chat_id: &str) -> bool {
    match channel_type {
        ChannelType::Telegram => chat_id.starts_with('-'),
        ChannelType::Signal => chat_id.starts_with("group:"),
        ChannelType::Slack => !chat_id.starts_with('D'),
        ChannelType::Whatsapp => chat_id.ends_with("@g.us"),
        ChannelType::Nostr | ChannelType::Telephony => false,
        ChannelType::MsTeams | ChannelType::Discord | ChannelType::Matrix => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_and_shared_shapes_are_classified_fail_closed() {
        assert!(!is_shared_chat(ChannelType::Telegram, "123"));
        assert!(is_shared_chat(ChannelType::Telegram, "-123"));
        assert!(!is_shared_chat(ChannelType::Slack, "D123"));
        assert!(is_shared_chat(ChannelType::Slack, "C123"));
        assert!(!is_shared_chat(
            ChannelType::Whatsapp,
            "15551234567@s.whatsapp.net"
        ));
        assert!(is_shared_chat(ChannelType::Whatsapp, "123@g.us"));
        assert!(is_shared_chat(ChannelType::Discord, "123"));
        assert!(is_shared_chat(ChannelType::Matrix, "!room:example.org"));
    }
}
