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
    outbound::{NoopOutbound, TelephonyOutbound, TelephonyStreamOutbound},
    providers::twilio::TwilioProvider,
};

/// Per-account runtime state.
struct AccountState {
    config: TelephonyAccountConfig,
    manager: Arc<RwLock<CallManager>>,
    #[allow(dead_code)]
    outbound: Arc<TelephonyOutbound>,
}

/// Telephony channel plugin.
pub struct TelephonyPlugin {
    accounts: HashMap<String, AccountState>,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
}

impl TelephonyPlugin {
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
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

    /// Access the call manager for a given account.
    pub fn call_manager(&self, account_id: &str) -> Option<Arc<RwLock<CallManager>>> {
        self.accounts
            .get(account_id)
            .map(|a| Arc::clone(&a.manager))
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
        };

        let manager = Arc::new(RwLock::new(CallManager::new(
            provider,
            cfg.max_duration_secs,
        )));

        let outbound = Arc::new(TelephonyOutbound::new(Arc::clone(&manager)));

        self.accounts.insert(account_id.to_string(), AccountState {
            config: cfg,
            manager,
            outbound,
        });

        info!(account_id = %account_id, "telephony account started");
        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> moltis_channels::Result<()> {
        if let Some(state) = self.accounts.remove(account_id) {
            let mgr = state.manager.read().await;
            for call in mgr.active_calls() {
                if let Err(e) = mgr.hangup(&call.call_id).await {
                    warn!(call_id = %call.call_id, "failed to hangup on stop: {e}");
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
        Arc::new(NoopOutbound)
    }

    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(TelephonyStreamOutbound)
    }
}
