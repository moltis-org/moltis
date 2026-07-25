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
    moltis_channels::gating::{self, GroupPolicy, MentionMode},
    nostr_sdk::prelude::{
        Alphabet, Client, Event, EventBuilder, EventId, Kind, PublicKey, SingleLetterTag, Tag,
        TagKind,
    },
};

use crate::error::Error;

/// NIP-29 group chat message kind (`kind:9`).
pub const GROUP_CHAT_KIND: u16 = 9;

/// The NIP-29 group chat message [`Kind`] (`kind:9`).
#[must_use]
pub fn group_chat_kind() -> Kind {
    Kind::from_u16(GROUP_CHAT_KIND)
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
    /// Group participation is disabled for this account.
    Disabled,
    /// The group id is not on the allowlist.
    NotAllowlisted,
}

impl std::fmt::Display for GroupDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "groups are disabled"),
            Self::NotAllowlisted => write!(f, "group not in allowlist"),
        }
    }
}

/// Check whether a group is allowed under the given policy and allowlist.
///
/// An empty allowlist under [`GroupPolicy::Allowlist`] allows every group,
/// matching the shared [`gating::is_allowed`] convention used across channels.
pub fn check_group_access(
    group_id: &str,
    policy: &GroupPolicy,
    allowed: &[String],
) -> Result<(), GroupDenied> {
    match policy {
        GroupPolicy::Disabled => Err(GroupDenied::Disabled),
        GroupPolicy::Open => Ok(()),
        GroupPolicy::Allowlist => {
            if gating::is_allowed(group_id, allowed) {
                Ok(())
            } else {
                Err(GroupDenied::NotAllowlisted)
            }
        },
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

/// Publish a plaintext NIP-29 group chat message (`kind:9`) to the relay.
///
/// The event is signed by the client's configured signer (the bot keys) and
/// carries the `h` tag routing it to `group_id`. When `reply_to` is set, a
/// NIP-10 `e` tag threads the reply to the originating message.
pub async fn send_group_message(
    client: &Client,
    group_id: &str,
    text: &str,
    reply_to: Option<EventId>,
    mention: Option<PublicKey>,
) -> Result<(), Error> {
    let tags = build_group_message_tags(group_id, reply_to, mention);
    let builder = EventBuilder::new(group_chat_kind(), text).tags(tags);
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
    fn group_access_open_allows_any() {
        assert!(check_group_access("anything", &GroupPolicy::Open, &[]).is_ok());
    }

    #[test]
    fn group_access_disabled_denies_all() {
        assert_eq!(
            check_group_access("g", &GroupPolicy::Disabled, &["g".into()]),
            Err(GroupDenied::Disabled)
        );
    }

    #[test]
    fn group_access_allowlist_matches() {
        let allowed = vec!["buzz-general".to_string()];
        assert!(check_group_access("buzz-general", &GroupPolicy::Allowlist, &allowed).is_ok());
        assert_eq!(
            check_group_access("other", &GroupPolicy::Allowlist, &allowed),
            Err(GroupDenied::NotAllowlisted)
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
