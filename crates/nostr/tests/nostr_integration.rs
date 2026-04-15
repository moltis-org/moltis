//! Live integration tests for the Nostr channel.
//!
//! These tests connect to real Nostr relays and require environment variables:
//!   - `NOSTR_TEST_BOT_KEY`: nsec1 or hex secret key for the bot
//!   - `NOSTR_TEST_SENDER_KEY` (optional): nsec1 or hex secret key for a
//!     simulated sender, used for round-trip DM tests
//!
//! They are `#[ignore]`d by default so `cargo test` skips them.
//!
//! Run with:
//!   cargo test -p moltis-nostr --test nostr_integration -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used, unused_qualifications)]

use std::time::Duration;

use {nostr_sdk::prelude::*, secrecy::Secret};

const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://relay.nostr.band",
    "wss://nos.lol",
];

fn bot_secret() -> Secret<String> {
    let key = std::env::var("NOSTR_TEST_BOT_KEY")
        .expect("NOSTR_TEST_BOT_KEY must be set for integration tests");
    Secret::new(key)
}

fn sender_secret() -> Option<Secret<String>> {
    std::env::var("NOSTR_TEST_SENDER_KEY").ok().map(Secret::new)
}

// ── Key parsing ─────────────────────────────────────────────

#[test]
#[ignore]
fn bot_key_parses_successfully() {
    let keys = moltis_nostr::keys::derive_keys(&bot_secret());
    assert!(keys.is_ok(), "bot key must parse: {keys:?}");
    let keys = keys.unwrap();
    let npub = keys.public_key().to_bech32().unwrap();
    println!("Bot public key: {npub}");
}

// ── Relay connectivity ──────────────────────────────────────

#[tokio::test]
#[ignore]
async fn connects_to_default_relays() {
    let keys = moltis_nostr::keys::derive_keys(&bot_secret()).unwrap();
    let client = Client::new(keys);

    for relay in DEFAULT_RELAYS {
        client.add_relay(*relay).await.expect("add relay");
    }

    client.connect().await;
    tokio::time::sleep(Duration::from_secs(3)).await;

    let relays = client.relays().await;
    let connected = relays
        .values()
        .filter(|r| r.status() == RelayStatus::Connected)
        .count();

    println!("{connected}/{} relays connected", relays.len());
    assert!(connected > 0, "at least one relay must be connected");

    client.disconnect().await;
}

// ── Profile publishing ──────────────────────────────────────

#[tokio::test]
#[ignore]
async fn publish_profile_metadata() {
    let keys = moltis_nostr::keys::derive_keys(&bot_secret()).unwrap();
    let client = Client::new(keys);

    for relay in DEFAULT_RELAYS {
        let _ = client.add_relay(*relay).await;
    }
    client.connect().await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    let profile = moltis_nostr::config::NostrProfile {
        name: Some("Moltis Integration Test".into()),
        about: Some("Automated test bot — do not interact".into()),
        ..Default::default()
    };

    let result = moltis_nostr::profile::publish_profile(&client, &profile).await;
    assert!(result.is_ok(), "profile publish failed: {result:?}");

    client.disconnect().await;
}

// ── DM round-trip ───────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn send_and_receive_dm() {
    let sender_key = match sender_secret() {
        Some(k) => k,
        None => {
            println!("NOSTR_TEST_SENDER_KEY not set — skipping round-trip test");
            return;
        },
    };

    // Set up bot (receiver) — get notifications receiver BEFORE connect
    let bot_keys = moltis_nostr::keys::derive_keys(&bot_secret()).unwrap();
    let bot_pubkey = bot_keys.public_key();
    let bot_client = Client::new(bot_keys.clone());
    let mut notifications = bot_client.notifications();
    for relay in DEFAULT_RELAYS {
        let _ = bot_client.add_relay(*relay).await;
    }
    bot_client.connect().await;

    // Set up sender
    let sender_keys = moltis_nostr::keys::derive_keys(&sender_key).unwrap();
    let sender_client = Client::new(sender_keys.clone());
    for relay in DEFAULT_RELAYS {
        let _ = sender_client.add_relay(*relay).await;
    }
    sender_client.connect().await;

    // Give relays time to establish both connections
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Subscribe bot to DMs
    let since = Timestamp::now();
    let filter = Filter::new()
        .kind(Kind::EncryptedDirectMessage)
        .pubkey(bot_pubkey)
        .since(since);
    bot_client.subscribe(filter, None).await.expect("subscribe");

    // Wait for subscription to propagate across relays
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Send DM from sender to bot
    let test_msg = format!("integration test {}", Timestamp::now().as_secs());
    let encrypted =
        nip04::encrypt(sender_keys.secret_key(), &bot_pubkey, &test_msg).expect("encrypt");
    let tag = Tag::public_key(bot_pubkey);
    let builder = EventBuilder::new(Kind::EncryptedDirectMessage, &encrypted).tag(tag);
    sender_client
        .send_event_builder(builder)
        .await
        .expect("send DM");

    println!("Sent DM: {test_msg}");

    // Wait for bot to receive — use Message variant for reliability (Event deduplicates)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut received = false;

    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            Ok(notification) = notifications.recv() => {
                match notification {
                    RelayPoolNotification::Event { event, .. } => {
                        if event.kind == Kind::EncryptedDirectMessage
                            && event.pubkey == sender_keys.public_key()
                        {
                            let decrypted = nip04::decrypt(
                                bot_keys.secret_key(),
                                &event.pubkey,
                                &event.content,
                            ).expect("decrypt");
                            println!("Received DM via Event: {decrypted}");
                            assert_eq!(decrypted, test_msg);
                            received = true;
                            break;
                        }
                    },
                    RelayPoolNotification::Message { message: RelayMessage::Event { event, .. }, .. }
                        if event.kind == Kind::EncryptedDirectMessage
                            && event.pubkey == sender_keys.public_key() =>
                    {
                        let decrypted = nip04::decrypt(
                            bot_keys.secret_key(),
                            &event.pubkey,
                            &event.content,
                        ).expect("decrypt via Message");
                        println!("Received DM via Message: {decrypted}");
                        assert_eq!(decrypted, test_msg);
                        received = true;
                        break;
                    },
                    _ => {},
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    assert!(received, "bot must receive the DM within 30 seconds");

    bot_client.disconnect().await;
    sender_client.disconnect().await;
}

// ── NIP-44 encrypt/decrypt round-trip ───────────────────────

#[test]
#[ignore]
fn nip44_encrypt_decrypt_round_trip() {
    let bot_keys = moltis_nostr::keys::derive_keys(&bot_secret()).unwrap();
    let sender_keys = match sender_secret() {
        Some(k) => moltis_nostr::keys::derive_keys(&k).unwrap(),
        None => Keys::generate(),
    };

    let plaintext = "NIP-44 test message";
    let encrypted = nip44::encrypt(
        sender_keys.secret_key(),
        &bot_keys.public_key(),
        plaintext,
        nip44::Version::V2,
    )
    .expect("NIP-44 encrypt");

    let decrypted = nip44::decrypt(bot_keys.secret_key(), &sender_keys.public_key(), &encrypted)
        .expect("NIP-44 decrypt");

    assert_eq!(decrypted, plaintext);
}
