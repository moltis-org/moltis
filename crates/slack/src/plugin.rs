use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use {
    async_trait::async_trait,
    secrecy::ExposeSecret,
    tracing::{info, warn},
};

use moltis_channels::{
    ChannelConfigView, Error as ChannelError, Result as ChannelResult,
    message_log::MessageLog,
    plugin::{
        ChannelEventSink, ChannelHealthSnapshot, ChannelOutbound, ChannelPlugin, ChannelStatus,
        ChannelStreamOutbound, ChannelThreadContext,
    },
};

use crate::{config::SlackAccountConfig, outbound::SlackOutbound, state::AccountStateMap};

/// Slack channel plugin.
pub struct SlackPlugin {
    accounts: AccountStateMap,
    outbound: SlackOutbound,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
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

    /// Ingest an Events API webhook request.
    ///
    /// Returns `Ok(Some(challenge))` for URL verification, `Ok(None)` for events.
    pub async fn ingest_webhook(
        &self,
        account_id: &str,
        body: &[u8],
        timestamp: &str,
        signature: &str,
    ) -> ChannelResult<Option<String>> {
        crate::webhook::handle_webhook(account_id, body, timestamp, signature, &self.accounts).await
    }

    /// Ingest an interaction webhook (button clicks).
    pub async fn ingest_interaction_webhook(
        &self,
        account_id: &str,
        body: &[u8],
        timestamp: &str,
        signature: &str,
    ) -> ChannelResult<()> {
        crate::webhook::handle_interaction_webhook(
            account_id,
            body,
            timestamp,
            signature,
            &self.accounts,
        )
        .await
    }

    /// Ingest an already-verified Events API webhook request.
    ///
    /// Use this when the caller has already verified the signature via
    /// the channel webhook middleware pipeline.
    pub async fn ingest_verified_webhook(
        &self,
        account_id: &str,
        body: &[u8],
    ) -> ChannelResult<Option<String>> {
        crate::webhook::handle_verified_webhook(account_id, body, &self.accounts).await
    }

    /// Ingest an already-verified interaction webhook.
    pub async fn ingest_verified_interaction_webhook(
        &self,
        account_id: &str,
        body: &[u8],
    ) -> ChannelResult<()> {
        crate::webhook::handle_verified_interaction_webhook(account_id, body, &self.accounts).await
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
        let cfg: SlackAccountConfig = serde_json::from_value(config)?;
        if cfg.bot_token.expose_secret().is_empty() {
            return Err(ChannelError::invalid_input("Slack bot_token is required"));
        }

        match cfg.connection_mode {
            crate::config::ConnectionMode::SocketMode => {
                if cfg.app_token.expose_secret().is_empty() {
                    return Err(ChannelError::invalid_input(
                        "Slack app_token is required for Socket Mode",
                    ));
                }
                info!(account_id, "starting slack account (socket mode)");
                crate::socket::start_socket_mode(
                    account_id,
                    cfg,
                    Arc::clone(&self.accounts),
                    self.message_log.clone(),
                    self.event_sink.clone(),
                )
                .await
            },
            crate::config::ConnectionMode::EventsApi => {
                if cfg
                    .signing_secret
                    .as_ref()
                    .is_none_or(|s| s.expose_secret().is_empty())
                {
                    return Err(ChannelError::invalid_input(
                        "Slack signing_secret is required for Events API mode",
                    ));
                }
                info!(account_id, "starting slack account (events api)");
                crate::webhook::register_events_api_account(
                    account_id,
                    cfg,
                    Arc::clone(&self.accounts),
                    self.message_log.clone(),
                    self.event_sink.clone(),
                )
                .await
            },
        }
    }

    async fn stop_account(&mut self, account_id: &str) -> ChannelResult<()> {
        let cancel = {
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            accounts.remove(account_id).map(|s| s.cancel)
        };
        if let Some(cancel) = cancel {
            cancel.cancel();
            info!(account_id, "stopped slack account");
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

    fn has_account(&self, account_id: &str) -> bool {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.contains_key(account_id)
    }

    fn account_ids(&self) -> Vec<String> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.keys().cloned().collect()
    }

    fn account_config(&self, account_id: &str) -> Option<Box<dyn ChannelConfigView>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| Box::new(s.config.clone()) as Box<dyn ChannelConfigView>)
    }

    fn account_config_json(&self, account_id: &str) -> Option<serde_json::Value> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .and_then(|s| serde_json::to_value(&s.config).ok())
    }

    fn update_account_config(
        &self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let parsed: SlackAccountConfig = serde_json::from_value(config)?;
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get_mut(account_id) {
            state.config = parsed;
            Ok(())
        } else {
            Err(ChannelError::unknown_account(account_id))
        }
    }

    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(SlackOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(SlackOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn thread_context(&self) -> Option<&dyn ChannelThreadContext> {
        Some(&self.outbound)
    }

    fn channel_webhook_verifier(
        &self,
        account_id: &str,
    ) -> Option<Box<dyn moltis_channels::channel_webhook_middleware::ChannelWebhookVerifier>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts.get(account_id)?;
        let secret = state.config.signing_secret.as_ref()?;
        Some(Box::new(
            crate::channel_webhook_verifier::SlackChannelWebhookVerifier::new(secret.clone()),
        ))
    }
}

#[async_trait]
impl ChannelStatus for SlackPlugin {
    async fn probe(&self, account_id: &str) -> ChannelResult<ChannelHealthSnapshot> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get(account_id) {
            let connected = state.bot_user_id.is_some();
            let details = if connected {
                "socket mode connected".to_string()
            } else {
                "connecting to Slack...".to_string()
            };
            Ok(ChannelHealthSnapshot {
                connected,
                account_id: state.account_id.clone(),
                details: Some(details),
            })
        } else {
            Ok(ChannelHealthSnapshot {
                connected: false,
                account_id: account_id.to_string(),
                details: Some("account not started".into()),
            })
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_and_name() {
        let plugin = SlackPlugin::new();
        assert_eq!(plugin.id(), "slack");
        assert_eq!(plugin.name(), "Slack");
    }

    #[test]
    fn empty_account_ids() {
        let plugin = SlackPlugin::new();
        assert!(plugin.account_ids().is_empty());
    }

    #[tokio::test]
    async fn start_rejects_empty_bot_token() {
        let mut plugin = SlackPlugin::new();
        let config = serde_json::json!({
            "bot_token": "",
            "app_token": "xapp-test",
        });
        let result = plugin.start_account("test", config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn start_rejects_empty_app_token() {
        let mut plugin = SlackPlugin::new();
        let config = serde_json::json!({
            "bot_token": "xoxb-test",
            "app_token": "",
        });
        let result = plugin.start_account("test", config).await;
        assert!(result.is_err());
    }

    #[test]
    fn update_config_unknown_account_errors() {
        let plugin = SlackPlugin::new();
        let result = plugin.update_account_config(
            "nope",
            serde_json::json!({
                "bot_token": "xoxb-test",
                "app_token": "xapp-test",
            }),
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn start_events_api_rejects_missing_signing_secret() {
        let mut plugin = SlackPlugin::new();
        let config = serde_json::json!({
            "bot_token": "xoxb-test",
            "app_token": "",
            "connection_mode": "events_api",
        });
        let result = plugin.start_account("test", config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("signing_secret"), "error: {err}");
    }

    #[tokio::test]
    async fn probe_no_account() {
        let plugin = SlackPlugin::new();
        let snap = plugin.probe("missing").await.unwrap();
        assert!(!snap.connected);
        assert_eq!(snap.details.as_deref(), Some("account not started"));
    }

    #[test]
    fn descriptor_coherence() {
        use moltis_channels::{ChannelType, InboundMode};
        let plugin = SlackPlugin::new();
        let desc = ChannelType::Slack.descriptor();

        assert_eq!(desc.channel_type, ChannelType::Slack);
        assert_eq!(desc.display_name, "Slack");
        assert_eq!(desc.capabilities.inbound_mode, InboundMode::SocketMode);

        // Threads: Slack implements ChannelThreadContext
        assert!(desc.capabilities.supports_threads);
        assert!(plugin.thread_context().is_some());

        // OTP: Slack does NOT implement ChannelOtpProvider
        assert!(!desc.capabilities.supports_otp);
        assert!(plugin.as_otp_provider().is_none());

        // Reactions: Slack supports reactions
        assert!(desc.capabilities.supports_reactions);

        // Interactive: Slack supports interactive messages
        assert!(desc.capabilities.supports_interactive);
    }
}
