//! `ChannelPlugin` implementation for telephony.

use {
    async_trait::async_trait,
    moltis_channels::{
        ChannelEventSink,
        config_view::ChannelConfigView,
        message_log::MessageLog,
        plugin::{ChannelOutbound, ChannelPlugin, ChannelStreamOutbound},
    },
    secrecy::{ExposeSecret, Secret},
    std::{collections::HashMap, sync::Arc},
    tokio::sync::RwLock,
    tracing::{info, warn},
};

use crate::{
    config::{TelephonyAccountConfig, TelephonyProviderId},
    manager::CallManager,
    outbound::{RoutingOutbound, TelephonyStreamOutbound},
    providers::twilio::TwilioProvider,
};

/// Per-account runtime state.
struct AccountState {
    config: TelephonyAccountConfig,
    manager: Arc<RwLock<CallManager>>,
}

/// Telephony channel plugin.
pub struct TelephonyPlugin {
    accounts: HashMap<String, AccountState>,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
    routing_outbound: Arc<RoutingOutbound>,
}

impl TelephonyPlugin {
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
            message_log: None,
            event_sink: None,
            routing_outbound: Arc::new(RoutingOutbound::new()),
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

    #[cfg(test)]
    pub(crate) fn set_test_account(
        &mut self,
        account_id: String,
        config: TelephonyAccountConfig,
        manager: Arc<RwLock<CallManager>>,
    ) {
        self.accounts
            .insert(account_id, AccountState { config, manager });
    }

    /// Access the call manager for a given account.
    pub fn call_manager(&self, account_id: &str) -> Option<Arc<RwLock<CallManager>>> {
        self.accounts
            .get(account_id)
            .map(|a| Arc::clone(&a.manager))
    }

    /// Get the configured from_number for an account.
    pub fn caller_number(&self, account_id: &str) -> Option<String> {
        self.accounts
            .get(account_id)
            .map(|a| a.config.from_number.clone())
    }

    /// Dispatch recognized speech to the agent loop via the channel event sink.
    ///
    /// This is called from the gather webhook handler when Twilio sends a
    /// speech recognition result. The text is dispatched asynchronously so
    /// the TwiML response returns immediately.
    pub async fn dispatch_speech(&self, account_id: &str, call_id: &str, caller: &str, text: &str) {
        let Some(sink) = &self.event_sink else {
            tracing::warn!("telephony: no event sink configured, cannot dispatch speech");
            return;
        };

        let config = self.accounts.get(account_id).map(|a| &a.config);

        let reply_to = moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Telephony,
            account_id: account_id.to_string(),
            chat_id: call_id.to_string(),
            message_id: None,
            thread_id: None,
        };

        let meta = moltis_channels::ChannelMessageMeta {
            channel_type: moltis_channels::ChannelType::Telephony,
            sender_name: Some(caller.to_string()),
            username: Some(caller.to_string()),
            sender_id: Some(caller.to_string()),
            message_kind: Some(moltis_channels::ChannelMessageKind::Voice),
            model: config.and_then(|c| c.model.clone()),
            agent_id: config.and_then(|c| c.agent_id.clone()),
            audio_filename: None,
            documents: None,
        };

        sink.dispatch_to_chat(text, reply_to, meta).await;
    }
}

impl Default for TelephonyPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelPlugin for TelephonyPlugin {
    fn id(&self) -> &str {
        "telephony"
    }

    fn name(&self) -> &str {
        "Telephony"
    }

    async fn start_account(
        &mut self,
        account_id: &str,
        config: serde_json::Value,
    ) -> moltis_channels::Result<()> {
        let cfg: TelephonyAccountConfig = serde_json::from_value(config)
            .map_err(|e| moltis_channels::Error::invalid_input(e.to_string()))?;

        let provider: Box<dyn crate::provider::TelephonyProvider> = match cfg.provider {
            TelephonyProviderId::Twilio => {
                let sid = cfg
                    .account_sid
                    .as_ref()
                    .map(|s| s.expose_secret().clone())
                    .unwrap_or_default();
                let token = cfg
                    .auth_token
                    .clone()
                    .unwrap_or_else(|| Secret::new(String::new()));

                if sid.is_empty() {
                    return Err(moltis_channels::Error::invalid_input(
                        "account_sid is required for Twilio",
                    ));
                }
                Box::new(TwilioProvider::new(sid, token))
            },
            TelephonyProviderId::Telnyx => {
                let api_key = cfg
                    .auth_token
                    .clone()
                    .unwrap_or_else(|| Secret::new(String::new()));
                let connection_id = cfg
                    .account_sid
                    .as_ref()
                    .map(|s| s.expose_secret().clone())
                    .unwrap_or_default();

                if connection_id.is_empty() {
                    return Err(moltis_channels::Error::invalid_input(
                        "connection_id (account_sid field) is required for Telnyx",
                    ));
                }
                Box::new(crate::providers::telnyx::TelnyxProvider::new(
                    api_key,
                    connection_id,
                ))
            },
            TelephonyProviderId::Plivo => {
                let auth_id = cfg
                    .account_sid
                    .as_ref()
                    .map(|s| s.expose_secret().clone())
                    .unwrap_or_default();
                let auth_token = cfg
                    .auth_token
                    .clone()
                    .unwrap_or_else(|| Secret::new(String::new()));

                if auth_id.is_empty() {
                    return Err(moltis_channels::Error::invalid_input(
                        "auth_id (account_sid field) is required for Plivo",
                    ));
                }
                Box::new(crate::providers::plivo::PlivoProvider::new(
                    auth_id, auth_token,
                ))
            },
        };

        if self.accounts.contains_key(account_id) {
            self.stop_account(account_id).await?;
        }

        let manager = Arc::new(RwLock::new(CallManager::new(
            provider,
            cfg.max_duration_secs,
        )));

        let gather_url = cfg
            .webhook_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(|base_url| {
                format!(
                    "{}/api/channels/telephony/{account_id}/gather",
                    base_url.trim_end_matches('/')
                )
            })
            .unwrap_or_else(|| format!("/api/channels/telephony/{account_id}/gather"));
        self.routing_outbound
            .set_manager(account_id, Arc::clone(&manager), gather_url);

        self.accounts.insert(account_id.to_string(), AccountState {
            config: cfg,
            manager,
        });

        info!(account_id = %account_id, "telephony account started");
        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> moltis_channels::Result<()> {
        self.routing_outbound.remove_manager(account_id);
        if let Some(state) = self.accounts.remove(account_id) {
            let call_ids = state
                .manager
                .read()
                .await
                .active_calls()
                .into_iter()
                .map(|call| call.call_id)
                .collect::<Vec<_>>();
            for call_id in call_ids {
                let mgr = state.manager.read().await;
                if let Err(e) = mgr.hangup(&call_id).await {
                    warn!(call_id = %call_id, "failed to hangup on stop: {e}");
                }
            }
            info!(account_id = %account_id, "telephony account stopped");
        }
        Ok(())
    }

    fn outbound(&self) -> Option<&dyn ChannelOutbound> {
        None
    }

    fn status(&self) -> Option<&dyn moltis_channels::plugin::ChannelStatus> {
        None
    }

    fn has_account(&self, account_id: &str) -> bool {
        self.accounts.contains_key(account_id)
    }

    fn account_ids(&self) -> Vec<String> {
        self.accounts.keys().cloned().collect()
    }

    fn account_config(&self, account_id: &str) -> Option<Box<dyn ChannelConfigView>> {
        self.accounts
            .get(account_id)
            .map(|a| Box::new(a.config.clone()) as Box<dyn ChannelConfigView>)
    }

    fn update_account_config(
        &self,
        _account_id: &str,
        _config: serde_json::Value,
    ) -> moltis_channels::Result<()> {
        Ok(())
    }

    fn account_config_json(&self, account_id: &str) -> Option<serde_json::Value> {
        self.accounts
            .get(account_id)
            .and_then(|a| serde_json::to_value(&a.config).ok())
    }

    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::clone(&self.routing_outbound) as Arc<dyn ChannelOutbound>
    }

    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(TelephonyStreamOutbound)
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::providers::mock::MockProvider};

    fn twilio_config(from_number: &str) -> serde_json::Value {
        serde_json::json!({
            "provider": "twilio",
            "account_sid": "AC_test",
            "auth_token": "test_token",
            "from_number": from_number,
        })
    }

    fn twilio_config_with_webhook(from_number: &str, webhook_url: &str) -> serde_json::Value {
        serde_json::json!({
            "provider": "twilio",
            "account_sid": "AC_test",
            "auth_token": "test_token",
            "from_number": from_number,
            "webhook_url": webhook_url,
        })
    }

    #[tokio::test]
    async fn start_account_stops_existing_account_before_replacing_manager() {
        let account_id = "default";
        let mut plugin = TelephonyPlugin::new();
        let old_manager = Arc::new(RwLock::new(CallManager::new(
            Box::new(MockProvider::new()),
            60,
        )));
        let old_call_id = old_manager
            .read()
            .await
            .register_inbound("PROV-OLD", "+1", "+2", account_id);
        plugin.routing_outbound.set_manager(
            account_id,
            Arc::clone(&old_manager),
            "/api/channels/telephony/default/gather".to_string(),
        );
        plugin
            .accounts
            .insert(account_id.to_string(), AccountState {
                config: TelephonyAccountConfig {
                    from_number: "+1".to_string(),
                    ..TelephonyAccountConfig::default()
                },
                manager: Arc::clone(&old_manager),
            });

        plugin
            .start_account(account_id, twilio_config("+15550000002"))
            .await
            .unwrap_or_else(|error| panic!("account should restart: {error}"));

        let new_manager = plugin
            .call_manager(account_id)
            .unwrap_or_else(|| panic!("replacement manager should be registered"));
        assert!(!Arc::ptr_eq(&old_manager, &new_manager));
        assert!(old_manager.read().await.get_call(&old_call_id).is_none());
        assert!(
            old_manager
                .read()
                .await
                .resolve_call_id("PROV-OLD")
                .is_none()
        );
        assert_eq!(
            plugin.caller_number(account_id).as_deref(),
            Some("+15550000002")
        );
    }

    #[tokio::test]
    async fn start_account_uses_absolute_gather_url_from_webhook_base() {
        let account_id = "default";
        let mut plugin = TelephonyPlugin::new();

        plugin
            .start_account(
                account_id,
                twilio_config_with_webhook("+15550000002", "https://calls.example.com/base/"),
            )
            .await
            .unwrap_or_else(|error| panic!("account should start: {error}"));

        assert_eq!(
            plugin.routing_outbound.gather_url(account_id).as_deref(),
            Some("https://calls.example.com/base/api/channels/telephony/default/gather")
        );
    }
}
