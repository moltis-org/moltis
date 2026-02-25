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
    ChannelEvent, ChannelEventSink, Error as ChannelError, Result as ChannelResult,
    gating::{DmPolicy, GroupPolicy, MentionMode, is_allowed},
    message_log::{MessageLog, MessageLogEntry},
    plugin::{
        ChannelHealthSnapshot, ChannelMessageKind, ChannelMessageMeta, ChannelOutbound,
        ChannelPlugin, ChannelReplyTarget, ChannelStatus, ChannelStreamOutbound, ChannelType,
    },
};

use crate::{
    activity::TeamsActivity,
    config::MsTeamsAccountConfig,
    outbound::MsTeamsOutbound,
    state::{AccountState, AccountStateMap},
};

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Microsoft Teams channel plugin.
pub struct MsTeamsPlugin {
    accounts: AccountStateMap,
    outbound: MsTeamsOutbound,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
}

impl MsTeamsPlugin {
    pub fn new() -> Self {
        let accounts: AccountStateMap = Arc::new(RwLock::new(HashMap::new()));
        let outbound = MsTeamsOutbound {
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

    pub fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(MsTeamsOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    pub fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(MsTeamsOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    pub fn account_ids(&self) -> Vec<String> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.keys().cloned().collect()
    }

    pub fn has_account(&self, account_id: &str) -> bool {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.contains_key(account_id)
    }

    pub fn account_config(&self, account_id: &str) -> Option<serde_json::Value> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .and_then(|s| serde_json::to_value(&s.config).ok())
    }

    pub fn update_account_config(
        &self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let parsed: MsTeamsAccountConfig = serde_json::from_value(config)?;
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get_mut(account_id) {
            state.config = parsed;
            Ok(())
        } else {
            Err(ChannelError::unknown_account(account_id))
        }
    }

    pub async fn ingest_activity(
        &self,
        account_id: &str,
        payload: serde_json::Value,
        webhook_secret: Option<&str>,
    ) -> anyhow::Result<()> {
        let activity: TeamsActivity = serde_json::from_value(payload)?;
        self.handle_activity(account_id, activity, webhook_secret)
            .await
    }

    async fn handle_activity(
        &self,
        account_id: &str,
        activity: TeamsActivity,
        webhook_secret: Option<&str>,
    ) -> anyhow::Result<()> {
        let (config, event_sink, message_log, service_urls) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let state = accounts
                .get(account_id)
                .ok_or_else(|| anyhow::anyhow!("unknown Teams account: {account_id}"))?;
            (
                state.config.clone(),
                state.event_sink.clone(),
                state.message_log.clone(),
                Arc::clone(&state.service_urls),
            )
        };

        if let Some(expected) = config
            .webhook_secret
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .filter(|s| !s.is_empty())
            && webhook_secret != Some(expected)
        {
            anyhow::bail!("invalid Teams webhook secret");
        }

        if let (Some(conversation_id), Some(service_url)) =
            (activity.conversation_id(), activity.service_url.as_deref())
        {
            let mut map = service_urls.write().unwrap_or_else(|e| e.into_inner());
            map.insert(conversation_id.to_string(), service_url.to_string());
        }

        if activity.activity_type != "message" {
            return Ok(());
        }

        let Some(text) = activity.cleaned_text() else {
            return Ok(());
        };
        let chat_id = activity
            .conversation_id()
            .ok_or_else(|| anyhow::anyhow!("missing conversation ID in Teams activity"))?
            .to_string();
        let peer_id = activity
            .sender_id()
            .unwrap_or_else(|| "unknown".to_string());
        let sender_name = activity.sender_name();
        let is_group = activity.is_group_chat();

        let policy_allowed = if is_group {
            match config.group_policy {
                GroupPolicy::Open => true,
                GroupPolicy::Allowlist => is_allowed(&chat_id, &config.group_allowlist),
                GroupPolicy::Disabled => false,
            }
        } else {
            match config.dm_policy {
                DmPolicy::Open => true,
                DmPolicy::Allowlist => is_allowed(&peer_id, &config.allowlist),
                DmPolicy::Disabled => false,
            }
        };
        let mention_allowed = if is_group {
            match config.mention_mode {
                MentionMode::Always => true,
                MentionMode::Mention => activity.bot_is_mentioned(),
                MentionMode::None => false,
            }
        } else {
            true
        };
        let access_granted = policy_allowed && mention_allowed;

        if let Some(log) = message_log {
            let _ = log
                .log(MessageLogEntry {
                    id: 0,
                    account_id: account_id.to_string(),
                    channel_type: "msteams".into(),
                    peer_id: peer_id.clone(),
                    username: None,
                    sender_name: sender_name.clone(),
                    chat_id: chat_id.clone(),
                    chat_type: if is_group {
                        "group".into()
                    } else {
                        "private".into()
                    },
                    body: text.clone(),
                    access_granted,
                    created_at: unix_now(),
                })
                .await;
        }

        if let Some(sink) = event_sink.as_ref() {
            sink.emit(ChannelEvent::InboundMessage {
                channel_type: ChannelType::MsTeams,
                account_id: account_id.to_string(),
                peer_id: peer_id.clone(),
                username: None,
                sender_name: sender_name.clone(),
                message_count: None,
                access_granted,
            })
            .await;
        }

        if !access_granted {
            return Ok(());
        }

        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::MsTeams,
            account_id: account_id.to_string(),
            chat_id: chat_id.clone(),
            message_id: activity.id,
        };

        let Some(sink) = event_sink else {
            warn!(
                account_id,
                "Teams inbound message ignored: no channel event sink"
            );
            return Ok(());
        };

        if let Some(command) = text.strip_prefix('/') {
            match sink
                .dispatch_command(command.trim(), reply_to.clone())
                .await
            {
                Ok(response) => {
                    if let Err(e) = self
                        .outbound
                        .send_text(
                            account_id,
                            &chat_id,
                            &response,
                            reply_to.message_id.as_deref(),
                        )
                        .await
                    {
                        warn!(
                            account_id,
                            chat_id, "failed to send Teams command response: {e}"
                        );
                    }
                },
                Err(e) => {
                    let message = format!("⚠️ Command failed: {e}");
                    if let Err(send_err) = self
                        .outbound
                        .send_text(
                            account_id,
                            &chat_id,
                            &message,
                            reply_to.message_id.as_deref(),
                        )
                        .await
                    {
                        warn!(
                            account_id,
                            chat_id, "failed to send Teams command error: {send_err}"
                        );
                    }
                },
            }
            return Ok(());
        }

        sink.dispatch_to_chat(&text, reply_to, ChannelMessageMeta {
            channel_type: ChannelType::MsTeams,
            sender_name,
            username: None,
            message_kind: Some(ChannelMessageKind::Text),
            model: config.model.clone(),
            audio_filename: None,
        })
        .await;
        Ok(())
    }
}

impl Default for MsTeamsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelPlugin for MsTeamsPlugin {
    fn id(&self) -> &str {
        "msteams"
    }

    fn name(&self) -> &str {
        "Microsoft Teams"
    }

    async fn start_account(
        &mut self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let cfg: MsTeamsAccountConfig = serde_json::from_value(config)?;
        if cfg.app_id.is_empty() {
            return Err(ChannelError::invalid_input("Teams app_id is required"));
        }
        if cfg.app_password.expose_secret().is_empty() {
            return Err(ChannelError::invalid_input(
                "Teams app_password is required",
            ));
        }

        info!(account_id, "starting microsoft teams account");
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        accounts.insert(account_id.to_string(), AccountState {
            account_id: account_id.to_string(),
            config: cfg,
            message_log: self.message_log.clone(),
            event_sink: self.event_sink.clone(),
            http: reqwest::Client::new(),
            token_cache: Arc::new(tokio::sync::Mutex::new(None)),
            service_urls: Arc::new(RwLock::new(HashMap::new())),
        });
        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> ChannelResult<()> {
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        if accounts.remove(account_id).is_none() {
            warn!(account_id, "Teams account not found");
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
impl ChannelStatus for MsTeamsPlugin {
    async fn probe(&self, account_id: &str) -> ChannelResult<ChannelHealthSnapshot> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get(account_id) {
            let service_url_count = {
                let map = state.service_urls.read().unwrap_or_else(|e| e.into_inner());
                map.len()
            };
            let details = if service_url_count == 0 {
                "waiting for first inbound activity".to_string()
            } else {
                format!("known conversations: {service_url_count}")
            };
            Ok(ChannelHealthSnapshot {
                connected: true,
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
