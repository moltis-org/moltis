use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Instant,
};

use {
    anyhow::Result,
    async_trait::async_trait,
    secrecy::ExposeSecret,
    teloxide::prelude::Requester,
    tracing::{info, warn},
};

use moltis_channels::{
    ChannelEventSink,
    message_log::MessageLog,
    plugin::{ChannelHealthSnapshot, ChannelOutbound, ChannelPlugin, ChannelStatus},
};

use crate::{
    bot, config::TelegramAccountConfig, outbound::TelegramOutbound, state::AccountStateMap,
};

/// Cache TTL for probe results (30 seconds).
const PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Telegram channel plugin.
pub struct TelegramPlugin {
    accounts: AccountStateMap,
    outbound: TelegramOutbound,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
    probe_cache: RwLock<HashMap<String, (ChannelHealthSnapshot, Instant)>>,
}

impl TelegramPlugin {
    pub fn new() -> Self {
        let accounts: AccountStateMap = Arc::new(RwLock::new(HashMap::new()));
        let outbound = TelegramOutbound {
            accounts: Arc::clone(&accounts),
        };
        Self {
            accounts,
            outbound,
            message_log: None,
            event_sink: None,
            probe_cache: RwLock::new(HashMap::new()),
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

    /// Get a shared reference to the outbound sender (for use outside the plugin).
    pub fn shared_outbound(&self) -> Arc<dyn moltis_channels::ChannelOutbound> {
        Arc::new(TelegramOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    /// List all active account IDs.
    pub fn account_ids(&self) -> Vec<String> {
        let accounts = self.accounts.read().unwrap();
        accounts.keys().cloned().collect()
    }

    /// Get the config for a specific account (serialized to JSON).
    pub fn account_config(&self, account_id: &str) -> Option<serde_json::Value> {
        let accounts = self.accounts.read().unwrap();
        accounts
            .get(account_id)
            .and_then(|s| serde_json::to_value(&s.config).ok())
    }

    /// Update the in-memory config for an account without restarting the
    /// polling loop.  Use for allowlist changes that don't need
    /// re-authentication or bot restart.
    pub fn update_account_config(&self, account_id: &str, config: serde_json::Value) -> Result<()> {
        let tg_config: TelegramAccountConfig = serde_json::from_value(config)?;
        let mut accounts = self.accounts.write().unwrap();
        if let Some(state) = accounts.get_mut(account_id) {
            state.config = tg_config;
            Ok(())
        } else {
            Err(anyhow::anyhow!("account not found: {account_id}"))
        }
    }

    /// List pending OTP challenges for a specific account.
    pub fn pending_otp_challenges(&self, account_id: &str) -> Vec<crate::otp::OtpChallengeInfo> {
        let accounts = self.accounts.read().unwrap();
        accounts
            .get(account_id)
            .map(|s| {
                let otp = s.otp.lock().unwrap();
                otp.list_pending()
            })
            .unwrap_or_default()
    }
}

impl Default for TelegramPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelPlugin for TelegramPlugin {
    fn id(&self) -> &str {
        "telegram"
    }

    fn name(&self) -> &str {
        "Telegram"
    }

    async fn start_account(&mut self, account_id: &str, config: serde_json::Value) -> Result<()> {
        let tg_config: TelegramAccountConfig = serde_json::from_value(config)?;

        if tg_config.token.expose_secret().is_empty() {
            return Err(anyhow::anyhow!("telegram bot token is required"));
        }

        info!(account_id, "starting telegram account");

        bot::start_polling(
            account_id.to_string(),
            tg_config,
            Arc::clone(&self.accounts),
            self.message_log.clone(),
            self.event_sink.clone(),
        )
        .await?;

        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> Result<()> {
        let cancel = {
            let accounts = self.accounts.read().unwrap();
            accounts.get(account_id).map(|s| s.cancel.clone())
        };

        if let Some(cancel) = cancel {
            info!(account_id, "stopping telegram account");
            cancel.cancel();
            let mut accounts = self.accounts.write().unwrap();
            accounts.remove(account_id);
        } else {
            warn!(account_id, "telegram account not found");
        }

        Ok(())
    }

    fn outbound(&self) -> Option<&dyn ChannelOutbound> {
        Some(&self.outbound)
    }

    fn status(&self) -> Option<&dyn ChannelStatus> {
        Some(self)
    }
}

#[async_trait]
impl ChannelStatus for TelegramPlugin {
    async fn probe(&self, account_id: &str) -> Result<ChannelHealthSnapshot> {
        // Return cached result if fresh enough.
        if let Ok(cache) = self.probe_cache.read()
            && let Some((snap, ts)) = cache.get(account_id)
            && ts.elapsed() < PROBE_CACHE_TTL
        {
            return Ok(snap.clone());
        }

        let bot = {
            let accounts = self.accounts.read().unwrap();
            accounts.get(account_id).map(|s| s.bot.clone())
        };

        let result = match bot {
            Some(bot) => match bot.get_me().await {
                Ok(me) => ChannelHealthSnapshot {
                    connected: true,
                    account_id: account_id.to_string(),
                    details: Some(format!(
                        "Bot: @{}",
                        me.username.as_deref().unwrap_or("unknown")
                    )),
                },
                Err(e) => ChannelHealthSnapshot {
                    connected: false,
                    account_id: account_id.to_string(),
                    details: Some(format!("API error: {e}")),
                },
            },
            None => ChannelHealthSnapshot {
                connected: false,
                account_id: account_id.to_string(),
                details: Some("account not started".into()),
            },
        };

        if let Ok(mut cache) = self.probe_cache.write() {
            cache.insert(account_id.to_string(), (result.clone(), Instant::now()));
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{otp::OtpState, outbound::TelegramOutbound, state::AccountState},
        moltis_channels::gating::DmPolicy,
        secrecy::{ExposeSecret, Secret},
        tokio_util::sync::CancellationToken,
    };

    /// Build a minimal `AccountState` for unit tests (no network calls).
    fn test_account_state(accounts: &AccountStateMap, cancel: CancellationToken) -> AccountState {
        AccountState {
            bot: teloxide::Bot::new("test:fake_token_for_unit_tests"),
            bot_username: Some("test_bot".into()),
            account_id: "test".into(),
            config: TelegramAccountConfig {
                token: Secret::new("test:fake_token_for_unit_tests".into()),
                ..Default::default()
            },
            outbound: Arc::new(TelegramOutbound {
                accounts: Arc::clone(accounts),
            }),
            cancel,
            message_log: None,
            event_sink: None,
            otp: std::sync::Mutex::new(OtpState::new(300)),
        }
    }

    #[test]
    fn update_account_config_updates_allowlist() {
        let plugin = TelegramPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap();
            map.insert("test".into(), test_account_state(&plugin.accounts, cancel));
        }

        // Initially empty allowlist.
        {
            let map = plugin.accounts.read().unwrap();
            assert!(map.get("test").unwrap().config.allowlist.is_empty());
        }

        // Update config with a populated allowlist.
        let new_config = serde_json::json!({
            "token": "test:fake_token_for_unit_tests",
            "dm_policy": "allowlist",
            "allowlist": ["alice", "bob"],
        });
        plugin.update_account_config("test", new_config).unwrap();

        // Verify the change is immediately visible.
        let map = plugin.accounts.read().unwrap();
        let state = map.get("test").unwrap();
        assert_eq!(state.config.dm_policy, DmPolicy::Allowlist);
        assert_eq!(state.config.allowlist, vec!["alice", "bob"]);
    }

    /// Security: `update_account_config` must NOT cancel the polling
    /// CancellationToken.  Cancelling it restarts the bot polling loop with
    /// offset 0, causing Telegram to re-deliver the OTP code message.  The
    /// re-delivered message would pass access control (user is now on the
    /// allowlist) and get forwarded to the LLM.
    #[test]
    fn security_update_config_does_not_cancel_polling() {
        let plugin = TelegramPlugin::new();
        let cancel = CancellationToken::new();
        let cancel_witness = cancel.clone();

        {
            let mut map = plugin.accounts.write().unwrap();
            map.insert("test".into(), test_account_state(&plugin.accounts, cancel));
        }

        let new_config = serde_json::json!({
            "token": "test:fake_token_for_unit_tests",
            "allowlist": ["new_user"],
        });
        plugin.update_account_config("test", new_config).unwrap();

        assert!(
            !cancel_witness.is_cancelled(),
            "update_account_config must NOT cancel the polling token — \
             cancelling restarts the bot and causes Telegram to re-deliver messages"
        );
    }

    /// Security: after a hot config update, the access control check must
    /// immediately reflect the new allowlist.  This simulates the exact
    /// sequence that happens during OTP self-approval.
    #[test]
    fn security_config_update_immediately_affects_access_control() {
        use {crate::access, moltis_common::types::ChatType};

        let plugin = TelegramPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap();
            let mut state = test_account_state(&plugin.accounts, cancel);
            state.config.dm_policy = DmPolicy::Allowlist;
            state.config.allowlist = vec![];
            map.insert("test".into(), state);
        }

        // Before approval: user is denied.
        {
            let map = plugin.accounts.read().unwrap();
            let config = &map.get("test").unwrap().config;
            assert!(
                access::check_access(config, &ChatType::Dm, "12345", Some("alice"), None, false)
                    .is_err()
            );
        }

        // OTP approval adds user to allowlist via update_account_config.
        let new_config = serde_json::json!({
            "token": "test:fake_token_for_unit_tests",
            "dm_policy": "allowlist",
            "allowlist": ["alice"],
        });
        plugin.update_account_config("test", new_config).unwrap();

        // After approval: user is allowed.
        {
            let map = plugin.accounts.read().unwrap();
            let config = &map.get("test").unwrap().config;
            assert!(
                access::check_access(config, &ChatType::Dm, "12345", Some("alice"), None, false)
                    .is_ok(),
                "approved user must pass access control immediately after config update"
            );
        }
    }

    #[test]
    fn update_account_config_nonexistent_account_errors() {
        let plugin = TelegramPlugin::new();
        let result = plugin.update_account_config("nonexistent", serde_json::json!({"token": "t"}));
        assert!(result.is_err());
    }

    #[test]
    fn update_account_config_preserves_otp_state() {
        let plugin = TelegramPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap();
            map.insert("test".into(), test_account_state(&plugin.accounts, cancel));
        }

        // Create a pending OTP challenge.
        {
            let map = plugin.accounts.read().unwrap();
            let state = map.get("test").unwrap();
            let mut otp = state.otp.lock().unwrap();
            otp.initiate("12345", Some("alice".into()), None);
        }

        // Update config.
        let new_config = serde_json::json!({
            "token": "test:fake_token_for_unit_tests",
            "allowlist": ["alice"],
        });
        plugin.update_account_config("test", new_config).unwrap();

        // OTP challenge must still be pending (state was not wiped).
        let map = plugin.accounts.read().unwrap();
        let state = map.get("test").unwrap();
        let otp = state.otp.lock().unwrap();
        assert!(
            otp.has_pending("12345"),
            "config update must preserve in-flight OTP challenges"
        );
    }

    #[test]
    fn update_account_config_preserves_bot_token() {
        let plugin = TelegramPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap();
            map.insert("test".into(), test_account_state(&plugin.accounts, cancel));
        }

        // Update config with a new allowlist but same token.
        let new_config = serde_json::json!({
            "token": "test:fake_token_for_unit_tests",
            "allowlist": ["alice"],
        });
        plugin.update_account_config("test", new_config).unwrap();

        // Bot instance itself is untouched (same object in memory).
        let map = plugin.accounts.read().unwrap();
        let state = map.get("test").unwrap();
        assert_eq!(
            state.config.token.expose_secret(),
            "test:fake_token_for_unit_tests"
        );
    }
}
