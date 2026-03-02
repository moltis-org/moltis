use std::sync::Arc;

use {
    async_trait::async_trait,
    serde_json::Value,
    tracing::{error, info, warn},
};

use {
    moltis_channels::{
        ChannelOutbound, ChannelType,
        message_log::MessageLog,
        plugin::ChannelHealthSnapshot,
        registry::ChannelRegistry,
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

/// Live channel service backed by the channel registry.
///
/// All per-channel dispatch is handled by the registry — no match arms needed.
pub struct LiveChannelService {
    registry: Arc<ChannelRegistry>,
    outbound: Arc<dyn ChannelOutbound>,
    store: Arc<dyn ChannelStore>,
    message_log: Arc<dyn MessageLog>,
    session_metadata: Arc<SqliteSessionMetadata>,
}

impl LiveChannelService {
    pub fn new(
        registry: Arc<ChannelRegistry>,
        outbound: Arc<dyn ChannelOutbound>,
        store: Arc<dyn ChannelStore>,
        message_log: Arc<dyn MessageLog>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            registry,
            outbound,
            store,
            message_log,
            session_metadata,
        }
    }

    /// Resolve channel type from explicit params, registry index, or store fallback.
    async fn resolve_channel_type(
        &self,
        params: &Value,
        account_id: &str,
        default_when_unknown: ChannelType,
    ) -> Result<ChannelType, String> {
        if let Some(type_str) = params.get("type").and_then(|v| v.as_str()) {
            return type_str.parse::<ChannelType>().map_err(|e| e.to_string());
        }

        // Check the registry index (O(1) lookup).
        if let Some(ct_str) = self.registry.resolve_channel_type(account_id) {
            return ct_str.parse::<ChannelType>().map_err(|e| e.to_string());
        }

        // Fall back to store lookup.
        let mut matches = Vec::new();
        for ct in ChannelType::ALL {
            if self
                .store
                .get(ct.as_str(), account_id)
                .await
                .map_err(|e| e.to_string())?
                .is_some()
            {
                matches.push(*ct);
            }
        }
        match matches.len() {
            1 => Ok(matches[0]),
            n if n > 1 => Err(format!(
                "account_id '{account_id}' exists in multiple stored channel types; pass explicit 'type'"
            )),
            _ => Ok(default_when_unknown),
        }
    }

    /// Build a status entry for a single channel account.
    async fn channel_status_entry(
        &self,
        channel_type: ChannelType,
        account_id: &str,
        snap: ChannelHealthSnapshot,
        config: Option<Value>,
    ) -> Value {
        let mut entry = serde_json::json!({
            "type": channel_type.as_str(),
            "name": format!("{} ({account_id})", channel_type.display_name()),
            "account_id": account_id,
            "status": if snap.connected { "connected" } else { "disconnected" },
            "details": snap.details,
            "capabilities": channel_type.descriptor().capabilities,
        });
        if let Some(cfg) = config {
            entry["config"] = cfg;
        }

        let ct = channel_type.as_str();
        let bound = self
            .session_metadata
            .list_account_sessions(ct, account_id)
            .await;
        let active_map = self
            .session_metadata
            .list_active_sessions(ct, account_id)
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
        entry
    }
}

#[async_trait]
impl ChannelService for LiveChannelService {
    async fn status(&self) -> ServiceResult {
        let mut channels = Vec::new();

        for ct_str in self.registry.list() {
            let Some(plugin_lock) = self.registry.get(ct_str) else {
                continue;
            };

            let Ok(channel_type) = ct_str.parse::<ChannelType>() else {
                continue;
            };

            let account_ids = {
                let p = plugin_lock.read().await;
                p.account_ids()
            };

            for aid in &account_ids {
                let (snap_result, config_json) = {
                    let p = plugin_lock.read().await;
                    let snap = match p.status() {
                        Some(status) => Some(status.probe(aid).await),
                        None => None,
                    };
                    let cfg = p.account_config_json(aid);
                    (snap, cfg)
                };

                match snap_result {
                    Some(Ok(snap)) => {
                        let entry = self
                            .channel_status_entry(channel_type, aid, snap, config_json)
                            .await;
                        channels.push(entry);
                    },
                    Some(Err(e)) => channels.push(serde_json::json!({
                        "type": ct_str,
                        "name": format!("{} ({aid})", channel_type.display_name()),
                        "account_id": aid,
                        "status": "error",
                        "details": e.to_string(),
                    })),
                    None => {},
                }
            }
        }

        Ok(serde_json::json!({ "channels": channels }))
    }

    async fn add(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;
        let channel_type = self
            .resolve_channel_type(&params, account_id, ChannelType::Telegram)
            .await?;
        let config = params
            .get("config")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        info!(
            account_id,
            channel_type = channel_type.as_str(),
            "adding channel account"
        );
        self.registry
            .start_account(channel_type.as_str(), account_id, config.clone())
            .await
            .map_err(|e| {
                error!(error = %e, account_id, channel_type = channel_type.as_str(), "failed to start account");
                e.to_string()
            })?;

        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.to_string(),
                config,
                created_at: now,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist channel");
        }

        Ok(serde_json::json!({
            "added": account_id,
            "type": channel_type.to_string()
        }))
    }

    async fn remove(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;
        let channel_type = self
            .resolve_channel_type(&params, account_id, ChannelType::Telegram)
            .await?;

        info!(
            account_id,
            channel_type = channel_type.as_str(),
            "removing channel account"
        );
        self.registry
            .stop_account(channel_type.as_str(), account_id)
            .await
            .map_err(|e| {
                error!(error = %e, account_id, channel_type = channel_type.as_str(), "failed to stop account");
                e.to_string()
            })?;

        if let Err(e) = self.store.delete(channel_type.as_str(), account_id).await {
            warn!(error = %e, account_id, "failed to delete channel from store");
        }

        Ok(serde_json::json!({
            "removed": account_id,
            "type": channel_type.to_string()
        }))
    }

    async fn logout(&self, params: Value) -> ServiceResult {
        self.remove(params).await
    }

    async fn update(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;
        let channel_type = self
            .resolve_channel_type(&params, account_id, ChannelType::Telegram)
            .await?;
        let config = params
            .get("config")
            .cloned()
            .ok_or_else(|| "missing 'config'".to_string())?;

        info!(
            account_id,
            channel_type = channel_type.as_str(),
            "updating channel account"
        );
        let ct = channel_type.as_str();
        self.registry
            .stop_account(ct, account_id)
            .await
            .map_err(|e| {
                error!(error = %e, account_id, channel_type = ct, "failed to stop account");
                e.to_string()
            })?;
        self.registry
            .start_account(ct, account_id, config.clone())
            .await
            .map_err(|e| {
                error!(error = %e, account_id, channel_type = ct, "failed to start account");
                e.to_string()
            })?;

        let created_at = self
            .store
            .get(ct, account_id)
            .await
            .map_err(|e| e.to_string())?
            .map(|s| s.created_at)
            .unwrap_or_else(unix_now);
        let now = unix_now();
        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.to_string(),
                config,
                created_at,
                updated_at: now,
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist channel update");
        }

        Ok(serde_json::json!({
            "updated": account_id,
            "type": channel_type.to_string()
        }))
    }

    async fn send(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .or_else(|| params.get("channel"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "missing 'account_id' (or alias 'channel')".to_string())?;
        let to = params
            .get("to")
            .or_else(|| params.get("chat_id"))
            .or_else(|| params.get("chatId"))
            .or_else(|| params.get("peer_id"))
            .or_else(|| params.get("peerId"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "missing 'to' (or aliases 'chat_id'/'peer_id')".to_string())?;
        let text = params
            .get("text")
            .or_else(|| params.get("message"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "missing 'text' (or alias 'message')".to_string())?;
        let reply_to = params
            .get("reply_to")
            .or_else(|| params.get("replyTo"))
            .or_else(|| params.get("message_id"))
            .or_else(|| params.get("messageId"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let silent = params
            .get("silent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let html = params
            .get("html")
            .or_else(|| params.get("as_html"))
            .or_else(|| params.get("asHtml"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if silent && html {
            return Err("invalid send options: 'silent' and 'html' cannot both be true".into());
        }

        let channel_type = self
            .resolve_channel_type(&params, account_id, ChannelType::Telegram)
            .await?;
        let reply_to_ref = reply_to;

        let send_result = if html {
            self.outbound
                .send_html(account_id, to, text, reply_to_ref)
                .await
        } else if silent {
            self.outbound
                .send_text_silent(account_id, to, text, reply_to_ref)
                .await
        } else {
            self.outbound
                .send_text(account_id, to, text, reply_to_ref)
                .await
        };
        send_result.map_err(ServiceError::message)?;

        info!(
            account_id,
            channel_type = channel_type.as_str(),
            to,
            silent,
            html,
            "sent outbound channel message"
        );

        Ok(serde_json::json!({
            "ok": true,
            "type": channel_type.as_str(),
            "account_id": account_id,
            "to": to,
            "silent": silent,
            "html": html,
            "reply_to": reply_to,
        }))
    }

    async fn senders_list(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;
        let channel_type = self
            .resolve_channel_type(&params, account_id, ChannelType::Telegram)
            .await?;

        let senders = self
            .message_log
            .unique_senders(channel_type.as_str(), account_id)
            .await
            .map_err(ServiceError::message)?;

        let allowlist = self
            .registry
            .account_config(account_id)
            .await
            .map(|cfg| cfg.allowlist().to_vec())
            .unwrap_or_default();

        // Query OTP challenges generically via the OTP provider sub-trait.
        let otp_challenges = {
            let ct_str = channel_type.as_str();
            if let Some(plugin_lock) = self.registry.get(ct_str) {
                let p = plugin_lock.read().await;
                p.as_otp_provider()
                    .map(|otp| otp.pending_otp_challenges(account_id))
            } else {
                None
            }
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
                if let Some(otp) = otp_challenges
                    .as_ref()
                    .and_then(|pending| pending.iter().find(|c| c.peer_id == s.peer_id))
                {
                    entry["otp_pending"] = serde_json::json!({
                        "code": otp.code,
                        "expires_at": otp.expires_at,
                    });
                }
                entry
            })
            .collect();

        Ok(serde_json::json!({
            "senders": list,
            "type": channel_type.to_string()
        }))
    }

    async fn sender_approve(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;
        let identifier = params
            .get("identifier")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'identifier'".to_string())?;
        let channel_type = self
            .resolve_channel_type(&params, account_id, ChannelType::Telegram)
            .await?;

        let stored = self
            .store
            .get(channel_type.as_str(), account_id)
            .await
            .map_err(ServiceError::message)?
            .ok_or_else(|| {
                format!(
                    "channel '{}' ({}) not found in store",
                    account_id,
                    channel_type.as_str()
                )
            })?;

        let mut config = stored.config.clone();
        let allowlist = config
            .as_object_mut()
            .ok_or_else(|| "config is not an object".to_string())?
            .entry("allowlist")
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
        if let Some(obj) = config.as_object_mut() {
            obj.insert("dm_policy".into(), serde_json::json!("allowlist"));
        }

        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.to_string(),
                config: config.clone(),
                created_at: stored.created_at,
                updated_at: unix_now(),
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist sender approval");
        }

        if let Err(e) = self
            .registry
            .update_account_config(account_id, config)
            .await
        {
            warn!(error = %e, account_id, channel_type = channel_type.as_str(), "failed to hot-update config");
        }

        info!(
            account_id,
            identifier,
            channel_type = channel_type.as_str(),
            "sender approved"
        );
        Ok(serde_json::json!({
            "approved": identifier,
            "type": channel_type.to_string()
        }))
    }

    async fn sender_deny(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;
        let identifier = params
            .get("identifier")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'identifier'".to_string())?;
        let channel_type = self
            .resolve_channel_type(&params, account_id, ChannelType::Telegram)
            .await?;

        let stored = self
            .store
            .get(channel_type.as_str(), account_id)
            .await
            .map_err(ServiceError::message)?
            .ok_or_else(|| {
                format!(
                    "channel '{}' ({}) not found in store",
                    account_id,
                    channel_type.as_str()
                )
            })?;

        let mut config = stored.config.clone();
        if let Some(arr) = config
            .as_object_mut()
            .and_then(|o| o.get_mut("allowlist"))
            .and_then(|v| v.as_array_mut())
        {
            let id_lower = identifier.to_lowercase();
            arr.retain(|v| v.as_str().is_none_or(|s| s.to_lowercase() != id_lower));
        }

        if let Err(e) = self
            .store
            .upsert(StoredChannel {
                account_id: account_id.to_string(),
                channel_type: channel_type.to_string(),
                config: config.clone(),
                created_at: stored.created_at,
                updated_at: unix_now(),
            })
            .await
        {
            warn!(error = %e, account_id, "failed to persist sender denial");
        }

        if let Err(e) = self
            .registry
            .update_account_config(account_id, config)
            .await
        {
            warn!(error = %e, account_id, channel_type = channel_type.as_str(), "failed to hot-update config");
        }

        info!(
            account_id,
            identifier,
            channel_type = channel_type.as_str(),
            "sender denied"
        );
        Ok(serde_json::json!({
            "denied": identifier,
            "type": channel_type.to_string()
        }))
    }
}
