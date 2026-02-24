use std::sync::Arc;

use {
    async_trait::async_trait,
    serde_json::Value,
    tokio::sync::RwLock,
    tracing::{error, info, warn},
};

use {
    moltis_channels::ChannelPlugin, moltis_discord::DiscordPlugin, moltis_slack::SlackPlugin,
    moltis_telegram::TelegramPlugin,
};

use {
    moltis_channels::{
        message_log::MessageLog,
        store::{ChannelStore, StoredChannel},
    },
    moltis_sessions::metadata::SqliteSessionMetadata,
};

use crate::services::{ChannelService, ServiceError, ServiceResult};

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Live channel service backed by multiple channel plugins.
pub struct LiveChannelService {
    telegram: Arc<RwLock<TelegramPlugin>>,
    slack: Arc<RwLock<SlackPlugin>>,
    discord: Arc<RwLock<DiscordPlugin>>,
    store: Arc<dyn ChannelStore>,
    message_log: Arc<dyn MessageLog>,
    session_metadata: Arc<SqliteSessionMetadata>,
}

impl LiveChannelService {
    pub fn new(
        telegram: TelegramPlugin,
        slack: SlackPlugin,
        discord: DiscordPlugin,
        store: Arc<dyn ChannelStore>,
        message_log: Arc<dyn MessageLog>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            telegram: Arc::new(RwLock::new(telegram)),
            slack: Arc::new(RwLock::new(slack)),
            discord: Arc::new(RwLock::new(discord)),
            store,
            message_log,
            session_metadata,
        }
    }
}

#[async_trait]
impl ChannelService for LiveChannelService {
    async fn status(&self) -> ServiceResult {
        let mut channels = Vec::new();

        // Telegram accounts
        {
            let tg = self.telegram.read().await;
            let account_ids = tg.account_ids();

            if let Some(status) = tg.status() {
                for aid in &account_ids {
                    match status.probe(aid).await {
                        Ok(snap) => {
                            let mut entry = serde_json::json!({
                                "type": "telegram",
                                "name": format!("Telegram ({})", aid),
                                "account_id": aid,
                                "status": if snap.connected { "connected" } else { "disconnected" },
                                "details": snap.details,
                            });
                            if let Some(cfg) = tg.account_config(aid) {
                                entry["config"] = cfg;
                            }

                            let bound = self
                                .session_metadata
                                .list_account_sessions("telegram", aid)
                                .await;
                            let active_map = self
                                .session_metadata
                                .list_active_sessions("telegram", aid)
                                .await;
                            let sessions: Vec<_> = bound
                                .iter()
                                .map(|s| {
                                    let is_active = active_map.iter().any(|(_, sk)| sk == &s.key);
                                    serde_json::json!({
                                        "key": s.key,
                                        "label": s.label,
                                        "messageCount": s.message_count,
                                        "active": is_active,
                                    })
                                })
                                .collect();
                            if !sessions.is_empty() {
                                entry["sessions"] = serde_json::json!(sessions);
                            }

                            channels.push(entry);
                        },
                        Err(e) => {
                            channels.push(serde_json::json!({
                                "type": "telegram",
                                "name": format!("Telegram ({})", aid),
                                "account_id": aid,
                                "status": "error",
                                "details": e.to_string(),
                            }));
                        },
                    }
                }
            }
        }

        // Slack accounts
        {
            let slack = self.slack.read().await;
            let account_ids = slack.account_ids();

            if let Some(status) = slack.status() {
                for aid in &account_ids {
                    match status.probe(aid).await {
                        Ok(snap) => {
                            let mut entry = serde_json::json!({
                                "type": "slack",
                                "name": format!("Slack ({})", aid),
                                "account_id": aid,
                                "status": if snap.connected { "connected" } else { "disconnected" },
                                "details": snap.details,
                            });
                            if let Some(cfg) = slack.account_config(aid) {
                                entry["config"] = cfg;
                            }

                            let bound = self
                                .session_metadata
                                .list_account_sessions("slack", aid)
                                .await;
                            let active_map = self
                                .session_metadata
                                .list_active_sessions("slack", aid)
                                .await;
                            let sessions: Vec<_> = bound
                                .iter()
                                .map(|s| {
                                    let is_active = active_map.iter().any(|(_, sk)| sk == &s.key);
                                    serde_json::json!({
                                        "key": s.key,
                                        "label": s.label,
                                        "messageCount": s.message_count,
                                        "active": is_active,
                                    })
                                })
                                .collect();
                            if !sessions.is_empty() {
                                entry["sessions"] = serde_json::json!(sessions);
                            }

                            channels.push(entry);
                        },
                        Err(e) => {
                            channels.push(serde_json::json!({
                                "type": "slack",
                                "name": format!("Slack ({})", aid),
                                "account_id": aid,
                                "status": "error",
                                "details": e.to_string(),
                            }));
                        },
                    }
                }
            }
        }

        // Discord accounts
        {
            let discord = self.discord.read().await;
            let account_ids = discord.account_ids();

            if let Some(status) = discord.status() {
                for aid in &account_ids {
                    match status.probe(aid).await {
                        Ok(snap) => {
                            let mut entry = serde_json::json!({
                                "type": "discord",
                                "name": format!("Discord ({})", aid),
                                "account_id": aid,
                                "status": if snap.connected { "connected" } else { "disconnected" },
                                "details": snap.details,
                            });
                            if let Some(cfg) = discord.account_config(aid) {
                                entry["config"] = cfg;
                            }

                            let bound = self
                                .session_metadata
                                .list_account_sessions("discord", aid)
                                .await;
                            let active_map = self
                                .session_metadata
                                .list_active_sessions("discord", aid)
                                .await;
                            let sessions: Vec<_> = bound
                                .iter()
                                .map(|s| {
                                    let is_active = active_map.iter().any(|(_, sk)| sk == &s.key);
                                    serde_json::json!({
                                        "key": s.key,
                                        "label": s.label,
                                        "messageCount": s.message_count,
                                        "active": is_active,
                                    })
                                })
                                .collect();
                            if !sessions.is_empty() {
                                entry["sessions"] = serde_json::json!(sessions);
                            }

                            channels.push(entry);
                        },
                        Err(e) => {
                            channels.push(serde_json::json!({
                                "type": "discord",
                                "name": format!("Discord ({})", aid),
                                "account_id": aid,
                                "status": "error",
                                "details": e.to_string(),
                            }));
                        },
                    }
                }
            }
        }

        Ok(serde_json::json!({ "channels": channels }))
    }

    async fn add(&self, params: Value) -> ServiceResult {
        let channel_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("telegram");
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let config = params
            .get("config")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        info!(account_id, channel_type, "adding channel account");

        match channel_type {
            "telegram" => {
                let mut tg = self.telegram.write().await;
                tg.start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to start telegram account");
                        e.to_string()
                    })?;
            },
            "slack" => {
                let mut slack = self.slack.write().await;
                slack
                    .start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to start slack account");
                        e.to_string()
                    })?;
            },
            "discord" => {
                let mut discord = self.discord.write().await;
                discord
                    .start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to start discord account");
                        e.to_string()
                    })?;
            },
            _ => return Err(format!("unsupported channel type: {channel_type}").into()),
        }

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.into(),
                config,
                created_at: now,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist channel");
        }

        Ok(serde_json::json!({ "added": account_id }))
    }

    async fn remove(&self, params: Value) -> ServiceResult {
        let channel_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("telegram");

        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        info!(account_id, channel_type, "removing channel account");

        match channel_type {
            "telegram" => {
                let mut tg = self.telegram.write().await;
                tg.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop telegram account");
                    e.to_string()
                })?;
            },
            "slack" => {
                let mut slack = self.slack.write().await;
                slack.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop slack account");
                    e.to_string()
                })?;
            },
            "discord" => {
                let mut discord = self.discord.write().await;
                discord.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop discord account");
                    e.to_string()
                })?;
            },
            _ => return Err(format!("unsupported channel type: {channel_type}").into()),
        }

        if let Err(e) = self.store.delete(account_id).await {
            warn!(error = %e, account_id, "failed to delete channel from store");
        }

        Ok(serde_json::json!({ "removed": account_id }))
    }

    async fn logout(&self, params: Value) -> ServiceResult {
        self.remove(params).await
    }

    async fn update(&self, params: Value) -> ServiceResult {
        let channel_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("telegram");

        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let config = params
            .get("config")
            .cloned()
            .ok_or_else(|| "missing 'config'".to_string())?;

        info!(account_id, channel_type, "updating channel account");

        match channel_type {
            "telegram" => {
                let mut tg = self.telegram.write().await;
                tg.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop telegram account for update");
                    e.to_string()
                })?;
                tg.start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to restart telegram account after update");
                        e.to_string()
                    })?;
            },
            "slack" => {
                let mut slack = self.slack.write().await;
                slack.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop slack account for update");
                    e.to_string()
                })?;
                slack
                    .start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to restart slack account after update");
                        e.to_string()
                    })?;
            },
            "discord" => {
                let mut discord = self.discord.write().await;
                discord.stop_account(account_id).await.map_err(|e| {
                    error!(error = %e, account_id, "failed to stop discord account for update");
                    e.to_string()
                })?;
                discord
                    .start_account(account_id, config.clone())
                    .await
                    .map_err(|e| {
                        error!(error = %e, account_id, "failed to restart discord account after update");
                        e.to_string()
                    })?;
            },
            _ => return Err(format!("unsupported channel type: {channel_type}").into()),
        }

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.into(),
                config,
                created_at: now,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist channel update");
        }

        Ok(serde_json::json!({ "updated": account_id }))
    }

    async fn send(&self, _params: Value) -> ServiceResult {
        Err("direct channel send not yet implemented".into())
    }

    async fn senders_list(&self, params: Value) -> ServiceResult {
        let channel_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("telegram");

        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let senders = self
            .message_log
            .unique_senders(account_id)
            .await
            .map_err(ServiceError::message)?;

        // Read allowlist from current config to tag each sender.
        let allowlist: Vec<String> = match channel_type {
            "telegram" => {
                let tg = self.telegram.read().await;
                tg.account_config(account_id)
                    .and_then(|cfg| cfg.get("allowlist").cloned())
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default()
            },
            "slack" => {
                let slack = self.slack.read().await;
                slack
                    .account_config(account_id)
                    .and_then(|cfg| cfg.get("user_allowlist").cloned())
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default()
            },
            "discord" => {
                let discord = self.discord.read().await;
                discord
                    .account_config(account_id)
                    .and_then(|cfg| cfg.get("user_allowlist").cloned())
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default()
            },
            _ => Vec::new(),
        };

        // Query pending OTP challenges for this account.
        let otp_challenges = {
            let tg_inner = self.telegram.read().await;
            tg_inner.pending_otp_challenges(account_id)
        };

        let list: Vec<Value> = senders
            .into_iter()
            .map(|s| {
                let is_allowed = allowlist.iter().any(|a| {
                    let a_lower = a.to_lowercase();
                    a_lower == s.peer_id.to_lowercase()
                        || s.username
                            .as_ref()
                            .is_some_and(|u| a_lower == u.to_lowercase())
                });
                let mut entry = serde_json::json!({
                    "peer_id": s.peer_id,
                    "username": s.username,
                    "sender_name": s.sender_name,
                    "message_count": s.message_count,
                    "last_seen": s.last_seen,
                    "allowed": is_allowed,
                });
                // Attach OTP info if a challenge is pending for this peer.
                if let Some(otp) = otp_challenges.iter().find(|c| c.peer_id == s.peer_id) {
                    entry["otp_pending"] = serde_json::json!({
                        "code": otp.code,
                        "expires_at": otp.expires_at,
                    });
                }
                entry
            })
            .collect();

        Ok(serde_json::json!({ "senders": list }))
    }

    async fn sender_approve(&self, params: Value) -> ServiceResult {
        let channel_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("telegram");

        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let identifier = params
            .get("identifier")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'identifier'".to_string())?;

        // Read current stored config, add identifier to allowlist, persist & restart.
        let stored = self
            .store
            .get(account_id)
            .await
            .map_err(ServiceError::message)?
            .ok_or_else(|| format!("channel '{account_id}' not found in store"))?;

        let mut config = stored.config.clone();

        // Determine allowlist field name based on channel type
        let allowlist_field = match channel_type {
            "telegram" => "allowlist",
            "slack" | "discord" => "user_allowlist",
            _ => "allowlist",
        };

        let allowlist = config
            .as_object_mut()
            .ok_or_else(|| "config is not an object".to_string())?
            .entry(allowlist_field)
            .or_insert_with(|| serde_json::json!([]));

        let arr = allowlist
            .as_array_mut()
            .ok_or_else(|| "allowlist is not an array".to_string())?;

        let id_lower = identifier.to_lowercase();
        if !arr
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.to_lowercase() == id_lower))
        {
            arr.push(serde_json::json!(identifier));
        }

        // Also ensure dm_policy is set to "allowlist" so the list is enforced.
        if let Some(obj) = config.as_object_mut() {
            obj.insert("dm_policy".into(), serde_json::json!("allowlist"));
        }

        // Persist.
        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.into(),
                config: config.clone(),
                created_at: stored.created_at,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist sender approval");
        }

        // Hot-update the in-memory config where supported, restart otherwise.
        match channel_type {
            "telegram" => {
                // Telegram supports hot-update (preserves polling offset).
                let tg = self.telegram.read().await;
                if let Err(e) = tg.update_account_config(account_id, config) {
                    warn!(error = %e, account_id, "failed to hot-update config for sender approval");
                }
            },
            "slack" => {
                let mut slack = self.slack.write().await;
                if let Err(e) = slack.stop_account(account_id).await {
                    warn!(error = %e, account_id, "failed to stop account for sender approval");
                }
                if let Err(e) = slack.start_account(account_id, config).await {
                    warn!(error = %e, account_id, "failed to restart account for sender approval");
                }
            },
            "discord" => {
                let mut discord = self.discord.write().await;
                if let Err(e) = discord.stop_account(account_id).await {
                    warn!(error = %e, account_id, "failed to stop account for sender approval");
                }
                if let Err(e) = discord.start_account(account_id, config).await {
                    warn!(error = %e, account_id, "failed to restart account for sender approval");
                }
            },
            _ => return Err(format!("unsupported channel type: {channel_type}").into()),
        }

        info!(account_id, identifier, "sender approved");
        Ok(serde_json::json!({ "approved": identifier }))
    }

    async fn sender_deny(&self, params: Value) -> ServiceResult {
        let channel_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("telegram");

        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;

        let identifier = params
            .get("identifier")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'identifier'".to_string())?;

        let stored = self
            .store
            .get(account_id)
            .await
            .map_err(ServiceError::message)?
            .ok_or_else(|| format!("channel '{account_id}' not found in store"))?;

        let mut config = stored.config.clone();

        // Determine allowlist field name based on channel type
        let allowlist_field = match channel_type {
            "telegram" => "allowlist",
            "slack" | "discord" => "user_allowlist",
            _ => "allowlist",
        };

        if let Some(arr) = config
            .as_object_mut()
            .and_then(|o| o.get_mut(allowlist_field))
            .and_then(|v| v.as_array_mut())
        {
            let id_lower = identifier.to_lowercase();
            arr.retain(|v| v.as_str().is_none_or(|s| s.to_lowercase() != id_lower));
        }

        // Persist.
        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.into(),
                config: config.clone(),
                created_at: stored.created_at,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist sender denial");
        }

        // Hot-update the in-memory config where supported, restart otherwise.
        match channel_type {
            "telegram" => {
                // Telegram supports hot-update.
                let tg = self.telegram.read().await;
                if let Err(e) = tg.update_account_config(account_id, config) {
                    warn!(error = %e, account_id, "failed to hot-update config for sender denial");
                }
            },
            "slack" => {
                let mut slack = self.slack.write().await;
                if let Err(e) = slack.stop_account(account_id).await {
                    warn!(error = %e, account_id, "failed to stop account for sender denial");
                }
                if let Err(e) = slack.start_account(account_id, config).await {
                    warn!(error = %e, account_id, "failed to restart account for sender denial");
                }
            },
            "discord" => {
                let mut discord = self.discord.write().await;
                if let Err(e) = discord.stop_account(account_id).await {
                    warn!(error = %e, account_id, "failed to stop account for sender denial");
                }
                if let Err(e) = discord.start_account(account_id, config).await {
                    warn!(error = %e, account_id, "failed to restart account for sender denial");
                }
            },
            _ => return Err(format!("unsupported channel type: {channel_type}").into()),
        }

        info!(account_id, identifier, "sender denied");
        Ok(serde_json::json!({ "denied": identifier }))
    }
}
