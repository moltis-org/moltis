//! NIP-29 relay-based group chat — support for [Buzz](https://github.com/block/buzz).
//!
//! Block's Buzz is a Nostr relay where AI agents and humans collaborate in
//! group "channels". Its primary API is
//! [NIP-29](https://github.com/nostr-protocol/nips/blob/master/29.md) group
//! chat: `kind:9` chat messages scoped to a group via an `h` tag, delivered
//! over a relay connection authenticated with
//! [NIP-42](https://github.com/nostr-protocol/nips/blob/master/42.md).
//!
//! This module lets the Moltis Nostr channel join those groups, decide when to
//! respond, and post threaded replies. It speaks plain NIP-29 — the same wire
//! protocol any NIP-29 client uses — so it works against Buzz relays and
//! generic NIP-29 relays alike. Unlike DMs (`gift_wrap`), group messages are
//! **not** encrypted; the relay enforces membership and authorization.

use {
    moltis_channels::gating::MentionMode,
    nostr_sdk::prelude::{
        Alphabet, Client, Event, EventBuilder, EventId, Kind, PublicKey, SingleLetterTag, Tag,
        TagKind,
    },
};

use crate::error::Error;

/// Standard NIP-29 group chat message kind (`kind:9`).
pub const GROUP_CHAT_KIND: u16 = 9;

/// Buzz's stream message kind (`kind:40002`, `KIND_STREAM_MESSAGE_V2`).
///
/// Buzz defines both `kind:9` and this v2 kind, and posts channel messages as
/// v2. Per its architecture notes, "clients filtering by kind 9 alone would not
/// receive kind 40002 messages, and vice versa" — so a bot that only speaks
/// `kind:9` is invisible in a Buzz channel, and vice versa. We therefore read
/// both and answer in whichever dialect we were addressed in.
pub const BUZZ_STREAM_MESSAGE_V2_KIND: u16 = 40002;

/// The standard NIP-29 group chat message [`Kind`] (`kind:9`).
#[must_use]
pub fn group_chat_kind() -> Kind {
    Kind::from_u16(GROUP_CHAT_KIND)
}

/// Buzz's stream message [`Kind`] (`kind:40002`).
#[must_use]
pub fn buzz_stream_message_kind() -> Kind {
    Kind::from_u16(BUZZ_STREAM_MESSAGE_V2_KIND)
}

/// Every message kind treated as group chat, for subscription filters.
#[must_use]
pub fn group_message_kinds() -> [Kind; 2] {
    [group_chat_kind(), buzz_stream_message_kind()]
}

/// Whether `kind` is one this module handles as a group chat message.
#[must_use]
pub fn is_group_message_kind(kind: Kind) -> bool {
    group_message_kinds().contains(&kind)
}

/// Which dialect to use for group messages that are not replies.
///
/// Replies mirror the kind of the message they answer, so this only decides
/// cold-start sends into a group we have not heard from yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupMessageKind {
    /// Standard NIP-29 `kind:9` — the interoperable default.
    #[default]
    Nip29,
    /// Buzz `kind:40002` — set this when the relay is a Buzz workspace.
    BuzzV2,
}

impl From<GroupMessageKind> for Kind {
    fn from(value: GroupMessageKind) -> Self {
        match value {
            GroupMessageKind::Nip29 => group_chat_kind(),
            GroupMessageKind::BuzzV2 => buzz_stream_message_kind(),
        }
    }
}

/// The single-letter `h` tag NIP-29 uses to scope an event to a group.
#[must_use]
pub fn h_tag() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::H)
}

/// Extract the NIP-29 group id (the `h` tag value) from an event.
#[must_use]
pub fn extract_group_id(event: &Event) -> Option<String> {
    event
        .tags
        .find(TagKind::h())
        .and_then(|t| t.content())
        .map(str::to_owned)
}

/// Whether the bot's pubkey is tagged (`p` tag) in the event — a NIP-29 mention.
#[must_use]
pub fn is_bot_mentioned(event: &Event, bot: &PublicKey) -> bool {
    event.tags.public_keys().any(|pk| pk == bot)
}

/// Decide whether the bot should respond to a group message under the
/// configured [`MentionMode`].
#[must_use]
pub fn should_respond(mode: &MentionMode, event: &Event, bot: &PublicKey) -> bool {
    match mode {
        MentionMode::Always => true,
        MentionMode::None => false,
        MentionMode::Mention => is_bot_mentioned(event, bot),
    }
}

/// Reason a group message was rejected by access control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupDenied {
    /// No groups are configured, so group chat is off for this account.
    Disabled,
    /// The event's group is not one this account joined.
    NotJoined,
}

impl std::fmt::Display for GroupDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "no groups configured"),
            Self::NotJoined => write!(f, "group not joined"),
        }
    }
}

/// Check that an inbound group message belongs to a group this account joined.
///
/// The configured `joined` list is authoritative: it is the only set of groups
/// we ever subscribe to, so anything else must be rejected. This is a security
/// boundary, not a convenience filter — a relay is untrusted and can push any
/// event it likes down the socket, including a `kind:9` carrying an arbitrary
/// `h` tag. The `h` tag is an unauthenticated label (the signature proves who
/// wrote the event, not which group it belongs to), so without this check a
/// hostile or buggy relay could inject text straight into the agent.
///
/// An empty `joined` list therefore denies everything rather than allowing
/// everything — the opposite of the `moltis_channels::gating::is_allowed`
/// convention used for optional allowlists, because here the list defines
/// membership itself rather than filtering within it.
pub fn check_group_access(group_id: &str, joined: &[String]) -> Result<(), GroupDenied> {
    if joined.is_empty() {
        return Err(GroupDenied::Disabled);
    }
    if joined.iter().any(|g| g == group_id) {
        Ok(())
    } else {
        Err(GroupDenied::NotJoined)
    }
}

/// Build the tags for an outbound NIP-29 group message: the required `h` tag
/// scoping it to the group, plus an optional NIP-10 reply `e` tag and an author
/// `p` tag mentioning the person being replied to.
#[must_use]
pub fn build_group_message_tags(
    group_id: &str,
    reply_to: Option<EventId>,
    mention: Option<PublicKey>,
) -> Vec<Tag> {
    let mut tags = vec![Tag::custom(TagKind::h(), [group_id.to_string()])];
    if let Some(event_id) = reply_to {
        tags.push(Tag::event(event_id));
    }
    if let Some(pubkey) = mention {
        tags.push(Tag::public_key(pubkey));
    }
    tags
}

/// Publish a plaintext group chat message to the relay.
///
/// `kind` selects the dialect — `kind:9` for standard NIP-29 or `kind:40002`
/// for Buzz. The event is signed by the client's configured signer (the bot
/// keys) and carries the `h` tag routing it to `group_id`. When `reply_to` is
/// set, a NIP-10 `e` tag threads the reply to the originating message, and
/// `mention` `p`-tags its author so they are notified.
pub async fn send_group_message(
    client: &Client,
    kind: Kind,
    group_id: &str,
    text: &str,
    reply_to: Option<EventId>,
    mention: Option<PublicKey>,
) -> Result<(), Error> {
    let tags = build_group_message_tags(group_id, reply_to, mention);
    let builder = EventBuilder::new(kind, text).tags(tags);
    client.send_event_builder(builder).await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use nostr_sdk::prelude::{EventBuilder, Keys};

    use super::*;

    /// Build a signed `kind:9` group event with the given `h` tag and optional
    /// `p` mention, for exercising the pure extraction/gating helpers.
    fn group_event(group_id: &str, mention: Option<PublicKey>) -> Event {
        let keys = Keys::generate();
        let tags = build_group_message_tags(group_id, None, mention);
        EventBuilder::new(group_chat_kind(), "hello")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign group event")
    }

    #[test]
    fn group_chat_kind_is_nine() {
        assert_eq!(group_chat_kind().as_u16(), 9);
    }

    #[test]
    fn buzz_stream_message_kind_is_40002() {
        assert_eq!(buzz_stream_message_kind().as_u16(), 40_002);
    }

    /// Buzz clients filtering kind:40002 never see kind:9 and vice versa, so
    /// both must be accepted — anything else is not group chat.
    #[test]
    fn both_dialects_are_group_messages() {
        assert!(is_group_message_kind(group_chat_kind()));
        assert!(is_group_message_kind(buzz_stream_message_kind()));
        assert!(!is_group_message_kind(Kind::from_u16(1)));
        assert!(!is_group_message_kind(Kind::from_u16(40_003)));
        assert_eq!(group_message_kinds().len(), 2);
    }

    #[test]
    fn message_kind_config_maps_to_wire_kind() {
        assert_eq!(Kind::from(GroupMessageKind::Nip29), group_chat_kind());
        assert_eq!(
            Kind::from(GroupMessageKind::BuzzV2),
            buzz_stream_message_kind()
        );
        // Default stays on the interoperable standard kind.
        assert_eq!(GroupMessageKind::default(), GroupMessageKind::Nip29);
    }

    #[test]
    fn message_kind_serde_round_trip() {
        for value in [GroupMessageKind::Nip29, GroupMessageKind::BuzzV2] {
            let json = serde_json::to_value(value).unwrap_or_default();
            let parsed: GroupMessageKind = serde_json::from_value(json).unwrap_or_default();
            assert_eq!(parsed, value);
        }
        let from_str: GroupMessageKind =
            serde_json::from_value(serde_json::json!("buzz_v2")).unwrap_or_default();
        assert_eq!(from_str, GroupMessageKind::BuzzV2);
    }

    #[test]
    fn extracts_group_id_from_h_tag() {
        let event = group_event("buzz-general", None);
        assert_eq!(extract_group_id(&event).as_deref(), Some("buzz-general"));
    }

    #[test]
    fn no_group_id_when_h_tag_absent() {
        let keys = Keys::generate();
        let event = EventBuilder::new(group_chat_kind(), "hi")
            .sign_with_keys(&keys)
            .expect("sign");
        assert_eq!(extract_group_id(&event), None);
    }

    #[test]
    fn detects_mention_via_p_tag() {
        let bot = Keys::generate().public_key();
        let mentioned = group_event("g1", Some(bot));
        let not_mentioned = group_event("g1", None);
        assert!(is_bot_mentioned(&mentioned, &bot));
        assert!(!is_bot_mentioned(&not_mentioned, &bot));
    }

    #[test]
    fn should_respond_honours_mention_mode() {
        let bot = Keys::generate().public_key();
        let mentioned = group_event("g1", Some(bot));
        let plain = group_event("g1", None);

        assert!(should_respond(&MentionMode::Always, &plain, &bot));
        assert!(!should_respond(&MentionMode::None, &mentioned, &bot));
        assert!(should_respond(&MentionMode::Mention, &mentioned, &bot));
        assert!(!should_respond(&MentionMode::Mention, &plain, &bot));
    }

    #[test]
    fn group_access_allows_joined_group() {
        let joined = vec!["buzz-general".to_string(), "buzz-dev".to_string()];
        assert!(check_group_access("buzz-general", &joined).is_ok());
        assert!(check_group_access("buzz-dev", &joined).is_ok());
    }

    /// An empty join list denies everything — group chat is simply off.
    #[test]
    fn group_access_empty_join_list_denies_all() {
        assert_eq!(
            check_group_access("anything", &[]),
            Err(GroupDenied::Disabled)
        );
    }

    /// A relay can push any event down the socket, so a `kind:9` for a group
    /// we never joined must be rejected rather than fed to the agent.
    #[test]
    fn group_access_rejects_unjoined_group_from_hostile_relay() {
        let joined = vec!["buzz-general".to_string()];
        assert_eq!(
            check_group_access("attacker-controlled-group", &joined),
            Err(GroupDenied::NotJoined)
        );
    }

    /// Group ids are matched exactly — no glob or case folding, since an `h`
    /// tag is an opaque identifier rather than a user-facing handle.
    #[test]
    fn group_access_matches_exactly() {
        let joined = vec!["buzz-general".to_string()];
        assert_eq!(
            check_group_access("buzz-general-2", &joined),
            Err(GroupDenied::NotJoined)
        );
        assert_eq!(
            check_group_access("BUZZ-GENERAL", &joined),
            Err(GroupDenied::NotJoined)
        );
        assert_eq!(
            check_group_access("*", &joined),
            Err(GroupDenied::NotJoined)
        );
    }

    #[test]
    fn build_tags_includes_h_and_reply() {
        let author = Keys::generate().public_key();
        let event_id = EventId::all_zeros();
        let tags = build_group_message_tags("grp", Some(event_id), Some(author));
        // h tag is always first.
        assert_eq!(tags[0].content(), Some("grp"));
        // e tag and p tag present.
        assert!(tags.iter().any(|t| t.kind() == TagKind::e()));
        assert!(tags.iter().any(|t| t.kind() == TagKind::p()));
    }

    #[test]
    fn build_tags_h_only_when_no_reply() {
        let tags = build_group_message_tags("grp", None, None);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].content(), Some("grp"));
    }
}
