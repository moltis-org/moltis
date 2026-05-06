//! Per-account runtime state for Nostr.

use std::sync::{Arc, Mutex, RwLock};

use {
    moltis_channels::otp::OtpState,
    nostr_sdk::prelude::{Client, Keys, PublicKey, ToBech32},
    tokio_util::sync::CancellationToken,
};

use crate::config::NostrAccountConfig;

/// Shared config reference — the bus loop and plugin both read/write through
/// this same `Arc` so runtime config updates (DM policy, allowlist) take
/// effect immediately without restarting the account.
///
/// Uses `std::sync::RwLock` (not `tokio::sync::RwLock`) because several
/// `ChannelPlugin` trait methods are synchronous and must be callable from
/// inside a tokio runtime without panicking.
pub type SharedConfig = Arc<RwLock<NostrAccountConfig>>;

/// Shared OTP state — bus loop initiates challenges, plugin reads pending list.
pub type SharedOtp = Arc<Mutex<OtpState>>;

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
