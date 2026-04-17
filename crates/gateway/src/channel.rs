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

fn is_redacted_secret(value: &Value) -> bool {
    matches!(value, Value::String(text) if text == moltis_common::secret_serde::REDACTED)
}

fn merge_channel_config_value(existing: &mut Value, patch: Value) {
    if is_redacted_secret(&patch) {
        return;
    }

    match (existing, patch) {
        (Value::Object(existing_obj), Value::Object(patch_obj)) => {
            for (key, patch_value) in patch_obj {
                if is_redacted_secret(&patch_value) {
                    continue;
                }
                if let Some(existing_value) = existing_obj.get_mut(&key) {
                    merge_channel_config_value(existing_value, patch_value);
                } else {
                    existing_obj.insert(key, patch_value);
                }
            }
        },
        (existing_value, patch_value) => {
            *existing_value = patch_value;
        },
    }
}

fn merge_channel_config(existing: Option<Value>, patch: Value) -> Value {
    match existing {
        Some(mut existing_value) => {
            merge_channel_config_value(&mut existing_value, patch);
            existing_value
        },
        None => patch,
    }
}

fn sender_allowlist_key(channel_type: ChannelType) -> &'static str {
    match channel_type {
        ChannelType::Matrix => "user_allowlist",
        _ => "allowlist",
    }
}

fn otp_pending_payload(code: &str, expires_at: i64) -> Value {
    serde_json::json!({
        "code": code,
        "expires_at": expires_at,
    })
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
        if let Some(extra) = snap.extra {
            entry["extra"] = extra;
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
    #[tracing::instrument(skip(self))]
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

    #[tracing::instrument(skip(self, params))]
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

    #[tracing::instrument(skip(self, params))]
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

    #[tracing::instrument(skip(self, params))]
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
        let ct = channel_type.as_str();
        let existing = self
            .store
            .get(ct, account_id)
            .await
            .map_err(|e| e.to_string())?;
        let created_at = existing
            .as_ref()
            .map(|stored| stored.created_at)
            .unwrap_or_else(unix_now);
        let config = merge_channel_config(existing.map(|stored| stored.config), config);

        info!(
            account_id,
            channel_type = channel_type.as_str(),
            "updating channel account"
        );
        let mut live_update_warning = None;
        match channel_type {
            ChannelType::Whatsapp => {
                // WhatsApp keeps a persistent sled DB lock while running; for
                // policy/config-only changes, apply hot updates in-place to
                // avoid stop/start lock races.
                //
                // Only suppress UnknownAccount (account not running) — config
                // validation errors (SerdeJson, InvalidInput) must fail the
                // request so we don't persist bad config to the store.
                match self
                    .registry
                    .update_account_config(account_id, config.clone())
                    .await
                {
                    Ok(()) => {},
                    Err(moltis_channels::Error::UnknownAccount { .. }) => {
                        warn!(
                            account_id,
                            channel_type = ct,
                            "WhatsApp account not running; config will apply on next start"
                        );
                        live_update_warning =
                            Some("config saved to store but live session was not updated");
                    },
                    Err(e) => {
                        error!(error = %e, account_id, channel_type = ct, "invalid config");
                        return Err(e.to_string().into());
                    },
                }
            },
            _ => {
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
            },
        }

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

        let mut result = serde_json::json!({
            "updated": account_id,
            "type": channel_type.to_string()
        });
        if let Some(warning) = live_update_warning {
            result["warning"] = Value::String(warning.to_string());
        }
        Ok(result)
    }

    #[tracing::instrument(skip(self, params))]
    async fn retry_ownership(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'account_id'".to_string())?;
        let channel_type = self
            .resolve_channel_type(&params, account_id, ChannelType::Telegram)
            .await?;

        if channel_type != ChannelType::Matrix {
            return Err(ServiceError::message(
                "ownership retry is only supported for Matrix accounts",
            ));
        }

        info!(
            account_id,
            channel_type = channel_type.as_str(),
            "retrying channel ownership setup"
        );

        self.registry
            .retry_account_setup(account_id)
            .await
            .map_err(|error| {
                error!(
                    error = %error,
                    account_id,
                    channel_type = channel_type.as_str(),
                    "failed to retry channel ownership setup"
                );
                ServiceError::message(error)
            })?;

        Ok(serde_json::json!({
            "retried": account_id,
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
                    entry["otp_pending"] = otp_pending_payload(&otp.code, otp.expires_at);
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
        let allowlist_key = sender_allowlist_key(channel_type);
        let allowlist = config
            .as_object_mut()
            .ok_or_else(|| "config is not an object".to_string())?
            .entry(allowlist_key)
            .or_insert_with(|| serde_json::json!([]));
        let arr = allowlist
            .as_array_mut()
            .ok_or_else(|| format!("{allowlist_key} is not an array"))?;

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
        let allowlist_key = sender_allowlist_key(channel_type);
        if let Some(arr) = config
            .as_object_mut()
            .and_then(|o| o.get_mut(allowlist_key))
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

    #[tracing::instrument(skip(self, params))]
    async fn oauth_start(&self, params: Value) -> ServiceResult {
        let account_id = params
            .get("account_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let homeserver = params
            .get("homeserver")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let redirect_uri = params
            .get("redirect_uri")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        if account_id.is_empty() {
            return Err("account_id is required".into());
        }
        if homeserver.is_empty() {
            return Err("homeserver is required".into());
        }
        if redirect_uri.is_empty() {
            return Err("redirect_uri is required".into());
        }

        // Merge caller-provided config (ownership_mode, policies, etc.) with
        // the required OIDC fields.
        let mut config = params
            .get("config")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(obj) = config.as_object_mut() {
            obj.insert("homeserver".into(), homeserver.into());
            obj.insert("auth_mode".into(), "oidc".into());
            obj.entry("access_token").or_insert_with(|| "".into());
        }

        let plugin_lock = self
            .registry
            .get("matrix")
            .ok_or_else(|| ServiceError::from("Matrix channel plugin is not available"))?;
        let plugin = plugin_lock.read().await;
        let result = plugin
            .oidc_start(&account_id, config, &redirect_uri)
            .await
            .map_err(|e| e.to_string())?;
        // Pre-register in the registry index so the account is findable
        // before oidc_complete runs.
        self.registry.index_account(&account_id, "matrix");
        Ok(result)
    }

    #[tracing::instrument(skip(self, params))]
    async fn oauth_complete(&self, params: Value) -> ServiceResult {
        let state = params
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let code = params
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        if state.is_empty() {
            return Err("state parameter is required".into());
        }
        if code.is_empty() {
            return Err("code parameter is required".into());
        }

        let callback_url = {
            let mut url = url::Url::parse("http://localhost/callback")
                .map_err(|e| ServiceError::from(e.to_string()))?;
            url.query_pairs_mut()
                .append_pair("code", &code)
                .append_pair("state", &state);
            url.to_string()
        };

        let plugin_lock = self
            .registry
            .get("matrix")
            .ok_or_else(|| ServiceError::from("Matrix channel plugin is not available"))?;
        let plugin = plugin_lock.read().await;
        let result = plugin
            .oidc_complete(&state, &callback_url)
            .await
            .map_err(|e| e.to_string())?;

        // Register in the registry index so retry_ownership and other
        // registry-level operations can find this account.
        if let Some(account_id) = result.get("account_id").and_then(Value::as_str) {
            self.registry.index_account(account_id, "matrix");
        }

        // Persist the new channel to the store.
        if let Some(account_id) = result.get("account_id").and_then(Value::as_str)
            && let Some(config_json) = plugin.account_config_json(account_id)
            && let Err(e) = self
                .store
                .upsert(StoredChannel {
                    account_id: account_id.to_string(),
                    channel_type: "matrix".to_string(),
                    config: config_json,
                    created_at: unix_now(),
                    updated_at: unix_now(),
                })
                .await
        {
            warn!(error = %e, account_id, "failed to persist OIDC channel to store");
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use {
        super::{merge_channel_config, otp_pending_payload, sender_allowlist_key},
        moltis_channels::ChannelType,
        serde_json::json,
    };

    #[test]
    fn merge_channel_config_preserves_omitted_fields() {
        let existing = json!({
            "dm_policy": "allowlist",
            "reply_to_message": false,
            "allowlist": ["alice"]
        });
        let patch = json!({
            "dm_policy": "open"
        });

        let merged = merge_channel_config(Some(existing), patch);
        assert_eq!(merged["dm_policy"], "open");
        assert_eq!(merged["reply_to_message"], false);
        assert_eq!(merged["allowlist"], json!(["alice"]));
    }

    #[test]
    fn merge_channel_config_ignores_redacted_secret_placeholders() {
        let existing = json!({
            "token": "real-secret",
            "webhook_secret": "real-webhook-secret"
        });
        let patch = json!({
            "token": "[REDACTED]",
            "webhook_secret": "[REDACTED]"
        });

        let merged = merge_channel_config(Some(existing), patch);
        assert_eq!(merged["token"], "real-secret");
        assert_eq!(merged["webhook_secret"], "real-webhook-secret");
    }

    #[test]
    fn merge_channel_config_allows_explicit_replacements() {
        let existing = json!({
            "channel_overrides": {
                "C123": {
                    "model": "old-model",
                    "model_provider": "anthropic"
                }
            }
        });
        let patch = json!({
            "channel_overrides": {
                "C123": {
                    "model": "new-model"
                }
            }
        });

        let merged = merge_channel_config(Some(existing), patch);
        assert_eq!(merged["channel_overrides"]["C123"]["model"], "new-model");
        assert_eq!(
            merged["channel_overrides"]["C123"]["model_provider"],
            "anthropic"
        );
    }

    #[test]
    fn sender_allowlist_key_uses_matrix_user_allowlist() {
        assert_eq!(sender_allowlist_key(ChannelType::Matrix), "user_allowlist");
        assert_eq!(sender_allowlist_key(ChannelType::Telegram), "allowlist");
    }

    #[test]
    fn otp_pending_payload_includes_code_for_authenticated_ui() {
        let payload = otp_pending_payload("954502", 1_234_567);

        assert_eq!(payload["expires_at"], 1_234_567);
        assert_eq!(payload["code"], "954502");
    }
}
