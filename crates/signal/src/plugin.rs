//! Signal channel plugin lifecycle.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use {
    async_trait::async_trait,
    futures::StreamExt,
    moltis_channels::{
        ChannelEventSink, ChannelHealthSnapshot, ChannelOtpProvider, ChannelOutbound,
        ChannelPlugin, ChannelStatus, ChannelStreamOutbound, Result as ChannelResult,
        config_view::ChannelConfigView, otp::OtpChallengeInfo,
    },
    serde_json::Value,
    tokio_util::sync::CancellationToken,
};

use crate::{
    client::SignalClient,
    config::SignalAccountConfig,
    outbound::{AccountStateMap, SignalOutbound},
    sse::SseParser,
    state::AccountState,
};

pub struct SignalPlugin {
    accounts: AccountStateMap,
    outbound: SignalOutbound,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
}

impl SignalPlugin {
    pub fn new() -> Self {
        let accounts: AccountStateMap = Arc::new(RwLock::new(HashMap::new()));
        Self {
            outbound: SignalOutbound {
                accounts: Arc::clone(&accounts),
            },
            accounts,
            event_sink: None,
        }
    }

    pub fn with_event_sink(mut self, sink: Arc<dyn ChannelEventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    pub fn with_message_log(self, _log: Arc<dyn moltis_channels::message_log::MessageLog>) -> Self {
        self
    }

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

impl Default for SignalPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelPlugin for SignalPlugin {
    fn id(&self) -> &str {
        "signal"
    }

    fn name(&self) -> &str {
        "Signal"
    }

    async fn start_account(&mut self, account_id: &str, config: Value) -> ChannelResult<()> {
        let signal_config: SignalAccountConfig = serde_json::from_value(config).map_err(|e| {
            moltis_channels::Error::invalid_input(format!("invalid signal config: {e}"))
        })?;
        let signal_config = signal_config.normalize(account_id);

        self.stop_account(account_id).await?;

        if !signal_config.enabled {
            tracing::info!(account_id, "Signal account disabled, skipping");
            return Ok(());
        }

        let client = SignalClient::new();
        client.check(&signal_config.http_url).await?;

        let event_sink = self
            .event_sink
            .clone()
            .ok_or_else(|| moltis_channels::Error::unavailable("event sink not configured"))?;

        let cancel = CancellationToken::new();
        let shared_config = Arc::new(RwLock::new(signal_config.clone()));
        let shared_otp = Arc::new(Mutex::new(moltis_channels::otp::OtpState::new(
            signal_config.otp_cooldown_secs,
        )));

        tokio::spawn(run_event_loop(
            client.clone(),
            Arc::clone(&shared_config),
            Arc::clone(&shared_otp),
            account_id.to_string(),
            Arc::clone(&event_sink),
            cancel.clone(),
        ));

        let state = AccountState {
            client,
            config: shared_config,
            cancel,
            otp: shared_otp,
        };
        self.accounts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(account_id.to_string(), state);

        tracing::info!(account_id, "Signal account started");
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
            tracing::info!(account_id, "Signal account stopped");
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
        accounts.get(account_id).map(|state| {
            let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
            Box::new(cfg.clone()) as Box<dyn ChannelConfigView>
        })
    }

    /// Update the in-memory config for a running account.
    ///
    /// Note: changing `http_url` takes effect only on the next SSE reconnect
    /// cycle — the existing stream continues from the old daemon URL until it
    /// drops naturally or errors out.
    fn update_account_config(&self, account_id: &str, config: Value) -> ChannelResult<()> {
        let new_config: SignalAccountConfig = serde_json::from_value(config).map_err(|e| {
            moltis_channels::Error::invalid_input(format!("invalid signal config: {e}"))
        })?;
        let new_config = new_config.normalize(account_id);
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get(account_id) {
            let mut cfg = state.config.write().unwrap_or_else(|e| e.into_inner());
            *cfg = new_config.clone();
            let mut otp = state.otp.lock().unwrap_or_else(|e| e.into_inner());
            otp.set_cooldown(new_config.otp_cooldown_secs);
        }
        Ok(())
    }

    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(SignalOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(SignalOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn account_config_json(&self, account_id: &str) -> Option<Value> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.get(account_id).and_then(|state| {
            let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
            serde_json::to_value(&*cfg).ok()
        })
    }

    fn as_otp_provider(&self) -> Option<&dyn ChannelOtpProvider> {
        Some(self)
    }
}

#[async_trait]
impl ChannelStatus for SignalPlugin {
    async fn probe(&self, account_id: &str) -> ChannelResult<ChannelHealthSnapshot> {
        let snapshot = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get(account_id).map(|state| {
                (
                    state.client.clone(),
                    state
                        .config
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone(),
                )
            })
        };

        let Some((client, config)) = snapshot else {
            return Ok(ChannelHealthSnapshot {
                connected: false,
                account_id: account_id.to_string(),
                details: Some("account not found".to_string()),
                extra: None,
            });
        };

        match client.check(&config.http_url).await {
            Ok(extra) => Ok(ChannelHealthSnapshot {
                connected: true,
                account_id: account_id.to_string(),
                details: Some("signal-cli daemon reachable".to_string()),
                extra: Some(extra),
            }),
            Err(e) => Ok(ChannelHealthSnapshot {
                connected: false,
                account_id: account_id.to_string(),
                details: Some(format!("signal-cli daemon unreachable: {e}")),
                extra: None,
            }),
        }
    }
}

impl ChannelOtpProvider for SignalPlugin {
    fn pending_otp_challenges(&self, account_id: &str) -> Vec<OtpChallengeInfo> {
        self.otp_challenges(account_id)
    }
}

async fn run_event_loop(
    client: SignalClient,
    config: Arc<RwLock<SignalAccountConfig>>,
    otp: Arc<Mutex<moltis_channels::otp::OtpState>>,
    account_id: String,
    event_sink: Arc<dyn ChannelEventSink>,
    cancel: CancellationToken,
) {
    let mut backoff = Duration::from_secs(2);

    loop {
        if cancel.is_cancelled() {
            break;
        }

        let cfg = config.read().unwrap_or_else(|e| e.into_inner()).clone();
        let account = cfg.account().map(ToString::to_string);
        let stream = client
            .stream_events(&cfg.http_url, account.as_deref())
            .await;

        match stream {
            Ok(response) if response.status().is_success() => {
                backoff = Duration::from_secs(2);
                let mut parser = SseParser::default();
                let mut bytes = response.bytes_stream();

                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        next = bytes.next() => {
                            let Some(next) = next else {
                                break;
                            };
                            match next {
                                Ok(chunk) => {
                                    let text = String::from_utf8_lossy(&chunk);
                                    for event in parser.push(&text) {
                                        if let Some(data) = event.data {
                                            handle_sse_data(&data, &client, &config, &otp, &account_id, &event_sink).await;
                                        }
                                    }
                                },
                                Err(e) => {
                                    tracing::warn!(account_id, "Signal SSE stream error: {e}");
                                    break;
                                },
                            }
                        },
                    }
                }
            },
            Ok(response) => {
                tracing::warn!(
                    account_id,
                    status = %response.status(),
                    "Signal SSE connection failed"
                );
            },
            Err(e) => {
                tracing::warn!(account_id, "Signal SSE connection error: {e}");
            },
        }

        if cancel.is_cancelled() {
            break;
        }

        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(backoff) => {},
        }
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }

    tracing::info!(account_id, "Signal event loop exited");
}

async fn handle_sse_data(
    data: &str,
    client: &SignalClient,
    config: &Arc<RwLock<SignalAccountConfig>>,
    otp: &Arc<Mutex<moltis_channels::otp::OtpState>>,
    account_id: &str,
    event_sink: &Arc<dyn ChannelEventSink>,
) {
    match serde_json::from_str::<Value>(data) {
        Ok(value) => {
            crate::inbound::handle_event(&value, client, config, otp, account_id, event_sink).await;
        },
        Err(e) => {
            tracing::debug!(account_id, "Signal SSE event was not JSON: {e}");
        },
    }
}
