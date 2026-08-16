//! Wire-format round trips for Buzz group chat, against a real relay.
//!
//! The kinds and tag shapes here were taken from Buzz's own source
//! (`crates/buzz-core/src/kind.rs` and `crates/buzz-sdk/src/builders.rs`).
//! Reading them is not the same as proving we emit them, so these tests publish
//! through the actual `nostr-sdk` client into an in-process relay and assert on
//! what comes back off the wire — kind numbers, tag names, tag ordering and the
//! NIP-10 marker. A mistake in any of those is invisible in unit tests but makes
//! the bot silently malformed in a Buzz channel.
//!
//! No network access is involved: `MockRelay` binds to loopback.

#![allow(clippy::expect_used)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use {
    moltis_channels::{
        ChannelOutbound, ChannelStreamOutbound, StreamReceiver, otp::OtpState, plugin::StreamEvent,
    },
    moltis_nostr::{
        config::NostrAccountConfig,
        groups::{self, GroupMessageKind},
        outbound::{AccountStateMap, NostrOutbound},
        reply_ctx::ReplyContexts,
        state::AccountState,
    },
    nostr_relay_builder::MockRelay,
    nostr_sdk::prelude::{
        Client, Event, EventBuilder, EventId, Filter, Keys, Kind, PublicKey, Tag, TagKind,
    },
    tokio::sync::mpsc,
    tokio_util::sync::CancellationToken,
};

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Connect a client to a fresh in-process relay.
async fn connected_client() -> (MockRelay, Client, Keys) {
    let relay = MockRelay::run().await.expect("start mock relay");
    let keys = Keys::generate();
    let client = Client::new(keys.clone());
    client
        .add_relay(relay.url().await)
        .await
        .expect("add relay");
    client.connect().await;
    (relay, client, keys)
}

/// Read back every stored event of `kind`.
async fn fetch_kind(client: &Client, kind: Kind) -> Vec<Event> {
    client
        .fetch_events(Filter::new().kind(kind), FETCH_TIMEOUT)
        .await
        .expect("fetch events")
        .into_iter()
        .collect()
}

/// All values of a single-letter tag, in document order.
fn tag_values<'a>(event: &'a Event, kind: TagKind<'a>) -> Vec<&'a [String]> {
    event.tags.filter(kind).map(Tag::as_slice).collect()
}

/// A Buzz reply must land as kind:40002 carrying the channel `h` tag, an `e`
/// tag with the NIP-10 `"reply"` marker, and a `p` tag notifying the author.
#[tokio::test]
async fn buzz_reply_round_trips_with_expected_kind_and_tags() {
    let (_relay, client, _keys) = connected_client().await;

    let parent = EventId::all_zeros();
    let author = Keys::generate().public_key();
    let published = groups::send_group_message(
        &client,
        groups::buzz_stream_message_kind(),
        "buzz-general",
        "hello from moltis",
        Some(parent),
        Some(author),
    )
    .await
    .expect("publish group message");

    let events = fetch_kind(&client, groups::buzz_stream_message_kind()).await;
    let event = events
        .iter()
        .find(|e| e.id == published)
        .expect("published message stored by relay");

    assert_eq!(event.kind.as_u16(), 40_002, "Buzz stream message v2");
    assert_eq!(event.content, "hello from moltis");

    // Channel scoping: NIP-29 `h` tag, not an `e` tag.
    let h = tag_values(event, TagKind::h());
    assert_eq!(h.len(), 1, "exactly one channel tag");
    assert_eq!(h[0], ["h".to_string(), "buzz-general".to_string()]);

    // Threading: NIP-10 marker, matching Buzz's build_message.
    let e = tag_values(event, TagKind::e());
    assert_eq!(e.len(), 1);
    assert_eq!(
        e[0],
        [
            "e".to_string(),
            parent.to_hex(),
            String::new(),
            "reply".to_string()
        ],
        "reply must carry the NIP-10 \"reply\" marker"
    );

    // Mention: the author being replied to is notified.
    let p = tag_values(event, TagKind::p());
    assert_eq!(p.len(), 1);
    assert_eq!(p[0][1], author.to_hex());
}

/// A standard NIP-29 relay reply must be kind:9, with no Buzz-only kinds.
#[tokio::test]
async fn nip29_reply_round_trips_as_kind_9() {
    let (_relay, client, _keys) = connected_client().await;

    let published = groups::send_group_message(
        &client,
        groups::group_chat_kind(),
        "grp",
        "plain nip29",
        None,
        None,
    )
    .await
    .expect("publish group message");

    let events = fetch_kind(&client, groups::group_chat_kind()).await;
    let event = events
        .iter()
        .find(|e| e.id == published)
        .expect("stored by relay");

    assert_eq!(event.kind.as_u16(), 9);
    // Only the channel tag when there is nothing to thread or notify.
    assert_eq!(tag_values(event, TagKind::h()).len(), 1);
    assert!(tag_values(event, TagKind::e()).is_empty());
    assert!(tag_values(event, TagKind::p()).is_empty());
}

/// Streaming edits publish kind:40003 with `["h", channel]` and `["e", target]`,
/// exactly as Buzz's build_edit does, and carry the replacement text.
#[tokio::test]
async fn edit_round_trips_as_kind_40003() {
    let (_relay, client, _keys) = connected_client().await;

    let target = groups::send_group_message(
        &client,
        groups::buzz_stream_message_kind(),
        "buzz-general",
        "partial",
        None,
        None,
    )
    .await
    .expect("publish original");

    groups::edit_group_message(
        &client,
        "buzz-general",
        target,
        "partial answer, now complete",
    )
    .await
    .expect("publish edit");

    let events = fetch_kind(&client, groups::buzz_edit_kind()).await;
    let edit = events.first().expect("edit stored by relay");

    assert_eq!(edit.kind.as_u16(), 40_003);
    assert_eq!(edit.content, "partial answer, now complete");
    assert_eq!(tag_values(edit, TagKind::h())[0], [
        "h".to_string(),
        "buzz-general".to_string()
    ]);
    // Buzz's build_edit uses a bare `e` tag here — no NIP-10 marker.
    assert_eq!(tag_values(edit, TagKind::e())[0], [
        "e".to_string(),
        target.to_hex()
    ]);
}

/// Acknowledgement reactions are NIP-25 kind:7 with the emoji as content, an
/// `e` tag naming the message (as in Buzz's build_reaction) and an `h` tag
/// scoping them to the group, which NIP-29 relays require.
#[tokio::test]
async fn reaction_round_trips_as_nip25_kind_7() {
    let (_relay, client, _keys) = connected_client().await;

    let target = groups::send_group_message(
        &client,
        groups::buzz_stream_message_kind(),
        "buzz-general",
        "work please",
        None,
        None,
    )
    .await
    .expect("publish target");

    let reaction_id = groups::send_reaction(
        &client,
        "buzz-general",
        target,
        groups::ack_emoji_glyph("eyes"),
    )
    .await
    .expect("publish reaction");

    let events = fetch_kind(&client, Kind::Reaction).await;
    let reaction = events
        .iter()
        .find(|e| e.id == reaction_id)
        .expect("reaction stored by relay");

    assert_eq!(reaction.kind.as_u16(), 7);
    assert_eq!(reaction.content, "👀", "NIP-25 wants the glyph");
    assert_eq!(tag_values(reaction, TagKind::e())[0], [
        "e".to_string(),
        target.to_hex()
    ]);
    // NIP-29 scopes group events by `h`; a strict relay refuses unscoped writes.
    assert_eq!(tag_values(reaction, TagKind::h())[0], [
        "h".to_string(),
        "buzz-general".to_string()
    ]);
}

/// Retracting an acknowledgement publishes a NIP-09 deletion referencing the
/// reaction event, not the message that was reacted to.
#[tokio::test]
async fn retracting_a_reaction_publishes_nip09_deletion() {
    let (_relay, client, _keys) = connected_client().await;

    let target = EventId::all_zeros();
    let reaction_id = groups::send_reaction(&client, "grp", target, "👀")
        .await
        .expect("publish reaction");

    groups::delete_event(&client, "grp", reaction_id)
        .await
        .expect("publish deletion");

    let events = fetch_kind(&client, Kind::EventDeletion).await;
    let deletion = events.first().expect("deletion stored by relay");

    assert_eq!(deletion.kind.as_u16(), 5);
    assert_eq!(
        tag_values(deletion, TagKind::e())[0],
        ["e".to_string(), reaction_id.to_hex()],
        "deletion must reference the reaction, not the reacted-to message"
    );
    assert_eq!(tag_values(deletion, TagKind::h())[0], [
        "h".to_string(),
        "grp".to_string()
    ]);
}

/// An `h`-tag subscription must deliver both dialects, since a Buzz channel can
/// legitimately carry either.
#[tokio::test]
async fn group_filter_matches_both_dialects() {
    let (_relay, client, _keys) = connected_client().await;

    for kind in groups::group_message_kinds() {
        groups::send_group_message(&client, kind, "buzz-general", "hi", None, None)
            .await
            .expect("publish");
    }
    // A message in a different channel must not match the filter.
    groups::send_group_message(
        &client,
        groups::buzz_stream_message_kind(),
        "other-channel",
        "elsewhere",
        None,
        None,
    )
    .await
    .expect("publish");

    let filter = Filter::new()
        .kinds(groups::group_message_kinds())
        .custom_tags(groups::h_tag(), ["buzz-general".to_string()]);
    let events: Vec<Event> = client
        .fetch_events(filter, FETCH_TIMEOUT)
        .await
        .expect("fetch")
        .into_iter()
        .collect();

    assert_eq!(events.len(), 2, "both kind:9 and kind:40002 in the channel");
    let mut kinds: Vec<u16> = events.iter().map(|e| e.kind.as_u16()).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, vec![9, 40_002]);
}

/// The production client runs with `verify_subscriptions(true)`, which makes
/// the SDK re-check every delivered event against the filters we subscribed
/// with and silently drop the rest. That is a security boundary, but it also
/// means a mismatch between our `h`-tag filter and the SDK's own matcher would
/// make the bot go quiet in group chat with nothing logged. Subscribe exactly
/// as `bus` does, publish from a second client, and require both dialects to
/// arrive as notifications.
#[tokio::test]
async fn verified_subscription_still_delivers_both_group_dialects() {
    let relay = MockRelay::run().await.expect("start mock relay");
    let url = relay.url().await;

    let bot = moltis_nostr::client::build_client(Keys::generate());
    bot.add_relay(&url).await.expect("add relay");
    bot.connect().await;

    let mut notifications = bot.notifications();
    bot.subscribe(
        Filter::new()
            .kinds(groups::group_message_kinds())
            .custom_tags(groups::h_tag(), ["buzz-general".to_string()]),
        None,
    )
    .await
    .expect("subscribe");

    let author = Client::new(Keys::generate());
    author.add_relay(&url).await.expect("add relay");
    author.connect().await;
    for kind in groups::group_message_kinds() {
        groups::send_group_message(&author, kind, "buzz-general", "hi", None, None)
            .await
            .expect("publish");
    }
    // Must not be delivered: different group, same kinds.
    groups::send_group_message(
        &author,
        groups::buzz_stream_message_kind(),
        "other-channel",
        "elsewhere",
        None,
        None,
    )
    .await
    .expect("publish");

    let mut received: Vec<u16> = Vec::new();
    let collect = async {
        while received.len() < 2 {
            if let Ok(nostr_sdk::prelude::RelayPoolNotification::Event { event, .. }) =
                notifications.recv().await
            {
                assert_eq!(
                    groups::extract_group_id(&event).as_deref(),
                    Some("buzz-general")
                );
                received.push(event.kind.as_u16());
            }
        }
    };
    tokio::time::timeout(FETCH_TIMEOUT, collect)
        .await
        .expect("both group messages must survive subscription verification");

    received.sort_unstable();
    assert_eq!(received, vec![9, 40_002]);
}

/// The `h` tag survives the round trip well enough for the inbound gate to read
/// it back — the value the access check keys on.
#[tokio::test]
async fn published_group_id_is_readable_by_the_inbound_gate() {
    let (_relay, client, _keys) = connected_client().await;

    groups::send_group_message(
        &client,
        groups::buzz_stream_message_kind(),
        "buzz-general",
        "hi",
        None,
        None,
    )
    .await
    .expect("publish");

    let events = fetch_kind(&client, groups::buzz_stream_message_kind()).await;
    let event = events.first().expect("stored");

    assert_eq!(
        groups::extract_group_id(event).as_deref(),
        Some("buzz-general")
    );
    assert!(groups::is_group_message_kind(event.kind));
}

/// A `p`-tagged message is seen as a mention by the responder gate; an
/// untagged one is not.
#[tokio::test]
async fn mention_detection_works_on_round_tripped_events() {
    let (_relay, client, _keys) = connected_client().await;
    let bot: PublicKey = Keys::generate().public_key();

    let mentioned = groups::send_group_message(
        &client,
        groups::buzz_stream_message_kind(),
        "buzz-general",
        "hey bot",
        None,
        Some(bot),
    )
    .await
    .expect("publish mention");

    let events = fetch_kind(&client, groups::buzz_stream_message_kind()).await;
    let event = events
        .iter()
        .find(|e| e.id == mentioned)
        .expect("stored by relay");

    assert!(groups::is_bot_mentioned(event, &bot));
    assert!(!groups::is_bot_mentioned(
        event,
        &Keys::generate().public_key()
    ));
}

/// Guard against silently dropping the `h` tag: an event built without it is
/// rejected by the gate even after a relay round trip.
#[tokio::test]
async fn message_without_channel_tag_has_no_group_id() {
    let (_relay, client, keys) = connected_client().await;

    let builder = EventBuilder::new(groups::buzz_stream_message_kind(), "oops");
    let event = builder.sign_with_keys(&keys).expect("sign");
    client.send_event(&event).await.expect("publish");

    let events = fetch_kind(&client, groups::buzz_stream_message_kind()).await;
    let stored = events
        .iter()
        .find(|e| e.id == event.id)
        .expect("stored by relay");

    assert_eq!(groups::extract_group_id(stored), None);
}

// ── Streaming replies ────────────────────────────────────────────────────

/// Build an outbound adapter whose single account talks to `relay`.
async fn outbound_for(relay: &MockRelay, config: NostrAccountConfig) -> NostrOutbound {
    let keys = Keys::generate();
    let client = Client::new(keys.clone());
    client
        .add_relay(relay.url().await)
        .await
        .expect("add relay");
    client.connect().await;

    let state = AccountState {
        client,
        keys,
        config: Arc::new(tokio::sync::RwLock::new(config)),
        cached_allowlist: Arc::new(RwLock::new(Vec::new())),
        cancel: CancellationToken::new(),
        otp: Arc::new(Mutex::new(OtpState::new(300))),
        reply_ctx: Arc::new(Mutex::new(ReplyContexts::new())),
    };
    let accounts: AccountStateMap = Arc::new(RwLock::new(HashMap::new()));
    accounts
        .write()
        .expect("lock")
        .insert("acct".to_string(), state);
    NostrOutbound { accounts }
}

/// Feed a finished stream of chunks.
fn stream_of(chunks: &[&str]) -> StreamReceiver {
    let (tx, rx) = mpsc::channel(16);
    for chunk in chunks {
        tx.try_send(StreamEvent::Delta((*chunk).to_string()))
            .expect("queue chunk");
    }
    tx.try_send(StreamEvent::Done).expect("queue done");
    rx
}

fn group_config(group: &str, kind: GroupMessageKind) -> NostrAccountConfig {
    NostrAccountConfig {
        groups: vec![group.to_string()],
        group_message_kind: kind,
        ..Default::default()
    }
}

/// In a Buzz channel the reply is published early and then revised, so the
/// channel watches the answer form. Chunks arrive faster than the throttle, so
/// the intermediate edits collapse into a single final edit carrying the whole
/// text — the message must never be left truncated.
#[tokio::test]
async fn buzz_stream_publishes_then_edits_to_final_text() {
    let relay = MockRelay::run().await.expect("relay");
    let outbound = outbound_for(
        &relay,
        group_config("buzz-general", GroupMessageKind::BuzzV2),
    )
    .await;

    outbound
        .send_stream(
            "acct",
            &groups::group_target("buzz-general"),
            None,
            stream_of(&["Hello", ", ", "world"]),
        )
        .await
        .expect("stream");

    let client = {
        let accounts = outbound.accounts.read().expect("lock");
        accounts.get("acct").expect("account").client.clone()
    };

    let messages = fetch_kind(&client, groups::buzz_stream_message_kind()).await;
    assert_eq!(messages.len(), 1, "exactly one message, then edits");
    assert_eq!(messages[0].content, "Hello", "published on the first chunk");

    let edits = fetch_kind(&client, groups::buzz_edit_kind()).await;
    assert_eq!(edits.len(), 1, "throttled into one final edit");
    assert_eq!(
        edits[0].content, "Hello, world",
        "final edit must carry the complete answer"
    );
    assert_eq!(
        tag_values(&edits[0], TagKind::e())[0],
        ["e".to_string(), messages[0].id.to_hex()],
        "edit must target the published message"
    );
}

/// A plain NIP-29 relay has no edit kind, so the reply is collected and posted
/// once — publishing a partial message there would strand it truncated.
#[tokio::test]
async fn nip29_stream_sends_one_complete_message_without_edits() {
    let relay = MockRelay::run().await.expect("relay");
    let outbound = outbound_for(&relay, group_config("grp", GroupMessageKind::Nip29)).await;

    outbound
        .send_stream(
            "acct",
            &groups::group_target("grp"),
            None,
            stream_of(&["Hello", ", ", "world"]),
        )
        .await
        .expect("stream");

    let client = {
        let accounts = outbound.accounts.read().expect("lock");
        accounts.get("acct").expect("account").client.clone()
    };

    let messages = fetch_kind(&client, groups::group_chat_kind()).await;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Hello, world", "sent once, complete");
    assert!(
        fetch_kind(&client, groups::buzz_edit_kind())
            .await
            .is_empty(),
        "must not emit Buzz edit kinds on a plain NIP-29 relay"
    );
}

/// Poll until an event of `kind` reaches the relay, so the test synchronises on
/// the publish itself rather than on a sleep.
async fn wait_for_kind(client: &Client, kind: Kind) -> Event {
    let mut found = None;
    for _ in 0..100 {
        found = fetch_kind(client, kind).await.into_iter().next();
        if found.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    found.expect("event published within timeout")
}

/// Group access can be revoked while a turn is still generating — saving
/// channel settings rewrites the very config the stream authorized against.
/// Once revoked, nothing further may reach the relay: the already-published
/// text stays truncated, which is the point, since the operator has withdrawn
/// the bot from that channel and group messages are plaintext to every member.
#[tokio::test]
async fn revoking_the_group_mid_stream_publishes_no_further_events() {
    let relay = MockRelay::run().await.expect("relay");
    let outbound = outbound_for(
        &relay,
        group_config("buzz-general", GroupMessageKind::BuzzV2),
    )
    .await;

    let accounts = outbound.accounts.clone();
    let client = {
        let guard = accounts.read().expect("lock");
        guard.get("acct").expect("account").client.clone()
    };

    let (tx, rx) = mpsc::channel(16);
    let target = groups::group_target("buzz-general");
    let stream = tokio::spawn(async move { outbound.send_stream("acct", &target, None, rx).await });

    // The first chunk publishes the reply that is forming.
    tx.send(StreamEvent::Delta("Hello".to_string()))
        .await
        .expect("queue chunk");
    let published = wait_for_kind(&client, groups::buzz_stream_message_kind()).await;
    assert_eq!(published.content, "Hello");

    // Operator drops the group, exactly as `plugin::upsert_account` does.
    // The handle is cloned out first — the accounts map is a std lock, whose
    // guard must not be held across the config await.
    let config = {
        let guard = accounts.read().expect("lock");
        Arc::clone(&guard.get("acct").expect("account").config)
    };
    config.write().await.groups.clear();

    // The rest of the answer, and the end of the turn, must stay unpublished.
    tx.send(StreamEvent::Delta(", world".to_string()))
        .await
        .expect("queue chunk");
    let _ = tx.send(StreamEvent::Done).await;
    assert!(
        stream.await.expect("join").is_err(),
        "a revoked stream must report the refusal, not silently succeed"
    );

    assert!(
        fetch_kind(&client, groups::buzz_edit_kind())
            .await
            .is_empty(),
        "no edit may be published after group access was revoked"
    );

    // The half-written reply is taken back rather than left standing. It can
    // never be completed — no further edit is permitted — so leaving it would
    // strand a partial sentence in the channel as the bot's final word.
    let deletions = fetch_kind(&client, Kind::EventDeletion).await;
    let deletion = deletions
        .first()
        .expect("the truncated reply must be withdrawn");
    assert_eq!(tag_values(deletion, TagKind::e())[0], [
        "e".to_string(),
        published.id.to_hex()
    ]);
    assert!(
        fetch_kind(&client, groups::buzz_stream_message_kind())
            .await
            .is_empty(),
        "the relay honoured the withdrawal, so nothing of the reply remains"
    );
}

/// The same revocation guarantee on the non-streaming path, which is where a
/// plain NIP-29 reply and every queued or resumed turn go. `send_text` resolves
/// the target, parses the reply id and plans the dialect before publishing, and
/// a settings save landing in that interval must still stop the send — the
/// authorization decision `resolve` made is not carried to the publish.
#[tokio::test]
async fn revoking_the_group_before_a_plain_send_publishes_nothing() {
    let relay = MockRelay::run().await.expect("relay");
    let outbound = outbound_for(&relay, group_config("grp", GroupMessageKind::Nip29)).await;

    let client = {
        let guard = outbound.accounts.read().expect("lock");
        guard.get("acct").expect("account").client.clone()
    };

    // Operator drops the group after the turn was authorized and bound to this
    // reply target, but before the reply is published.
    let config = {
        let guard = outbound.accounts.read().expect("lock");
        Arc::clone(&guard.get("acct").expect("account").config)
    };
    config.write().await.groups.clear();

    let result = outbound
        .send_text("acct", &groups::group_target("grp"), "too late", None)
        .await;

    assert!(
        result.is_err(),
        "a revoked group send must report the refusal, not silently succeed"
    );
    assert!(
        fetch_kind(&client, groups::group_chat_kind())
            .await
            .is_empty(),
        "nothing may reach the relay after group access was revoked"
    );
}

/// Receive-only is enforced at the publish too, not just on the inbound side.
#[tokio::test]
async fn receive_only_mode_publishes_nothing() {
    let relay = MockRelay::run().await.expect("relay");
    let outbound = outbound_for(&relay, NostrAccountConfig {
        groups: vec!["grp".to_string()],
        group_mention_mode: moltis_channels::gating::MentionMode::None,
        ..Default::default()
    })
    .await;

    let client = {
        let guard = outbound.accounts.read().expect("lock");
        guard.get("acct").expect("account").client.clone()
    };

    assert!(
        outbound
            .send_text("acct", &groups::group_target("grp"), "quiet please", None)
            .await
            .is_err()
    );
    assert!(
        fetch_kind(&client, groups::group_chat_kind())
            .await
            .is_empty(),
        "receive-only must not put anything on the relay"
    );
}

/// Withdrawing the bot must not freeze its 👀 on a user's message.
///
/// Every path that puts new content in a channel is refused once the group is
/// revoked — but a retraction takes content *out*, and gating it would mean the
/// last acknowledgement the bot placed stays there permanently, which is the
/// opposite of what removing the bot should do. The NIP-09 deletion therefore
/// still goes out after revocation, while a fresh reaction on the same target
/// is refused.
#[tokio::test]
async fn a_reaction_can_still_be_retracted_after_the_group_is_revoked() {
    let relay = MockRelay::run().await.expect("relay");
    let outbound = outbound_for(
        &relay,
        group_config("buzz-general", GroupMessageKind::BuzzV2),
    )
    .await;
    let chat_id = groups::group_target("buzz-general");
    let target = EventId::all_zeros();

    let (client, config) = {
        let guard = outbound.accounts.read().expect("lock");
        let state = guard.get("acct").expect("account");
        (state.client.clone(), Arc::clone(&state.config))
    };

    // Acknowledge a message while still authorized.
    outbound
        .add_reaction("acct", &chat_id, &target.to_hex(), "eyes")
        .await
        .expect("react");
    let reaction = fetch_kind(&client, Kind::Reaction)
        .await
        .first()
        .expect("reaction stored")
        .id;

    // Operator withdraws the bot mid-turn.
    config.write().await.groups.clear();

    // The terminal ✅ is refused — nothing new may be published...
    assert!(
        outbound
            .add_reaction("acct", &chat_id, &target.to_hex(), "white_check_mark")
            .await
            .is_err(),
        "a revoked group must not accept a new reaction"
    );

    // ...but that same attempt clears the 👀 on its way through, which is the
    // retry path for a retraction nothing else revisits. The deletion carries
    // the group's `h` tag and names the reaction, not the message.
    let deletions = fetch_kind(&client, Kind::EventDeletion).await;
    let deletion = deletions
        .first()
        .expect("the stale acknowledgement must be withdrawn after revocation");
    assert_eq!(tag_values(deletion, TagKind::e())[0], [
        "e".to_string(),
        reaction.to_hex()
    ]);
    assert_eq!(tag_values(deletion, TagKind::h())[0], [
        "h".to_string(),
        "buzz-general".to_string()
    ]);
    assert!(
        fetch_kind(&client, Kind::Reaction).await.is_empty(),
        "the relay honoured the withdrawal, so no acknowledgement is left behind"
    );

    // Retracting again is a no-op: it has already gone.
    outbound
        .remove_reaction("acct", &chat_id, &target.to_hex(), "eyes")
        .await
        .expect("second retraction is a no-op");
}

/// The property the async config lock exists for: a settings save that
/// withdraws the group cannot interleave between the authorization check and
/// the publish.
///
/// A publish holds the config read guard until the event has been handed to the
/// relay, so `update_account_config` (which takes the write guard) either lands
/// entirely before it — and the publish is refused — or entirely after it. It
/// can never land in between. This test drives the write concurrently with a
/// send and asserts the two possible outcomes are the only ones: either the
/// send was refused and nothing is on the relay, or it succeeded and exactly
/// the authorized message is there. What must never happen is a refusal that
/// still published, or a success carrying an unauthorized event.
#[tokio::test]
async fn a_concurrent_revocation_cannot_interleave_with_a_publish() {
    let relay = MockRelay::run().await.expect("relay");
    let outbound =
        Arc::new(outbound_for(&relay, group_config("grp", GroupMessageKind::Nip29)).await);

    let (client, config) = {
        let guard = outbound.accounts.read().expect("lock");
        let state = guard.get("acct").expect("account");
        (state.client.clone(), Arc::clone(&state.config))
    };

    let sender = Arc::clone(&outbound);
    let send = tokio::spawn(async move {
        sender
            .send_text("acct", &groups::group_target("grp"), "racing", None)
            .await
    });
    // Withdraw the group at the same moment the send is being authorized.
    let revoke = tokio::spawn(async move {
        config.write().await.groups.clear();
    });

    let sent = send.await.expect("join");
    revoke.await.expect("join");

    let published = fetch_kind(&client, groups::group_chat_kind()).await;
    match sent {
        Ok(()) => assert_eq!(
            published.len(),
            1,
            "an authorized send must publish exactly its own message"
        ),
        Err(_) => assert!(
            published.is_empty(),
            "a refused send must publish nothing at all"
        ),
    }
}

/// The gateway hands `add_reaction`/`remove_reaction` the reply target's
/// `chat_id`, which is the *prefixed* form (`group:<id>`). The `h` tag on the
/// published kind:7 and kind:5 must still be the bare group id — a NIP-29 relay
/// keys authorization on it, so `group:buzz-general` is either refused or filed
/// under a group nobody is watching.
#[tokio::test]
async fn ack_reactions_use_the_bare_group_id_in_the_h_tag() {
    let relay = MockRelay::run().await.expect("relay");
    let outbound = outbound_for(
        &relay,
        group_config("buzz-general", GroupMessageKind::BuzzV2),
    )
    .await;
    let target = EventId::all_zeros();
    let chat_id = groups::group_target("buzz-general");

    outbound
        .add_reaction("acct", &chat_id, &target.to_hex(), "eyes")
        .await
        .expect("react");

    let client = {
        let accounts = outbound.accounts.read().expect("lock");
        accounts.get("acct").expect("account").client.clone()
    };

    let reactions = fetch_kind(&client, Kind::Reaction).await;
    let reaction = reactions.first().expect("reaction stored by relay");
    assert_eq!(tag_values(reaction, TagKind::h())[0], [
        "h".to_string(),
        "buzz-general".to_string()
    ]);

    // The retraction is scoped the same way, and must reference the reaction.
    outbound
        .remove_reaction("acct", &chat_id, &target.to_hex(), "eyes")
        .await
        .expect("retract");

    let deletions = fetch_kind(&client, Kind::EventDeletion).await;
    let deletion = deletions.first().expect("deletion stored by relay");
    assert_eq!(tag_values(deletion, TagKind::h())[0], [
        "h".to_string(),
        "buzz-general".to_string()
    ]);
    assert_eq!(tag_values(deletion, TagKind::e())[0], [
        "e".to_string(),
        reaction.id.to_hex()
    ]);
}

/// A stream that errors before producing text still says something rather than
/// leaving the channel silent.
#[tokio::test]
async fn stream_error_before_any_text_still_replies() {
    let relay = MockRelay::run().await.expect("relay");
    let outbound = outbound_for(&relay, group_config("grp", GroupMessageKind::Nip29)).await;

    let (tx, rx) = mpsc::channel(4);
    tx.try_send(StreamEvent::Error("boom".into()))
        .expect("queue");
    drop(tx);

    outbound
        .send_stream("acct", &groups::group_target("grp"), None, rx)
        .await
        .expect("stream");

    let client = {
        let accounts = outbound.accounts.read().expect("lock");
        accounts.get("acct").expect("account").client.clone()
    };
    let messages = fetch_kind(&client, groups::group_chat_kind()).await;
    assert_eq!(messages.len(), 1, "no silent failure");
    assert!(messages[0].content.contains("Error"));
}
