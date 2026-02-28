use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Instant,
};

use {
    async_trait::async_trait,
    tracing::{info, warn},
};

use moltis_channels::{
    ChannelEventSink, Result as ChannelResult,
    message_log::MessageLog,
    plugin::{
        ChannelHealthSnapshot, ChannelOutbound, ChannelPlugin, ChannelStatus, ChannelStreamOutbound,
    },
};

use crate::{
    config::WhatsAppAccountConfig, connection, outbound::WhatsAppOutbound, state::AccountStateMap,
};

/// Cache TTL for probe results (30 seconds).
const PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// WhatsApp channel plugin.
pub struct WhatsAppPlugin {
    accounts: AccountStateMap,
    outbound: WhatsAppOutbound,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
    data_dir: PathBuf,
    probe_cache: RwLock<HashMap<String, (ChannelHealthSnapshot, Instant)>>,
}

impl WhatsAppPlugin {
    pub fn new(data_dir: PathBuf) -> Self {
        let accounts: AccountStateMap = Arc::new(RwLock::new(HashMap::new()));
        let outbound = WhatsAppOutbound {
            accounts: Arc::clone(&accounts),
        };
        Self {
            accounts,
            outbound,
            message_log: None,
            event_sink: None,
            data_dir,
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

    /// Get a shared reference to the outbound sender.
    pub fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(WhatsAppOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    /// Get a shared reference to the streaming outbound sender.
    pub fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(WhatsAppOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    /// List all active account IDs.
    pub fn account_ids(&self) -> Vec<String> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.keys().cloned().collect()
    }

    /// Check whether the given account ID is registered.
    pub fn has_account(&self, account_id: &str) -> bool {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.contains_key(account_id)
    }

    /// Get the config for a specific account (serialized to JSON).
    pub fn account_config(&self, account_id: &str) -> Option<serde_json::Value> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .and_then(|s| serde_json::to_value(&s.config).ok())
    }

    /// Get the latest QR code data for a specific account.
    pub fn latest_qr(&self, account_id: &str) -> Option<String> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .and_then(|s| s.latest_qr.read().ok()?.clone())
    }

    /// Update the in-memory config for an account without restarting.
    /// Use for allowlist changes that don't need re-pairing.
    pub fn update_account_config(
        &self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let wa_config: WhatsAppAccountConfig = serde_json::from_value(config)?;
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get_mut(account_id) {
            state.config = wa_config;
            Ok(())
        } else {
            Err(moltis_channels::Error::unknown_account(account_id))
        }
    }

    /// List pending OTP challenges for a specific account.
    pub fn pending_otp_challenges(&self, account_id: &str) -> Vec<crate::otp::OtpChallengeInfo> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| {
                let otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                otp.list_pending()
            })
            .unwrap_or_default()
    }
}

#[async_trait]
impl ChannelPlugin for WhatsAppPlugin {
    fn id(&self) -> &str {
        "whatsapp"
    }

    fn name(&self) -> &str {
        "WhatsApp"
    }

    async fn start_account(
        &mut self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let wa_config: WhatsAppAccountConfig = serde_json::from_value(config)?;

        info!(account_id, "starting WhatsApp account");

        connection::start_connection(
            account_id.to_string(),
            wa_config,
            Arc::clone(&self.accounts),
            self.data_dir.clone(),
            self.message_log.clone(),
            self.event_sink.clone(),
        )
        .await
        .map_err(|e| moltis_channels::Error::unavailable(format!("whatsapp start: {e}")))?;

        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> ChannelResult<()> {
        let cancel = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get(account_id).map(|s| s.cancel.clone())
        };

        if let Some(cancel) = cancel {
            info!(account_id, "stopping WhatsApp account");
            cancel.cancel();
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            accounts.remove(account_id);
        } else {
            warn!(account_id, "WhatsApp account not found");
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
impl ChannelStatus for WhatsAppPlugin {
    async fn probe(&self, account_id: &str) -> ChannelResult<ChannelHealthSnapshot> {
        // Return cached result if fresh enough.
        if let Ok(cache) = self.probe_cache.read()
            && let Some((snap, ts)) = cache.get(account_id)
            && ts.elapsed() < PROBE_CACHE_TTL
        {
            return Ok(snap.clone());
        }

        let result = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            match accounts.get(account_id) {
                Some(state) => {
                    let connected = state.connected.load(std::sync::atomic::Ordering::Relaxed);
                    let details = if connected {
                        state
                            .config
                            .display_name
                            .as_ref()
                            .map(|n| format!("WhatsApp: {n}"))
                            .or_else(|| Some("WhatsApp: connected".into()))
                    } else if state
                        .latest_qr
                        .read()
                        .ok()
                        .and_then(|q| q.clone())
                        .is_some()
                    {
                        Some("waiting for QR scan".into())
                    } else {
                        Some("disconnected".into())
                    };
                    ChannelHealthSnapshot {
                        connected,
                        account_id: account_id.to_string(),
                        details,
                    }
                },
                None => ChannelHealthSnapshot {
                    connected: false,
                    account_id: account_id.to_string(),
                    details: Some("account not started".into()),
                },
            }
        };

        if let Ok(mut cache) = self.probe_cache.write() {
            cache.insert(account_id.to_string(), (result.clone(), Instant::now()));
        }

        Ok(result)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_and_name() {
        let plugin = WhatsAppPlugin::new(PathBuf::from("/tmp/test"));
        assert_eq!(plugin.id(), "whatsapp");
        assert_eq!(plugin.name(), "WhatsApp");
    }

    #[test]
    fn empty_account_ids() {
        let plugin = WhatsAppPlugin::new(PathBuf::from("/tmp/test"));
        assert!(plugin.account_ids().is_empty());
    }

    #[test]
    fn account_config_returns_none_for_unknown() {
        let plugin = WhatsAppPlugin::new(PathBuf::from("/tmp/test"));
        assert!(plugin.account_config("nonexistent").is_none());
    }

    #[test]
    fn latest_qr_returns_none_for_unknown() {
        let plugin = WhatsAppPlugin::new(PathBuf::from("/tmp/test"));
        assert!(plugin.latest_qr("nonexistent").is_none());
    }
}
