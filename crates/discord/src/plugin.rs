use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Instant,
};

use {
    async_trait::async_trait,
    secrecy::ExposeSecret,
    serenity::all::{Client, Http},
    tracing::{error, info, warn},
};

use moltis_channels::{
    ChannelEventSink, Error as ChannelError, Result as ChannelResult,
    message_log::MessageLog,
    plugin::{ChannelHealthSnapshot, ChannelOutbound, ChannelPlugin, ChannelStatus},
};

use crate::{
    config::DiscordAccountConfig,
    handler::DiscordHandler,
    outbound::DiscordOutbound,
    state::{AccountState, AccountStateMap},
};

/// Cache TTL for probe results (30 seconds).
const PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Discord channel plugin.
pub struct DiscordPlugin {
    accounts: AccountStateMap,
    outbound: DiscordOutbound,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
    probe_cache: RwLock<HashMap<String, (ChannelHealthSnapshot, Instant)>>,
}

impl DiscordPlugin {
    pub fn new() -> Self {
        let accounts: AccountStateMap = Arc::new(RwLock::new(HashMap::new()));
        let outbound = DiscordOutbound {
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
        Arc::new(DiscordOutbound {
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

impl Default for DiscordPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelPlugin for DiscordPlugin {
    fn id(&self) -> &str {
        "discord"
    }

    fn name(&self) -> &str {
        "Discord"
    }

    async fn start_account(
        &mut self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let discord_config: DiscordAccountConfig = serde_json::from_value(config)?;

        if discord_config.token.expose_secret().is_empty() {
            return Err(ChannelError::invalid_input("discord bot token is required"));
        }

        info!(account_id, "starting discord account");

        let token = discord_config.token.expose_secret().clone();
        let accounts = Arc::clone(&self.accounts);
        let account_id_owned = account_id.to_string();
        let config_clone = discord_config.clone();
        let message_log = self.message_log.clone();
        let event_sink = self.event_sink.clone();

        // Create cancellation token
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_clone = cancel.clone();

        // Create outbound sender
        let outbound = Arc::new(DiscordOutbound {
            accounts: Arc::clone(&accounts),
        });

        // Create HTTP client for initial setup
        let http = Arc::new(Http::new(&token));

        // Store initial state (will be updated when ready event fires)
        {
            let mut accts = accounts.write().unwrap_or_else(|e| e.into_inner());
            accts.insert(account_id.to_string(), AccountState {
                http: Arc::clone(&http),
                bot_user_id: None,
                account_id: account_id.to_string(),
                config: discord_config,
                outbound,
                cancel: cancel.clone(),
                message_log: message_log.clone(),
                event_sink: event_sink.clone(),
                pending_replies: HashMap::new(),
            });
        }

        // Spawn Discord client
        tokio::spawn(async move {
            let handler = DiscordHandler {
                account_id: account_id_owned.clone(),
                config: config_clone,
                accounts: accounts.clone(),
                message_log,
                event_sink,
            };

            let intents = DiscordHandler::intents();

            let client_result = Client::builder(&token, intents)
                .event_handler(handler)
                .await;

            match client_result {
                Ok(mut client) => {
                    tokio::select! {
                        result = client.start() => {
                            if let Err(e) = result {
                                error!(
                                    account_id = %account_id_owned,
                                    error = %e,
                                    "discord client error"
                                );
                            }
                        }
                        _ = cancel_clone.cancelled() => {
                            info!(account_id = %account_id_owned, "discord client cancelled");
                        }
                    }
                },
                Err(e) => {
                    error!(
                        account_id = %account_id_owned,
                        error = %e,
                        "failed to create discord client"
                    );
                },
            }

            // Clean up on exit
            let mut accts = accounts.write().unwrap_or_else(|e| e.into_inner());
            accts.remove(&account_id_owned);
        });

        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> ChannelResult<()> {
        let cancel = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get(account_id).map(|s| s.cancel.clone())
        };

        if let Some(cancel) = cancel {
            info!(account_id, "stopping discord account");
            cancel.cancel();
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            accounts.remove(account_id);
        } else {
            warn!(account_id, "discord account not found");
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
impl ChannelStatus for DiscordPlugin {
    async fn probe(&self, account_id: &str) -> ChannelResult<ChannelHealthSnapshot> {
        // Return cached result if fresh enough
        if let Ok(cache) = self.probe_cache.read()
            && let Some((snap, ts)) = cache.get(account_id)
            && ts.elapsed() < PROBE_CACHE_TTL
        {
            return Ok(snap.clone());
        }

        let http = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get(account_id).map(|s| s.http.clone())
        };

        let result = match http {
            Some(http) => match http.get_current_user().await {
                Ok(user) => ChannelHealthSnapshot {
                    connected: true,
                    account_id: account_id.to_string(),
                    details: Some(format!("Bot: {} ({})", user.name, user.id)),
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
