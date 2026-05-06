//! NIP-59 Gift Wrap helpers for sending and receiving private DMs.
//!
//! Wraps messages with ephemeral keys (NIP-17/NIP-59), fully hiding
//! sender and receiver metadata — unlike legacy NIP-04 (kind:4).

use nostr_sdk::prelude::{Event, EventBuilder, Keys, Kind, PublicKey, Timestamp, nip59};

use crate::error::Error;

/// Gift wrap timestamps are randomly tweaked by 0–2 days (per NIP-59), so
/// relay filters need a wider `since` window to avoid missing recent wraps.
pub const TIMESTAMP_WINDOW_SECS: u64 = ::time::Duration::days(2).whole_seconds() as u64;

/// Send a gift-wrapped DM (NIP-17/NIP-59, kind:1059) to the recipient.
///
/// Uses `EventBuilder::private_msg` to create a sealed, gift-wrapped event
/// and publishes it to connected relays.
pub async fn send_gift_wrapped_dm(
    client: &nostr_sdk::Client,
    keys: &Keys,
    recipient: &PublicKey,
    text: &str,
) -> Result<(), Error> {
    let event = EventBuilder::private_msg(keys, *recipient, text, [])
        .await
        .map_err(|e| Error::Encryption(format!("gift wrap failed: {e}")))?;

    client.send_event(&event).await?;
    Ok(())
}

/// Unwrap a gift-wrapped event (kind:1059) and return the sender, plaintext,
/// and the rumor's `created_at` timestamp.
///
/// The outer event's timestamp is randomly tweaked (0–2 days), so callers
/// should use the returned timestamp for staleness checks.
pub async fn unwrap_gift_wrap(
    keys: &Keys,
    event: &Event,
) -> Result<(PublicKey, String, Timestamp), Error> {
    let unwrapped = nip59::extract_rumor(keys, event)
        .await
        .map_err(|e| Error::Encryption(format!("gift unwrap failed: {e}")))?;

    if unwrapped.rumor.kind != Kind::PrivateDirectMessage {
        return Err(Error::Encryption(format!(
            "expected kind:14 (PrivateDirectMessage), got kind:{}",
            unwrapped.rumor.kind.as_u16()
        )));
    }

    Ok((
        unwrapped.sender,
        unwrapped.rumor.content,
        unwrapped.rumor.created_at,
    ))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn gift_wrap_round_trip() {
        let sender_keys = Keys::generate();
        let receiver_keys = Keys::generate();

        let text = "hello, gift-wrapped world!";
        let event = EventBuilder::private_msg(&sender_keys, receiver_keys.public_key(), text, [])
            .await
            .expect("create gift wrap");

        assert_eq!(event.kind, Kind::GiftWrap);

        let (sender, content, _ts) = unwrap_gift_wrap(&receiver_keys, &event)
            .await
            .expect("unwrap gift wrap");

        assert_eq!(sender, sender_keys.public_key());
        assert_eq!(content, text);
    }

    #[tokio::test]
    async fn wrong_recipient_fails() {
        let sender_keys = Keys::generate();
        let receiver_keys = Keys::generate();
        let wrong_keys = Keys::generate();

        let event = EventBuilder::private_msg(
            &sender_keys,
            receiver_keys.public_key(),
            "secret message",
            [],
        )
        .await
        .expect("create gift wrap");

        let result = unwrap_gift_wrap(&wrong_keys, &event).await;
        assert!(result.is_err(), "unwrap with wrong keys must fail");
    }

    #[tokio::test]
    async fn non_gift_wrap_rejected() {
        let keys = Keys::generate();
        let event = EventBuilder::text_note("not a gift wrap")
            .sign(&keys)
            .await
            .expect("sign event");

        let result = unwrap_gift_wrap(&keys, &event).await;
        assert!(result.is_err(), "non-gift-wrap event must be rejected");
    }
}
