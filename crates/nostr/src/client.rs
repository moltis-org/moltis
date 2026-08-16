//! Construction of the `nostr-sdk` client used for every account.

use nostr_sdk::prelude::{Client, ClientOptions, Keys};

/// Build the relay client with both trust-relevant options pinned explicitly
/// rather than inherited from upstream defaults:
///
/// * `verify_subscriptions` (off by default) makes the SDK drop events that do
///   not match the filters we asked for. Relays are untrusted and can push
///   anything down the socket, and an inbound event is agent input — so this is
///   a prompt-injection boundary, not a tidiness knob. Group access is
///   re-checked in `bus` regardless.
/// * NIP-42 auto-authentication is already the upstream default, but Buzz relays
///   challenge every connection before granting NIP-29 group scopes, so state it
///   here to keep it from silently regressing.
///
/// Both settings decide what reaches the agent, so they are built here — and
/// exercised end to end against a relay in `tests/buzz_wire_format.rs` — rather
/// than inline where a subscription change could quietly break delivery.
#[must_use]
pub fn build_client(keys: Keys) -> Client {
    let opts = ClientOptions::new()
        .verify_subscriptions(true)
        .automatic_authentication(true);
    Client::builder().signer(keys).opts(opts).build()
}
