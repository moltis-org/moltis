use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Instant,
};

use {
    async_trait::async_trait,
    secrecy::ExposeSecret,
    tracing::{info, warn},
};

use moltis_channels::{
    ChannelEventSink, Error as ChannelError, Result as ChannelResult,
    message_log::MessageLog,
    plugin::{ChannelHealthSnapshot, ChannelOutbound, ChannelPlugin, ChannelStatus},
};

use crate::{config::SlackAccountConfig, outbound::SlackOutbound, socket, state::AccountStateMap};

/// Cache TTL for probe results (30 seconds).
const PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Slack channel plugin.
pub struct SlackPlugin {
    accounts: AccountStateMap,
    outbound: SlackOutbound,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
    probe_cache: RwLock<HashMap<String, (ChannelHealthSnapshot, Instant)>>,
}

impl SlackPlugin {
    pub fn new() -> Self {
        let accounts: AccountStateMap = Arc::new(RwLock::new(HashMap::new()));
        let outbound = SlackOutbound {
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
    pub fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(SlackOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    /// List all active account IDs.
    pub fn account_ids(&self) -> Vec<String> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.keys().cloned().collect()
    }

    /// Get the config for a specific account (serialized to JSON).
    pub fn account_config(&self, account_id: &str) -> Option<serde_json::Value> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .and_then(|s| serde_json::to_value(&s.config).ok())
    }
}

impl Default for SlackPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelPlugin for SlackPlugin {
    fn id(&self) -> &str {
        "slack"
    }

    fn name(&self) -> &str {
        "Slack"
    }

    async fn start_account(
        &mut self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let slack_config: SlackAccountConfig = serde_json::from_value(config)?;

        if slack_config.bot_token.expose_secret().is_empty() {
            return Err(ChannelError::invalid_input("slack bot token is required"));
        }

        if slack_config.app_token.expose_secret().is_empty() {
            return Err(ChannelError::invalid_input(
                "slack app token is required for socket mode",
            ));
        }

        info!(account_id, "starting slack account");

        socket::start_socket_mode(
            account_id.to_string(),
            slack_config,
            Arc::clone(&self.accounts),
            self.message_log.clone(),
            self.event_sink.clone(),
        )
        .await?;

        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> ChannelResult<()> {
        let cancel = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get(account_id).map(|s| s.cancel.clone())
        };

        if let Some(cancel) = cancel {
            info!(account_id, "stopping slack account");
            cancel.cancel();
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            accounts.remove(account_id);
        } else {
            warn!(account_id, "slack account not found");
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
impl ChannelStatus for SlackPlugin {
    async fn probe(&self, account_id: &str) -> ChannelResult<ChannelHealthSnapshot> {
        // Return cached result if fresh enough
        if let Ok(cache) = self.probe_cache.read()
            && let Some((snap, ts)) = cache.get(account_id)
            && ts.elapsed() < PROBE_CACHE_TTL
        {
            return Ok(snap.clone());
        }

        let (client, token) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            match accounts.get(account_id) {
                Some(state) => {
                    let token = slack_morphism::prelude::SlackApiToken::new(
                        state.config.bot_token.expose_secret().into(),
                    );
                    (Some(state.client.clone()), Some(token))
                },
                None => (None, None),
            }
        };

        let result = match (client, token) {
            (Some(client), Some(token)) => {
                let session = client.open_session(&token);
                match session.auth_test().await {
                    Ok(auth) => ChannelHealthSnapshot {
                        connected: true,
                        account_id: account_id.to_string(),
                        details: Some(format!(
                            "Team: {} ({})",
                            &auth.team,
                            auth.user.as_deref().unwrap_or("unknown")
                        )),
                    },
                    Err(e) => ChannelHealthSnapshot {
                        connected: false,
                        account_id: account_id.to_string(),
                        details: Some(format!("API error: {e}")),
                    },
                }
            },
            _ => ChannelHealthSnapshot {
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
