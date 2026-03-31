use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};

use {
    async_trait::async_trait,
    tracing::{info, warn},
};

use moltis_channels::{
    ChannelConfigView, ChannelEventSink, Error as ChannelError, Result as ChannelResult,
    message_log::MessageLog,
    otp::OtpChallengeInfo,
    plugin::{
        ChannelHealthSnapshot, ChannelOtpProvider, ChannelOutbound, ChannelPlugin, ChannelStatus,
        ChannelStreamOutbound,
    },
};

use crate::{
    client,
    config::MatrixAccountConfig,
    handlers,
    otp::OtpState,
    outbound::MatrixOutbound,
    state::{AccountState, AccountStateMap},
    stream::MatrixStreamOutbound,
};

const PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

pub struct MatrixPlugin {
    accounts: AccountStateMap,
    outbound: MatrixOutbound,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
    probe_cache: RwLock<HashMap<String, (ChannelHealthSnapshot, Instant)>>,
    data_dir: std::path::PathBuf,
}

impl MatrixPlugin {
    pub fn new(data_dir: std::path::PathBuf) -> Self {
        let accounts: AccountStateMap = Arc::new(RwLock::new(HashMap::new()));
        let outbound = MatrixOutbound {
            accounts: Arc::clone(&accounts),
        };
        Self {
            accounts,
            outbound,
            message_log: None,
            event_sink: None,
            probe_cache: RwLock::new(HashMap::new()),
            data_dir,
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
}

#[async_trait]
impl ChannelPlugin for MatrixPlugin {
    fn id(&self) -> &str {
        "matrix"
    }

    fn name(&self) -> &str {
        "Matrix"
    }

    #[tracing::instrument(skip(self, config))]
    async fn start_account(
        &mut self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let matrix_config: MatrixAccountConfig = serde_json::from_value(config)?;

        client::validate_homeserver_url(
            &matrix_config.homeserver_url,
            matrix_config.allow_private_network,
        )
        .map_err(ChannelError::invalid_input)?;

        info!(account_id, homeserver = %matrix_config.homeserver_url, "starting matrix account");

        let store_path = self.data_dir.join(account_id);
        if let Err(e) = std::fs::create_dir_all(&store_path) {
            warn!(account_id, error = %e, "failed to create matrix store dir");
        }

        let sdk_client = client::build_client(&matrix_config, &store_path)
            .await
            .map_err(|e| ChannelError::unavailable(format!("build matrix client: {e}")))?;

        client::authenticate(&sdk_client, &matrix_config)
            .await
            .map_err(|e| ChannelError::unavailable(format!("matrix auth: {e}")))?;

        let cancel = tokio_util::sync::CancellationToken::new();
        let outbound = Arc::new(MatrixOutbound {
            accounts: Arc::clone(&self.accounts),
        });

        let state = AccountState {
            client: sdk_client.clone(),
            account_id: account_id.to_string(),
            config: matrix_config,
            outbound,
            cancel: cancel.clone(),
            message_log: self.message_log.clone(),
            event_sink: self.event_sink.clone(),
            otp: Mutex::new(OtpState::new(300)),
        };

        {
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            accounts.insert(account_id.to_string(), state);
        }

        handlers::register_event_handlers(
            &sdk_client,
            Arc::clone(&self.accounts),
            account_id.to_string(),
        );

        client::start_sync(
            sdk_client,
            cancel,
            Arc::clone(&self.accounts),
            account_id.to_string(),
        );

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn stop_account(&mut self, account_id: &str) -> ChannelResult<()> {
        let cancel = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get(account_id).map(|s| s.cancel.clone())
        };

        if let Some(cancel) = cancel {
            info!(account_id, "stopping matrix account");
            cancel.cancel();
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            accounts.remove(account_id);
        } else {
            warn!(account_id, "matrix account not found");
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
            .and_then(|s| serde_json::to_value(crate::config::RedactedConfig(&s.config)).ok())
    }

    fn update_account_config(
        &self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let matrix_config: MatrixAccountConfig = serde_json::from_value(config)?;
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get_mut(account_id) {
            state.config = matrix_config;
            Ok(())
        } else {
            Err(ChannelError::unknown_account(account_id))
        }
    }

    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(MatrixOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(MatrixStreamOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn as_otp_provider(&self) -> Option<&dyn ChannelOtpProvider> {
        Some(self)
    }
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    #[test]
    fn descriptor_coherence() {
        use moltis_channels::{ChannelType, InboundMode};
        let desc = ChannelType::Matrix.descriptor();
        assert_eq!(desc.channel_type, ChannelType::Matrix);
        assert_eq!(desc.display_name, "Matrix");
        assert_eq!(desc.capabilities.inbound_mode, InboundMode::GatewayLoop);
        assert!(desc.capabilities.supports_otp);
        assert!(desc.capabilities.supports_threads);
        assert!(desc.capabilities.supports_reactions);
        assert!(desc.capabilities.supports_location);
    }
}

impl ChannelOtpProvider for MatrixPlugin {
    fn pending_otp_challenges(&self, account_id: &str) -> Vec<OtpChallengeInfo> {
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
impl ChannelStatus for MatrixPlugin {
    #[tracing::instrument(skip(self))]
    async fn probe(&self, account_id: &str) -> ChannelResult<ChannelHealthSnapshot> {
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
                    let logged_in = state.client.user_id().is_some();
                    ChannelHealthSnapshot {
                        connected: logged_in,
                        account_id: account_id.to_string(),
                        details: Some(if logged_in {
                            format!("User: {}", state.config.user_id)
                        } else {
                            "not logged in".into()
                        }),
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
