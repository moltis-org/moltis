//! Nostr channel plugin — lifecycle, registration, and OTP provider.

use std::{collections::HashMap, sync::Arc};

use {
    async_trait::async_trait,
    moltis_channels::{
        ChannelEventSink, ChannelOtpProvider, ChannelOutbound, ChannelPlugin, ChannelStatus,
        ChannelStreamOutbound, Result as ChannelResult, config_view::ChannelConfigView,
        message_log::MessageLog, otp::OtpChallengeInfo, plugin::ChannelHealthSnapshot,
    },
    nostr_sdk::prelude::*,
    secrecy::ExposeSecret,
    serde_json::Value,
    tokio_util::sync::CancellationToken,
};

use crate::{
    bus,
    config::NostrAccountConfig,
    keys,
    outbound::{AccountStateMap, NostrOutbound},
    profile,
    state::AccountState,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{gauge, nostr as nostr_metrics};

/// Sentinel value used by `RedactedConfig` — must be detected on update to
/// avoid overwriting the real secret key with the redacted placeholder.
const REDACTED_SENTINEL: &str = "[REDACTED]";

/// Nostr channel plugin.
pub struct NostrPlugin {
    accounts: AccountStateMap,
    outbound: NostrOutbound,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
}

impl NostrPlugin {
    pub fn new() -> Self {
        let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
        Self {
            outbound: NostrOutbound {
                accounts: Arc::clone(&accounts),
            },
            accounts,
            message_log: None,
            event_sink: None,
        }
    }

    pub fn with_message_log(mut self, log: Arc<dyn MessageLog>) -> Self {
        self.message_log = Some(log);
        self
    }

    pub fn with_event_sink(mut self, sink: Arc<dyn ChannelEventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    /// List pending OTP challenges for a specific account.
    fn otp_challenges(&self, account_id: &str) -> Vec<OtpChallengeInfo> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|state| {
                let otp = state.otp.lock().unwrap_or_else(|e| e.into_inner());
                otp.list_pending()
            })
            .unwrap_or_default()
    }
}

impl Default for NostrPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelPlugin for NostrPlugin {
    fn id(&self) -> &str {
        "nostr"
    }

    fn name(&self) -> &str {
        "Nostr"
    }

    async fn start_account(&mut self, account_id: &str, config: Value) -> ChannelResult<()> {
        let nostr_config: NostrAccountConfig = serde_json::from_value(config).map_err(|e| {
            moltis_channels::Error::invalid_input(format!("invalid nostr config: {e}"))
        })?;

        if !nostr_config.enabled {
            tracing::info!(account_id, "nostr account disabled, skipping");
            return Ok(());
        }

        // Parse keys
        let bot_keys = keys::derive_keys(&nostr_config.secret_key).map_err(|e| {
            moltis_channels::Error::invalid_input(format!("invalid secret key: {e}"))
        })?;

        let npub = bot_keys
            .public_key()
            .to_bech32()
            .unwrap_or_else(|_| bot_keys.public_key().to_hex());
        tracing::info!(
            account_id,
            pubkey = %npub,
            relays = ?nostr_config.relays,
            "starting Nostr account"
        );

        // Build nostr-sdk client
        let client = Client::new(bot_keys.clone());

        // Add relays
        for relay_url in &nostr_config.relays {
            if let Err(e) = client.add_relay(relay_url).await {
                tracing::warn!(account_id, relay = relay_url, "failed to add relay: {e}");
            }
        }

        // Connect to relays
        client.connect().await;

        // Publish profile if configured
        if let Some(ref prof) = nostr_config.profile
            && let Err(e) = profile::publish_profile(&client, prof).await
        {
            tracing::warn!(account_id, "failed to publish profile: {e}");
        }

        let cancel = CancellationToken::new();

        // Spawn subscription loop
        let event_sink = self
            .event_sink
            .clone()
            .ok_or_else(|| moltis_channels::Error::unavailable("event sink not configured"))?;

        // Pre-parse allowlist and create shared config — the bus loop and
        // update_account_config both use this same Arc so policy changes
        // take effect immediately.
        let cached_allowlist = Arc::new(std::sync::RwLock::new(keys::normalize_pubkeys(
            &nostr_config.allowed_pubkeys,
        )));
        let otp_cooldown = nostr_config.otp_cooldown_secs;
        let shared_config = Arc::new(std::sync::RwLock::new(nostr_config));
        let shared_otp = Arc::new(std::sync::Mutex::new(moltis_channels::otp::OtpState::new(
            otp_cooldown,
        )));

        let loop_client = client.clone();
        let loop_keys = bot_keys.clone();
        let loop_config = Arc::clone(&shared_config);
        let loop_allowlist = Arc::clone(&cached_allowlist);
        let loop_otp = Arc::clone(&shared_otp);
        let loop_account_id = account_id.to_string();
        let loop_cancel = cancel.clone();
        let loop_sink = Arc::clone(&event_sink);

        tokio::spawn(async move {
            bus::run_subscription_loop(
                loop_client,
                loop_keys,
                loop_config,
                loop_allowlist,
                loop_otp,
                loop_account_id,
                loop_sink,
                loop_cancel,
            )
            .await;
        });

        // Store account state
        let state = AccountState {
            client,
            keys: bot_keys,
            config: shared_config,
            cached_allowlist,
            cancel,
            otp: shared_otp,
        };

        self.accounts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(account_id.to_string(), state);

        #[cfg(feature = "metrics")]
        gauge!(nostr_metrics::ACTIVE_ACCOUNTS).set(
            self.accounts
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .len() as f64,
        );

        tracing::info!(account_id, "Nostr account started");
        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> ChannelResult<()> {
        let state = self
            .accounts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(account_id);
        if let Some(state) = state {
            state.cancel.cancel();
            state.client.disconnect().await;
            #[cfg(feature = "metrics")]
            gauge!(nostr_metrics::ACTIVE_ACCOUNTS).set(
                self.accounts
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .len() as f64,
            );
            tracing::info!(account_id, "Nostr account stopped");
        }
        Ok(())
    }

    fn outbound(&self) -> Option<&dyn ChannelOutbound> {
        Some(&self.outbound)
    }

    fn status(&self) -> Option<&dyn ChannelStatus> {
        Some(self)
    }

    fn has_account(&self, account_id: &str) -> bool {
        self.accounts
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(account_id)
    }

    fn account_ids(&self) -> Vec<String> {
        self.accounts
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    fn account_config(&self, account_id: &str) -> Option<Box<dyn ChannelConfigView>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.get(account_id).map(|s| {
            let cfg = s.config.read().unwrap_or_else(|e| e.into_inner());
            Box::new(cfg.clone()) as Box<dyn ChannelConfigView>
        })
    }

    fn account_config_json(&self, account_id: &str) -> Option<Value> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.get(account_id).and_then(|s| {
            let cfg = s.config.read().unwrap_or_else(|e| e.into_inner());
            serde_json::to_value(crate::config::RedactedConfig(&cfg)).ok()
        })
    }

    fn update_account_config(&self, account_id: &str, config: Value) -> ChannelResult<()> {
        let mut new_config: NostrAccountConfig = serde_json::from_value(config).map_err(|e| {
            moltis_channels::Error::invalid_input(format!("invalid nostr config: {e}"))
        })?;

        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get(account_id) {
            // Merge guard: if the incoming secret_key is the redacted sentinel,
            // preserve the existing key instead of corrupting it.
            if new_config.secret_key.expose_secret() == REDACTED_SENTINEL {
                let existing = state.config.read().unwrap_or_else(|e| e.into_inner());
                new_config.secret_key = existing.secret_key.clone();
            }

            // Update shared config — the bus loop sees this immediately.
            let mut cfg = state.config.write().unwrap_or_else(|e| e.into_inner());
            *cfg = new_config.clone();
            drop(cfg);

            // Refresh cached allowlist so access control uses the new list
            // without re-parsing on every DM.
            let mut al = state
                .cached_allowlist
                .write()
                .unwrap_or_else(|e| e.into_inner());
            *al = keys::normalize_pubkeys(&new_config.allowed_pubkeys);
        }
        Ok(())
    }

    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(NostrOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(NostrOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn as_otp_provider(&self) -> Option<&dyn ChannelOtpProvider> {
        Some(self)
    }
}

#[async_trait]
impl ChannelStatus for NostrPlugin {
    async fn probe(&self, account_id: &str) -> ChannelResult<ChannelHealthSnapshot> {
        // Clone client + keys out of the std RwLock guard before any .await
        // (std guards are not Send).
        let snapshot = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts
                .get(account_id)
                .map(|s| (s.client.clone(), s.keys.clone()))
        };

        match snapshot {
            Some((client, keys)) => {
                let relays = client.relays().await;
                let connected_count = relays
                    .values()
                    .filter(|r| r.status() == RelayStatus::Connected)
                    .count();
                let total = relays.len();

                let npub = keys
                    .public_key()
                    .to_bech32()
                    .unwrap_or_else(|_| keys.public_key().to_hex());
                Ok(ChannelHealthSnapshot {
                    connected: connected_count > 0,
                    account_id: account_id.to_string(),
                    details: Some(format!("{connected_count}/{total} relays connected")),
                    extra: Some(serde_json::json!({
                        "pubkey": npub,
                        "connected_relays": connected_count,
                        "total_relays": total,
                    })),
                })
            },
            None => Ok(ChannelHealthSnapshot {
                connected: false,
                account_id: account_id.to_string(),
                details: Some("account not found".to_string()),
                extra: None,
            }),
        }
    }
}

impl ChannelOtpProvider for NostrPlugin {
    fn pending_otp_challenges(&self, account_id: &str) -> Vec<OtpChallengeInfo> {
        self.otp_challenges(account_id)
    }
}

#[cfg(test)]
mod tests {
    use {super::*, moltis_channels::ChannelPlugin, nostr_sdk::prelude::Keys};

    /// Build a minimal `NostrPlugin` with one dummy account inserted.
    ///
    /// The account has no relay connections — only enough state to exercise
    /// the sync config accessors without touching the network.
    fn plugin_with_dummy_account() -> NostrPlugin {
        let keys = Keys::generate();
        let client = Client::new(keys.clone());
        let config = NostrAccountConfig::default();
        let cached_allowlist = Arc::new(std::sync::RwLock::new(keys::normalize_pubkeys(
            &config.allowed_pubkeys,
        )));
        let shared_config = Arc::new(std::sync::RwLock::new(config));
        let otp = Arc::new(std::sync::Mutex::new(moltis_channels::otp::OtpState::new(
            300,
        )));
        let cancel = CancellationToken::new();

        let state = AccountState {
            client,
            keys,
            config: shared_config,
            cached_allowlist,
            cancel,
            otp,
        };

        let plugin = NostrPlugin::new();
        plugin
            .accounts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert("test".to_string(), state);
        plugin
    }

    /// Regression test for <https://github.com/moltis-org/moltis/issues/736>.
    ///
    /// `account_config_json` is a sync trait method that must be callable from
    /// inside a tokio runtime. With `tokio::sync::RwLock` + `blocking_read()`
    /// this panics; with `std::sync::RwLock` it works.
    #[tokio::test]
    async fn account_config_json_does_not_panic_in_async() {
        let plugin = plugin_with_dummy_account();
        let json = plugin.account_config_json("test");
        assert!(json.is_some(), "should return config for existing account");
    }

    /// Same regression test for `update_account_config`.
    #[tokio::test]
    async fn update_account_config_does_not_panic_in_async() {
        let plugin = plugin_with_dummy_account();
        let new_config =
            serde_json::to_value(NostrAccountConfig::default()).expect("serialize default config");
        let result = plugin.update_account_config("test", new_config);
        assert!(result.is_ok(), "update_account_config must not panic");
    }

    /// Same regression test for `account_config`.
    #[tokio::test]
    async fn account_config_does_not_panic_in_async() {
        let plugin = plugin_with_dummy_account();
        let view = plugin.account_config("test");
        assert!(
            view.is_some(),
            "should return config view for existing account"
        );
    }
}
