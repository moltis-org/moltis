use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use {
    async_trait::async_trait,
    secrecy::ExposeSecret,
    tracing::{debug, info, warn},
};

use moltis_channels::{
    ChannelConfigView, ChannelEvent, ChannelEventSink, Error as ChannelError,
    Result as ChannelResult,
    gating::{DmPolicy, GroupPolicy, is_allowed},
    message_log::{MessageLog, MessageLogEntry},
    plugin::{
        ChannelAttachment, ChannelHealthSnapshot, ChannelMessageKind, ChannelMessageMeta,
        ChannelOutbound, ChannelPlugin, ChannelReplyTarget, ChannelStatus, ChannelStreamOutbound,
        ChannelThreadContext, ChannelType, ThreadMessage,
    },
};

use crate::{
    activity::TeamsActivity,
    auth::get_access_token,
    cards,
    config::{MsTeamsAccountConfig, resolve_mention_mode},
    jwt::BotFrameworkJwtValidator,
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

    /// Acquire an authenticated token for Graph API calls on behalf of the given account.
    ///
    /// Uses the `https://graph.microsoft.com/.default` scope (separate from the
    /// Bot Framework token which is scoped to `api.botframework.com`).
    pub async fn graph_client(
        &self,
        account_id: &str,
    ) -> anyhow::Result<(reqwest::Client, secrecy::Secret<String>)> {
        let (http, config, graph_cache) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let state = accounts
                .get(account_id)
                .ok_or_else(|| anyhow::anyhow!("unknown Teams account: {account_id}"))?;
            (
                state.http.clone(),
                state.config.clone(),
                Arc::clone(&state.graph_token_cache),
            )
        };
        let token = crate::auth::get_graph_token(&http, &config, &graph_cache).await?;
        Ok((http, token))
    }

    /// Edit the text of an existing bot message by activity ID.
    pub async fn edit_message(
        &self,
        account_id: &str,
        conversation_id: &str,
        activity_id: &str,
        new_text: &str,
    ) -> ChannelResult<()> {
        self.outbound
            .edit_message(account_id, conversation_id, activity_id, new_text)
            .await
    }

    /// Delete a bot message by activity ID.
    pub async fn delete_message(
        &self,
        account_id: &str,
        conversation_id: &str,
        activity_id: &str,
    ) -> ChannelResult<()> {
        self.outbound
            .delete_activity(account_id, conversation_id, activity_id)
            .await
    }

    /// Get the JWT validator for an account (if JWT auth is configured).
    pub fn jwt_validator(&self, account_id: &str) -> Option<Arc<BotFrameworkJwtValidator>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .and_then(|s| s.jwt_validator.clone())
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

    /// Ingest an already-verified webhook activity.
    ///
    /// Use this when the caller has already verified the secret via
    /// the channel webhook middleware pipeline.
    pub async fn ingest_verified_activity(
        &self,
        account_id: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<()> {
        let activity: TeamsActivity = serde_json::from_value(payload)?;
        self.process_activity(account_id, activity).await
    }

    async fn handle_activity(
        &self,
        account_id: &str,
        activity: TeamsActivity,
        webhook_secret: Option<&str>,
    ) -> anyhow::Result<()> {
        let (config, _event_sink, _message_log, _service_urls) = {
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

        self.process_activity(account_id, activity).await
    }

    /// Core activity processing logic, called after authentication.
    async fn process_activity(
        &self,
        account_id: &str,
        activity: TeamsActivity,
    ) -> anyhow::Result<()> {
        let (config, event_sink, message_log, service_urls, welcomed) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let state = accounts
                .get(account_id)
                .ok_or_else(|| anyhow::anyhow!("unknown Teams account: {account_id}"))?;
            (
                state.config.clone(),
                state.event_sink.clone(),
                state.message_log.clone(),
                Arc::clone(&state.service_urls),
                Arc::clone(&state.welcomed_conversations),
            )
        };

        // Cache service URL for outbound routing.
        if let (Some(conversation_id), Some(service_url)) =
            (activity.conversation_id(), activity.service_url.as_deref())
        {
            let mut map = service_urls.write().unwrap_or_else(|e| e.into_inner());
            map.insert(conversation_id.to_string(), service_url.to_string());
        }

        // Handle conversationUpdate: welcome cards when bot is added.
        if activity.activity_type == "conversationUpdate" {
            return self
                .handle_conversation_update(account_id, &activity, &config, &welcomed)
                .await;
        }

        // Handle messageReaction activities.
        if activity.activity_type == "messageReaction" {
            debug!(account_id, "Teams messageReaction activity (logged)");
            return Ok(());
        }

        // Only process message activities from here.
        if activity.activity_type != "message" {
            return Ok(());
        }

        let text = activity.cleaned_text();
        let chat_id = activity
            .conversation_id()
            .ok_or_else(|| anyhow::anyhow!("missing conversation ID in Teams activity"))?
            .to_string();
        let peer_id = activity
            .sender_id()
            .unwrap_or_else(|| "unknown".to_string());
        let sender_name = activity.sender_name();
        let is_group = activity.is_group_chat();
        let team_id = activity.team_id().map(String::from);
        let channel_id = activity.channel_id().map(String::from);

        // Resolve mention mode with per-team/channel overrides.
        let effective_mention_mode =
            resolve_mention_mode(&config, team_id.as_deref(), channel_id.as_deref());

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
            match effective_mention_mode {
                moltis_channels::gating::MentionMode::Always => true,
                moltis_channels::gating::MentionMode::Mention => activity.bot_is_mentioned(),
                moltis_channels::gating::MentionMode::None => false,
            }
        } else {
            true
        };
        let access_granted = policy_allowed && mention_allowed;

        // Log inbound message text (or empty if attachment-only).
        let log_text = text.clone().unwrap_or_default();

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
                    body: log_text.clone(),
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

        // Send welcome card for first DM if enabled.
        if !is_group && config.welcome_card {
            let is_new = {
                let mut set = welcomed.write().unwrap_or_else(|e| e.into_inner());
                set.insert(chat_id.clone())
            };
            if is_new {
                let bot_name = config.bot_name.as_deref().unwrap_or("Moltis");
                let card = cards::build_welcome_card(bot_name, &config.prompt_starters);
                let card_activity = cards::card_activity(card, None);
                if let Err(e) = self
                    .outbound
                    .send_activity_with_retry(account_id, &chat_id, card_activity)
                    .await
                {
                    debug!(
                        account_id,
                        chat_id, "failed to send Teams welcome card: {e}"
                    );
                }
            }
        }

        let message_id = activity.id.clone();
        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::MsTeams,
            account_id: account_id.to_string(),
            chat_id: chat_id.clone(),
            message_id,
            thread_id: None,
        };

        let Some(sink) = event_sink else {
            warn!(
                account_id,
                "Teams inbound message ignored: no channel event sink"
            );
            return Ok(());
        };

        // Download attachments if present.
        let downloaded_attachments = if activity.has_downloadable_attachments() {
            self.download_activity_attachments(account_id, &activity)
                .await
        } else {
            Vec::new()
        };

        // Handle slash commands.
        let dispatch_text = text.as_deref().unwrap_or("");
        if let Some(command) = dispatch_text.strip_prefix('/') {
            match sink
                .dispatch_command(command.trim(), reply_to.clone(), Some(&peer_id))
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
                    let message = format!("Command failed: {e}");
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

        // No text and no attachments — nothing to dispatch.
        if dispatch_text.is_empty() && downloaded_attachments.is_empty() {
            return Ok(());
        }

        #[cfg(feature = "metrics")]
        moltis_metrics::counter!(
            moltis_metrics::channels::MESSAGES_RECEIVED_TOTAL,
            moltis_metrics::labels::CHANNEL => "msteams"
        )
        .increment(1);

        let meta = ChannelMessageMeta {
            channel_type: ChannelType::MsTeams,
            sender_name,
            username: None,
            sender_id: Some(peer_id.clone()),
            message_kind: Some(if !downloaded_attachments.is_empty() {
                ChannelMessageKind::Photo
            } else {
                ChannelMessageKind::Text
            }),
            model: config.resolve_model(&chat_id, &peer_id).map(String::from),
            agent_id: config
                .resolve_agent_id(&chat_id, &peer_id)
                .map(String::from),
            audio_filename: None,
            documents: None,
        };

        if !downloaded_attachments.is_empty() {
            sink.dispatch_to_chat_with_attachments(
                dispatch_text,
                downloaded_attachments,
                reply_to,
                meta,
            )
            .await;
        } else {
            sink.dispatch_to_chat(dispatch_text, reply_to, meta).await;
        }
        Ok(())
    }

    /// Handle conversationUpdate activities (welcome cards).
    async fn handle_conversation_update(
        &self,
        account_id: &str,
        activity: &TeamsActivity,
        config: &MsTeamsAccountConfig,
        welcomed: &Arc<RwLock<HashSet<String>>>,
    ) -> anyhow::Result<()> {
        if !activity.bot_was_added() {
            return Ok(());
        }

        let Some(chat_id) = activity.conversation_id() else {
            return Ok(());
        };

        let is_group = activity.is_group_chat();

        // Send welcome card/text.
        if is_group && config.group_welcome_card {
            let bot_name = config.bot_name.as_deref().unwrap_or("Moltis");
            let text = cards::build_group_welcome_text(bot_name);
            if let Err(e) = self
                .outbound
                .send_text(account_id, chat_id, &text, None)
                .await
            {
                debug!(account_id, chat_id, "failed to send group welcome: {e}");
            }
        } else if !is_group && config.welcome_card {
            let is_new = {
                let mut set = welcomed.write().unwrap_or_else(|e| e.into_inner());
                set.insert(chat_id.to_string())
            };
            if is_new {
                let bot_name = config.bot_name.as_deref().unwrap_or("Moltis");
                let card = cards::build_welcome_card(bot_name, &config.prompt_starters);
                let card_activity = cards::card_activity(card, None);
                if let Err(e) = self
                    .outbound
                    .send_activity_with_retry(account_id, chat_id, card_activity)
                    .await
                {
                    debug!(account_id, chat_id, "failed to send welcome card: {e}");
                }
            }
        }

        Ok(())
    }

    /// Download attachments from an inbound activity.
    async fn download_activity_attachments(
        &self,
        account_id: &str,
        activity: &TeamsActivity,
    ) -> Vec<ChannelAttachment> {
        let (http, config, token_cache) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            match accounts.get(account_id) {
                Some(state) => (
                    state.http.clone(),
                    state.config.clone(),
                    Arc::clone(&state.token_cache),
                ),
                None => return Vec::new(),
            }
        };

        let token = match get_access_token(&http, &config, &token_cache).await {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    account_id,
                    "failed to get token for attachment download: {e}"
                );
                return Vec::new();
            },
        };

        let mut result = Vec::new();
        for att in &activity.attachments {
            match crate::attachments::download_attachment(&http, &token, att).await {
                Ok(Some(downloaded)) => {
                    result.push(ChannelAttachment {
                        media_type: downloaded.media_type,
                        data: downloaded.data,
                    });
                },
                Ok(None) => {}, // Non-downloadable (card, etc.)
                Err(e) => {
                    warn!(account_id, "attachment download failed: {e}");
                },
            }
        }
        result
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

        let http = moltis_common::http_client::build_default_http_client();

        // Create JWT validator if tenant_id is configured.
        let jwt_validator = if !cfg.app_id.is_empty() {
            let tenant = if cfg.tenant_id == "botframework.com" {
                None
            } else {
                Some(cfg.tenant_id.clone())
            };
            Some(Arc::new(BotFrameworkJwtValidator::new(
                cfg.app_id.clone(),
                tenant,
                http.clone(),
            )))
        } else {
            None
        };

        info!(account_id, "starting microsoft teams account");
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        accounts.insert(account_id.to_string(), AccountState {
            account_id: account_id.to_string(),
            config: cfg,
            message_log: self.message_log.clone(),
            event_sink: self.event_sink.clone(),
            http,
            token_cache: Arc::new(tokio::sync::Mutex::new(None)),
            graph_token_cache: Arc::new(tokio::sync::Mutex::new(None)),
            service_urls: Arc::new(RwLock::new(HashMap::new())),
            jwt_validator,
            welcomed_conversations: Arc::new(RwLock::new(HashSet::new())),
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
        let parsed: MsTeamsAccountConfig = serde_json::from_value(config)?;
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get_mut(account_id) {
            state.config = parsed;
            Ok(())
        } else {
            Err(ChannelError::unknown_account(account_id))
        }
    }

    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(MsTeamsOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(MsTeamsOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn channel_webhook_verifier(
        &self,
        account_id: &str,
    ) -> Option<Box<dyn moltis_channels::channel_webhook_middleware::ChannelWebhookVerifier>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts.get(account_id)?;
        Some(Box::new(
            crate::channel_webhook_verifier::TeamsChannelWebhookVerifier::new(
                state.config.webhook_secret.clone(),
                true,
            ),
        ))
    }

    fn thread_context(&self) -> Option<&dyn ChannelThreadContext> {
        Some(self)
    }
}

#[async_trait]
impl ChannelThreadContext for MsTeamsPlugin {
    async fn fetch_thread_messages(
        &self,
        account_id: &str,
        channel_id: &str,
        _thread_id: &str,
        limit: usize,
    ) -> ChannelResult<Vec<ThreadMessage>> {
        let (http, config, graph_cache) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let state = accounts
                .get(account_id)
                .ok_or_else(|| ChannelError::unknown_account(account_id))?;
            (
                state.http.clone(),
                state.config.clone(),
                Arc::clone(&state.graph_token_cache),
            )
        };

        let token = crate::auth::get_graph_token(&http, &config, &graph_cache)
            .await
            .map_err(|e| ChannelError::unavailable(format!("Teams Graph token: {e}")))?;

        let effective_limit = if limit == 0 {
            config.history_limit
        } else {
            limit.min(config.history_limit)
        };

        let messages =
            crate::graph::fetch_chat_messages(&http, &token, channel_id, effective_limit)
                .await
                .map_err(|e| {
                    ChannelError::external(
                        "Teams Graph fetch messages",
                        std::io::Error::other(e.to_string()),
                    )
                })?;

        Ok(messages
            .into_iter()
            .map(|m| ThreadMessage {
                sender_id: m.from_user_id.unwrap_or_default(),
                is_bot: m.is_bot,
                text: m.body_content.unwrap_or_default(),
                timestamp: m.created_at.unwrap_or_default(),
            })
            .collect())
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
                extra: None,
            })
        } else {
            Ok(ChannelHealthSnapshot {
                connected: false,
                account_id: account_id.to_string(),
                details: Some("account not started".into()),
                extra: None,
            })
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {
        super::*,
        moltis_channels::{InboundMode, plugin::ChannelType},
    };

    #[test]
    fn descriptor_coherence() {
        let plugin = MsTeamsPlugin::new();
        let desc = ChannelType::MsTeams.descriptor();

        assert_eq!(desc.channel_type, ChannelType::MsTeams);
        assert_eq!(desc.display_name, "Microsoft Teams");
        assert_eq!(desc.capabilities.inbound_mode, InboundMode::Webhook);

        // OTP: MsTeams does NOT implement ChannelOtpProvider
        assert!(!desc.capabilities.supports_otp);
        assert!(plugin.as_otp_provider().is_none());

        // Threads: MsTeams now implements ChannelThreadContext
        assert!(plugin.thread_context().is_some());
    }
}
