//! Per-account runtime state for Nostr.

use std::sync::{Arc, Mutex, RwLock};

use {
    moltis_channels::otp::OtpState,
    nostr_sdk::prelude::{Client, Keys, PublicKey, ToBech32},
    tokio_util::sync::CancellationToken,
};

use crate::{config::NostrAccountConfig, reply_ctx::ReplyContexts};

/// Shared config reference — the bus loop and plugin both read/write through
/// this same `Arc` so runtime config updates (DM policy, allowlist) take
/// effect immediately without restarting the account.
///
/// Deliberately a `tokio::sync::RwLock`, not a `std::sync::RwLock`: its guards
/// are held across `.await`, which is what makes group authorization atomic
/// with respect to publishing. The outbound path takes a read guard, checks
/// that the group is still permitted, and keeps the guard until the event has
/// been handed to the relay — so a settings save that withdraws the group waits
/// for that publish rather than slipping in behind the check. See
/// `NostrOutbound::authorized_group_publish`.
///
/// This is why `ChannelPlugin`'s config accessors are async: a synchronous
/// writer could not take this lock (`blocking_write` panics inside a runtime).
pub type SharedConfig = Arc<tokio::sync::RwLock<NostrAccountConfig>>;

/// Shared OTP state — bus loop initiates challenges, plugin reads pending list.
pub type SharedOtp = Arc<Mutex<OtpState>>;

/// Shared group reply context — the bus loop records inbound group messages,
/// the outbound adapter reads them to mirror the message kind and `p`-tag the
/// author it is replying to.
pub type SharedReplyContexts = Arc<Mutex<ReplyContexts>>;

/// Runtime state for a single active Nostr account.
pub struct AccountState {
    /// The nostr-sdk client connected to relays.
    pub client: Client,
    /// Bot key pair (secret + public).
    pub keys: Keys,
    /// Shared account configuration — same Arc given to the bus loop.
    pub config: SharedConfig,
    /// Pre-parsed allowlist pubkeys, refreshed on config update.
    pub cached_allowlist: Arc<RwLock<Vec<PublicKey>>>,
    /// Cancellation token for the subscription loop.
    pub cancel: CancellationToken,
    /// OTP self-approval state — shared with bus loop.
    pub otp: SharedOtp,
    /// Group reply context — shared with bus loop.
    pub reply_ctx: SharedReplyContexts,
}

impl std::fmt::Debug for AccountState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pk = self
            .keys
            .public_key()
            .to_bech32()
            .unwrap_or_else(|_| self.keys.public_key().to_hex());
        f.debug_struct("AccountState")
            .field("pubkey", &pk)
            .finish_non_exhaustive()
    }
}
