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

use crate::{error::Error, state::SharedConfig};

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

/// Marks a reply target as a group rather than a DM recipient.
///
/// Outbound only receives a `chat_id`, and a NIP-29 group id is an opaque
/// string that can perfectly well be 64 hex characters — which is also exactly
/// what a Nostr public key looks like. Classifying by "is this id currently in
/// `groups`?" therefore breaks the moment the group is removed: the reply
/// target persists in the session (`_channel_reply_target`), so a queued or
/// resumed turn would fall through to the DM path, parse the former group id as
/// a pubkey, and gift-wrap the channel's reply to whoever owns that key.
///
/// Prefixing removes the ambiguity: the target carries its own kind, so
/// classification never depends on mutable config, and a stale group target
/// fails closed instead of being misdelivered.
pub const GROUP_TARGET_PREFIX: &str = "group:";

/// Build the reply-target chat id for a group.
#[must_use]
pub fn group_target(group_id: &str) -> String {
    format!("{GROUP_TARGET_PREFIX}{group_id}")
}

/// Recover the group id from a reply target, if it is one.
#[must_use]
pub fn parse_group_target(target: &str) -> Option<&str> {
    target.strip_prefix(GROUP_TARGET_PREFIX)
}

/// Channel commands that must not be reachable by an arbitrary group member.
///
/// Group chat has no per-sender allowlist by design — NIP-29 makes the relay
/// the authority on membership. That is fine for conversation, but these two
/// commands control *code execution*: `/sh` turns subsequent messages in the
/// session into shell commands, and `/sandbox` can move execution off the
/// sandbox onto the host. Anyone the relay admits to a joined channel could
/// otherwise reach them.
///
/// The gateway already gates `/approve`, `/deny` and `/update` centrally, so
/// they are deliberately not repeated here.
pub const OPERATOR_ONLY_COMMANDS: &[&str] = &["sh", "sandbox"];

/// Whether `command` may only be run by an operator (see
/// [`OPERATOR_ONLY_COMMANDS`]).
#[must_use]
pub fn is_operator_only_command(command: &str) -> bool {
    OPERATOR_ONLY_COMMANDS.contains(&command)
}

/// Whether a group sender counts as an operator of this account.
///
/// Reuses the account's DM allowlist (`allowed_pubkeys`) as the operator list
/// rather than inventing a second one: those are the keys the owner already
/// nominated as trusted. An empty allowlist means nobody is an operator, so
/// execution commands fail closed in groups.
#[must_use]
pub fn is_group_operator(sender: &PublicKey, allowed: &[PublicKey]) -> bool {
    allowed.contains(sender)
}

/// Build the tags for an outbound group message: the required `h` tag scoping
/// it to the group, plus an optional NIP-10 reply `e` tag and an author `p` tag
/// mentioning the person being replied to.
///
/// The reply tag carries the NIP-10 `"reply"` marker (`["e", <id>, "", "reply"]`)
/// rather than a bare `["e", <id>]`, matching how Buzz's own SDK builds threaded
/// messages — an unmarked `e` tag is not recognised as a threaded reply.
#[must_use]
pub fn build_group_message_tags(
    group_id: &str,
    reply_to: Option<EventId>,
    mention: Option<PublicKey>,
) -> Vec<Tag> {
    let mut tags = vec![Tag::custom(TagKind::h(), [group_id.to_string()])];
    if let Some(event_id) = reply_to {
        tags.push(Tag::custom(TagKind::e(), [
            event_id.to_hex(),
            String::new(),
            "reply".to_string(),
        ]));
    }
    if let Some(pubkey) = mention {
        tags.push(Tag::public_key(pubkey));
    }
    tags
}

/// Authorize a group publish and perform it **without releasing the config lock
/// in between**.
///
/// The single gate every outbound group event goes through — replies, streamed
/// edits, reactions, and the command responses `bus` sends directly. It lives
/// here rather than in `outbound` so those two callers cannot drift apart.
///
/// A check that merely runs before the publish leaves an interval — however
/// short — in which a settings save can withdraw the group while the event is
/// already on its way to the relay, and group messages are plaintext to every
/// member of the channel. Holding the read guard across `publish` removes it: a
/// writer disabling group chat blocks until the in-flight publish finishes, and
/// any publish starting afterwards observes the new config, so "the operator's
/// save has returned" and "nothing further is published" become the same
/// instant. This is why [`SharedConfig`] is a `tokio::sync::RwLock`.
///
/// `publish` must not touch the config lock: `tokio::sync::RwLock` is
/// write-preferring, so a nested read while a writer waits would deadlock.
/// Resolve anything config-derived before calling.
pub async fn authorized_publish<T, F>(
    config: &SharedConfig,
    group_id: &str,
    publish: F,
) -> Result<T, Error>
where
    F: AsyncFnOnce() -> Result<T, Error>,
{
    let cfg = config.read().await;
    check_group_send(&cfg.groups, &cfg.group_mention_mode, group_id)?;
    let result = publish().await;
    drop(cfg);
    result
}

/// Whether the account may currently publish into `group_id`.
///
/// Both ways of turning group chat off are honoured, reusing
/// [`check_group_access`] so the outbound direction cannot drift from the
/// inbound one:
///
/// * clearing `groups` (or removing this one) — the join list *is* the access
///   model, and an empty list means group chat is disabled;
/// * `group_mention_mode = "none"`, documented as receive-only.
///
/// Takes the two fields rather than the whole config so it is obvious what a
/// send decision actually depends on.
pub fn check_group_send(
    groups: &[String],
    mention_mode: &MentionMode,
    group_id: &str,
) -> Result<(), Error> {
    check_group_access(group_id, groups)
        .map_err(|denied| Error::GroupSendDenied(format!("{group_id}: {denied}")))?;
    if *mention_mode == MentionMode::None {
        return Err(Error::GroupSendDenied(format!(
            "{group_id}: receive-only (group_mention_mode = none)"
        )));
    }
    Ok(())
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
) -> Result<EventId, Error> {
    let tags = build_group_message_tags(group_id, reply_to, mention);
    let builder = EventBuilder::new(kind, text).tags(tags);
    let output = client.send_event_builder(builder).await?;
    require_accepted(output)
}

/// Treat an event that no relay accepted as a failure.
///
/// `RelayPool::send_event_to` returns `Ok` even when every relay answered
/// `OK false` — the rejections are collected into `output.failed` and the call
/// still succeeds. That default is dangerous for group chat, where rejection is
/// the *expected* response to a routine misconfiguration: a NIP-29 relay
/// refuses posts from a pubkey it has not admitted to the group
/// (`"restricted: ..."`). Without this check the bot would report success,
/// acknowledge the user with a ✅, and say nothing in the channel.
fn require_accepted(output: nostr_sdk::prelude::Output<EventId>) -> Result<EventId, Error> {
    if output.success.is_empty() {
        let reasons = output
            .failed
            .iter()
            .map(|(url, err)| format!("{url}: {err}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(Error::Relay(if reasons.is_empty() {
            "no relay accepted the event".to_string()
        } else {
            format!("all relays rejected the event ({reasons})")
        }));
    }
    Ok(*output.id())
}

/// Buzz's stream-message edit kind (`kind:40003`, `KIND_STREAM_MESSAGE_EDIT`).
pub const BUZZ_STREAM_MESSAGE_EDIT_KIND: u16 = 40_003;

/// The edit [`Kind`] (`kind:40003`).
#[must_use]
pub fn buzz_edit_kind() -> Kind {
    Kind::from_u16(BUZZ_STREAM_MESSAGE_EDIT_KIND)
}

/// Whether messages in this dialect can be edited in place.
///
/// Editing is a Buzz extension (`kind:40003`); a plain NIP-29 relay has no
/// equivalent, so callers must fall back to posting a single final message.
#[must_use]
pub fn supports_edit(kind: Kind) -> bool {
    kind == buzz_stream_message_kind()
}

/// Replace the text of a previously published group message (Buzz `kind:40003`).
///
/// Tags mirror Buzz's own builder exactly: `["h", <group>]` and
/// `["e", <target>]`, with the full replacement text as the content.
pub async fn edit_group_message(
    client: &Client,
    group_id: &str,
    target: EventId,
    text: &str,
) -> Result<(), Error> {
    let builder = EventBuilder::new(buzz_edit_kind(), text).tags([
        Tag::custom(TagKind::h(), [group_id.to_string()]),
        Tag::custom(TagKind::e(), [target.to_hex()]),
    ]);
    let output = client.send_event_builder(builder).await?;
    require_accepted(output)?;
    Ok(())
}

/// React to a message with an emoji (NIP-25 `kind:7`).
///
/// The emoji goes in the content and `["e", <target>]` names the message, as in
/// Buzz's own builder. The `["h", <group>]` tag is additional: NIP-29 scopes
/// group events by `h`, and a strict relay will refuse an unscoped write, so
/// omitting it would make acknowledgements fail everywhere except Buzz. Buzz
/// itself does not require the tag, and an extra tag is inert there.
///
/// Returns the reaction's own event id, which is what a later NIP-09 deletion
/// has to reference.
pub async fn send_reaction(
    client: &Client,
    group_id: &str,
    target: EventId,
    emoji: &str,
) -> Result<EventId, Error> {
    let builder = EventBuilder::new(Kind::Reaction, emoji).tags([
        Tag::custom(TagKind::h(), [group_id.to_string()]),
        Tag::custom(TagKind::e(), [target.to_hex()]),
    ]);
    let output = client.send_event_builder(builder).await?;
    require_accepted(output)
}

/// Request deletion of an event we published (NIP-09 `kind:5`).
///
/// Used to retract an acknowledgement reaction once the turn finishes. Nostr
/// deletion is a *request* — relays honour it at their discretion. Carries the
/// `h` tag for the same NIP-29 scoping reason as [`send_reaction`].
pub async fn delete_event(client: &Client, group_id: &str, target: EventId) -> Result<(), Error> {
    let builder = EventBuilder::new(Kind::EventDeletion, "").tags([
        Tag::custom(TagKind::h(), [group_id.to_string()]),
        Tag::custom(TagKind::e(), [target.to_hex()]),
    ]);
    let output = client.send_event_builder(builder).await?;
    require_accepted(output)?;
    Ok(())
}

/// Render an acknowledgement emoji in the form NIP-25 expects.
///
/// The reaction controller emits the channel-neutral glyph vocabulary
/// ([`ack_emoji::ALL`](moltis_channels::activity::ack_emoji::ALL)) — 👀 on
/// receipt, a phase glyph per tool, then ✅/❌ — and NIP-25 wants exactly that:
/// the literal glyph (or `+`/`-`). Glyphs therefore pass through untouched.
///
/// The shortcode arm remains because a name can also arrive from a config
/// override rather than the controller, and Slack-style shortcodes are the form
/// operators are used to writing. Publishing `white_check_mark` as the content
/// of a `kind:7` would render as that literal text in every NIP-25 client.
#[must_use]
pub fn ack_emoji_glyph(shortcode: &str) -> &str {
    match shortcode {
        "eyes" => "👀",
        "white_check_mark" | "heavy_check_mark" => "✅",
        "x" | "negative_squared_cross_mark" => "❌",
        "hourglass_flowing_sand" => "⏳",
        "globe_with_meridians" => "🌐",
        "computer" => "💻",
        "pencil2" => "✏️",
        "airplane_departure" => "🛫",
        "building_construction" => "🏗️",
        "hammer_and_wrench" => "🛠️",
        other => other,
    }
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

    /// `RelayPool::send_event_to` returns `Ok` even when every relay answered
    /// `OK false`, so a NIP-29 relay refusing a post from a pubkey it has not
    /// admitted would otherwise look like a successful send — the bot would ✅
    /// the user and say nothing in the channel.
    #[test]
    fn rejection_by_every_relay_is_an_error() {
        use std::collections::{HashMap, HashSet};

        use nostr_sdk::prelude::{Output, RelayUrl};

        let url = RelayUrl::parse("wss://relay.example").expect("parse relay url");
        let rejected: Output<EventId> = Output {
            val: EventId::all_zeros(),
            success: HashSet::new(),
            failed: HashMap::from([(url, "restricted: not a member of the group".to_string())]),
        };

        let err = require_accepted(rejected).expect_err("all-rejected must fail");
        let msg = err.to_string();
        assert!(msg.contains("rejected"), "unexpected message: {msg}");
        // The relay's reason has to survive, or the operator cannot diagnose it.
        assert!(msg.contains("not a member"), "reason lost: {msg}");
    }

    /// One accepting relay is enough — partial failure is normal on Nostr.
    #[test]
    fn acceptance_by_any_relay_succeeds() {
        use std::collections::{HashMap, HashSet};

        use nostr_sdk::prelude::{Output, RelayUrl};

        let good = RelayUrl::parse("wss://good.example").expect("parse");
        let bad = RelayUrl::parse("wss://bad.example").expect("parse");
        let id = EventId::all_zeros();
        let mixed: Output<EventId> = Output {
            val: id,
            success: HashSet::from([good]),
            failed: HashMap::from([(bad, "rate-limited".to_string())]),
        };

        assert_eq!(require_accepted(mixed).ok(), Some(id));
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

    /// Buzz threads on the NIP-10 `"reply"` marker; a bare `["e", id]` is not
    /// recognised as a threaded reply.
    #[test]
    fn reply_tag_carries_nip10_reply_marker() {
        let event_id = EventId::all_zeros();
        let tags = build_group_message_tags("grp", Some(event_id), None);
        let reply = tags
            .iter()
            .find(|t| t.kind() == TagKind::e())
            .expect("e tag present");
        assert_eq!(reply.as_slice(), &[
            "e".to_string(),
            event_id.to_hex(),
            String::new(),
            "reply".to_string()
        ]);
    }

    #[test]
    fn edit_kind_is_40003_and_only_buzz_supports_it() {
        assert_eq!(buzz_edit_kind().as_u16(), 40_003);
        assert!(supports_edit(buzz_stream_message_kind()));
        // Plain NIP-29 has no edit kind, so streaming must not try to edit.
        assert!(!supports_edit(group_chat_kind()));
    }

    /// Only the execution-controlling commands are operator-gated; ordinary
    /// ones must stay usable by anyone in the channel.
    #[test]
    fn only_execution_commands_are_operator_only() {
        assert!(is_operator_only_command("sh"));
        assert!(is_operator_only_command("sandbox"));
        for open_cmd in ["help", "new", "clear", "context", "model", "compact"] {
            assert!(
                !is_operator_only_command(open_cmd),
                "/{open_cmd} should not be operator-gated"
            );
        }
    }

    /// The DM allowlist doubles as the operator list; empty means nobody, so
    /// execution commands fail closed rather than open.
    #[test]
    fn group_operator_is_drawn_from_the_allowlist() {
        let operator = Keys::generate().public_key();
        let stranger = Keys::generate().public_key();
        let allowed = vec![operator];

        assert!(is_group_operator(&operator, &allowed));
        assert!(!is_group_operator(&stranger, &allowed));
        assert!(
            !is_group_operator(&operator, &[]),
            "an empty allowlist must not make everyone an operator"
        );
    }

    #[test]
    fn ack_shortcodes_map_to_nip25_glyphs() {
        // A shortcode from a config override still has to reach the wire as a
        // glyph — NIP-25 renders the content literally.
        assert_eq!(ack_emoji_glyph("eyes"), "👀");
        assert_eq!(ack_emoji_glyph("white_check_mark"), "✅");
        assert_eq!(ack_emoji_glyph("x"), "❌");
        assert_eq!(ack_emoji_glyph("globe_with_meridians"), "🌐");
        // Already a glyph, or unknown: pass through untouched.
        assert_eq!(ack_emoji_glyph("🎉"), "🎉");
        assert_eq!(ack_emoji_glyph("+"), "+");
    }

    /// The cross-crate contract `ack_emoji::ALL` exists for: the reaction
    /// controller emits this vocabulary — 👀, the phase glyphs, then ✅/❌ — and
    /// NIP-25 puts the content on the wire verbatim. Every one of them must
    /// survive unchanged, so adding a canonical emoji that Nostr mangles into
    /// literal text fails here rather than in a channel.
    #[test]
    fn every_canonical_ack_emoji_reaches_nip25_unchanged() {
        for glyph in moltis_channels::activity::ack_emoji::ALL {
            assert_eq!(
                ack_emoji_glyph(glyph),
                *glyph,
                "canonical ack emoji {glyph} must pass through untouched"
            );
            assert!(
                !glyph.is_ascii(),
                "{glyph} is not a glyph — a shortcode would publish as literal text"
            );
        }
    }

    #[test]
    fn build_tags_h_only_when_no_reply() {
        let tags = build_group_message_tags("grp", None, None);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].content(), Some("grp"));
    }
}
