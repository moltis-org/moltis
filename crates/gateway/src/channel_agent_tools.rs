use {
    anyhow::{Result, anyhow},
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    moltis_channels::{
        gating::{DmPolicy, GroupPolicy, MentionMode},
        plugin::ChannelType,
        store::{ChannelStore, StoredChannel},
    },
    serde::{Deserialize, Serialize},
    serde_json::{Map, Value, json},
    std::sync::Arc,
};

use crate::services::ChannelService;

/// Agent tool that sends proactive outbound messages to configured channels.
///
/// Validation and alias resolution are handled by the underlying
/// [`ChannelService::send`] implementation; this tool only provides the
/// LLM-facing schema and forwards the parameters.
pub struct SendMessageTool {
    channel_service: Arc<dyn ChannelService>,
}

impl SendMessageTool {
    pub fn new(channel_service: Arc<dyn ChannelService>) -> Self {
        Self { channel_service }
    }
}

#[async_trait]
impl AgentTool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a proactive message to any configured channel account/chat (Telegram, Discord, Slack, Matrix, Teams, WhatsApp). Use this for alerts, reminders, and scheduled outreach."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["account_id", "to", "text"],
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Channel account identifier (for example a Telegram bot account id). Alias: channel."
                },
                "to": {
                    "type": "string",
                    "description": "Destination recipient/chat id in the target channel. Aliases: chat_id, chatId, peer_id, peerId."
                },
                "text": {
                    "type": "string",
                    "description": "Message text to send. Alias: message."
                },
                "type": {
                    "type": "string",
                    "enum": ["telegram", "discord", "slack", "matrix", "msteams", "whatsapp"],
                    "description": "Optional channel type hint when account ids may overlap across channel types."
                },
                "reply_to": {
                    "type": "string",
                    "description": "Optional platform message id to thread the outbound reply. Aliases: replyTo, message_id, messageId."
                },
                "silent": {
                    "type": "boolean",
                    "description": "Send without notification when supported by the channel.",
                    "default": false
                },
                "html": {
                    "type": "boolean",
                    "description": "Treat text as pre-formatted HTML when supported by the channel.",
                    "default": false
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        self.channel_service
            .send(params)
            .await
            .map_err(|e| anyhow!(e.to_string()))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ChannelSettingsPatch {
    dm_policy: Option<DmPolicy>,
    group_policy: Option<GroupPolicy>,
    mention_mode: Option<MentionMode>,
    model: Option<Option<String>>,
    model_provider: Option<Option<String>>,
    otp_self_approval: Option<bool>,
    otp_cooldown_secs: Option<u64>,
    reply_to_message: Option<bool>,
    thread_replies: Option<bool>,
    stream_mode: Option<ChannelSettingsStreamMode>,
    allowlist_add: Vec<String>,
    allowlist_remove: Vec<String>,
    group_allowlist_add: Vec<String>,
    group_allowlist_remove: Vec<String>,
    channel_override: Option<ModelOverridePatch>,
    user_override: Option<ModelOverridePatch>,
}

#[derive(Debug, Deserialize)]
struct ModelOverridePatch {
    target_id: String,
    #[serde(default)]
    model: Option<Option<String>>,
    #[serde(default)]
    model_provider: Option<Option<String>>,
    #[serde(default)]
    clear: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChannelSettingsStreamMode {
    EditInPlace,
    Native,
    Off,
}

impl ChannelSettingsStreamMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::EditInPlace => "edit_in_place",
            Self::Native => "native",
            Self::Off => "off",
        }
    }
}

/// Agent tool that safely updates persisted channel settings.
///
/// This deliberately exposes a narrow patch surface instead of arbitrary config
/// editing, preserving secret fields from the stored channel config.
pub struct UpdateChannelSettingsTool {
    channel_service: Arc<dyn ChannelService>,
    channel_store: Option<Arc<dyn ChannelStore>>,
}

impl UpdateChannelSettingsTool {
    pub fn new(
        channel_service: Arc<dyn ChannelService>,
        channel_store: Option<Arc<dyn ChannelStore>>,
    ) -> Self {
        Self {
            channel_service,
            channel_store,
        }
    }
}

#[async_trait]
impl AgentTool for UpdateChannelSettingsTool {
    fn name(&self) -> &str {
        "update_channel_settings"
    }

    fn description(&self) -> &str {
        "Safely update non-secret settings for a configured channel account after the user explicitly asks for a channel config change. Supports access policies, allowlists, default model routing, and selected per-channel overrides. Do not use this for tokens, secrets, or arbitrary config edits."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["account_id", "settings"],
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Configured channel account identifier to update."
                },
                "type": {
                    "type": "string",
                    "enum": ["telegram", "discord", "slack", "msteams", "whatsapp"],
                    "description": "Optional explicit channel type hint if account ids might overlap."
                },
                "settings": {
                    "type": "object",
                    "description": "Safe channel settings patch. Only include fields you want to change.",
                    "properties": {
                        "dm_policy": {
                            "type": "string",
                            "enum": ["open", "allowlist", "disabled"]
                        },
                        "group_policy": {
                            "type": "string",
                            "enum": ["open", "allowlist", "disabled"]
                        },
                        "mention_mode": {
                            "type": "string",
                            "enum": ["mention", "always", "none"],
                            "description": "Supported by Telegram, Discord, Slack, and Microsoft Teams."
                        },
                        "model": {
                            "type": ["string", "null"],
                            "description": "Set the default model id for this account, or null to clear it."
                        },
                        "model_provider": {
                            "type": ["string", "null"],
                            "description": "Set the provider name paired with `model`, or null to clear it."
                        },
                        "allowlist_add": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Identifiers to add to the DM allowlist."
                        },
                        "allowlist_remove": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Identifiers to remove from the DM allowlist."
                        },
                        "group_allowlist_add": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Group, guild, or channel ids to add to the group allowlist."
                        },
                        "group_allowlist_remove": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Group, guild, or channel ids to remove from the group allowlist."
                        },
                        "otp_self_approval": {
                            "type": "boolean",
                            "description": "Supported by Telegram, Discord, and WhatsApp."
                        },
                        "otp_cooldown_secs": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Supported by Telegram, Discord, and WhatsApp."
                        },
                        "reply_to_message": {
                            "type": "boolean",
                            "description": "Supported by Telegram and Discord."
                        },
                        "thread_replies": {
                            "type": "boolean",
                            "description": "Supported by Slack."
                        },
                        "stream_mode": {
                            "type": "string",
                            "enum": ["edit_in_place", "native", "off"],
                            "description": "Supported by Telegram (`edit_in_place`, `off`) and Slack (`edit_in_place`, `native`, `off`)."
                        },
                        "channel_override": {
                            "type": "object",
                            "description": "Set or clear a model override for a specific Telegram/Discord/Slack channel id.",
                            "required": ["target_id"],
                            "properties": {
                                "target_id": { "type": "string" },
                                "model": { "type": ["string", "null"] },
                                "model_provider": { "type": ["string", "null"] },
                                "clear": { "type": "boolean", "default": false }
                            }
                        },
                        "user_override": {
                            "type": "object",
                            "description": "Set or clear a model override for a specific Telegram/Discord/Slack user id.",
                            "required": ["target_id"],
                            "properties": {
                                "target_id": { "type": "string" },
                                "model": { "type": ["string", "null"] },
                                "model_provider": { "type": ["string", "null"] },
                                "clear": { "type": "boolean", "default": false }
                            }
                        }
                    }
                }
            }
        })
    }

    #[tracing::instrument(skip(self, params))]
    async fn execute(&self, params: Value) -> Result<Value> {
        let Some(store) = self.channel_store.as_ref() else {
            return Err(anyhow!("channel store is not available"));
        };

        let account_id = params
            .get("account_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing 'account_id'"))?;
        let explicit_type = params
            .get("type")
            .and_then(Value::as_str)
            .map(str::parse::<ChannelType>)
            .transpose()
            .map_err(|e| anyhow!(e.to_string()))?;
        let settings = params
            .get("settings")
            .cloned()
            .ok_or_else(|| anyhow!("missing 'settings'"))?;
        let patch: ChannelSettingsPatch =
            serde_json::from_value(settings).map_err(|e| anyhow!("invalid 'settings': {e}"))?;

        let stored = load_stored_channel(store.as_ref(), account_id, explicit_type).await?;
        let channel_type = stored
            .channel_type
            .parse::<ChannelType>()
            .map_err(|e| anyhow!(e.to_string()))?;
        let mut merged = stored.config.clone();
        let config = merged
            .as_object_mut()
            .ok_or_else(|| anyhow!("stored config for '{account_id}' is not an object"))?;
        let changes = apply_channel_settings_patch(config, channel_type, &patch)?;

        let update_result = self
            .channel_service
            .update(json!({
                "account_id": stored.account_id,
                "type": channel_type.as_str(),
                "config": merged,
            }))
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        let mut result = json!({
            "ok": true,
            "account_id": account_id,
            "type": channel_type.as_str(),
            "changes": changes,
        });
        if let Some(warning) = update_result.get("warning").cloned() {
            result["warning"] = warning;
        }
        Ok(result)
    }
}

#[tracing::instrument(skip(store), fields(account_id, explicit_type = ?explicit_type))]
async fn load_stored_channel(
    store: &dyn ChannelStore,
    account_id: &str,
    explicit_type: Option<ChannelType>,
) -> Result<StoredChannel> {
    if let Some(channel_type) = explicit_type {
        return store
            .get(channel_type.as_str(), account_id)
            .await
            .map_err(|e| anyhow!(e.to_string()))?
            .ok_or_else(|| {
                anyhow!(
                    "channel account '{account_id}' not found for type '{}'",
                    channel_type.as_str()
                )
            });
    }

    let mut matches = Vec::new();
    for channel_type in ChannelType::ALL {
        if let Some(stored) = store
            .get(channel_type.as_str(), account_id)
            .await
            .map_err(|e| anyhow!(e.to_string()))?
        {
            matches.push(stored);
        }
    }

    match matches.len() {
        0 => Err(anyhow!("channel account '{account_id}' not found")),
        1 => Ok(matches.remove(0)),
        _ => Err(anyhow!(
            "channel account '{account_id}' exists in multiple channel types, pass explicit 'type'"
        )),
    }
}

fn apply_channel_settings_patch(
    config: &mut Map<String, Value>,
    channel_type: ChannelType,
    patch: &ChannelSettingsPatch,
) -> Result<Vec<String>> {
    let mut changes = Vec::new();

    if let Some(dm_policy) = &patch.dm_policy {
        config.insert("dm_policy".into(), json!(dm_policy));
        changes.push("dm_policy".to_string());
    }
    if let Some(group_policy) = &patch.group_policy {
        config.insert("group_policy".into(), json!(group_policy));
        changes.push("group_policy".to_string());
    }
    if let Some(mention_mode) = &patch.mention_mode {
        ensure_supported(
            channel_type,
            "mention_mode",
            supports_mention_mode(channel_type),
        )?;
        config.insert("mention_mode".into(), json!(mention_mode));
        changes.push("mention_mode".to_string());
    }
    if let Some(model) = &patch.model {
        set_optional_string(config, "model", model);
        changes.push("model".to_string());
    }
    if let Some(model_provider) = &patch.model_provider {
        set_optional_string(config, "model_provider", model_provider);
        changes.push("model_provider".to_string());
    }
    if let Some(enabled) = patch.otp_self_approval {
        ensure_supported(
            channel_type,
            "otp_self_approval",
            supports_otp_settings(channel_type),
        )?;
        config.insert("otp_self_approval".into(), json!(enabled));
        changes.push("otp_self_approval".to_string());
    }
    if let Some(cooldown) = patch.otp_cooldown_secs {
        ensure_supported(
            channel_type,
            "otp_cooldown_secs",
            supports_otp_settings(channel_type),
        )?;
        config.insert("otp_cooldown_secs".into(), json!(cooldown));
        changes.push("otp_cooldown_secs".to_string());
    }
    if let Some(reply_to_message) = patch.reply_to_message {
        ensure_supported(
            channel_type,
            "reply_to_message",
            supports_reply_to_message(channel_type),
        )?;
        config.insert("reply_to_message".into(), json!(reply_to_message));
        changes.push("reply_to_message".to_string());
    }
    if let Some(thread_replies) = patch.thread_replies {
        ensure_supported(
            channel_type,
            "thread_replies",
            supports_thread_replies(channel_type),
        )?;
        config.insert("thread_replies".into(), json!(thread_replies));
        changes.push("thread_replies".to_string());
    }
    if let Some(stream_mode) = &patch.stream_mode {
        validate_stream_mode(channel_type, stream_mode)?;
        config.insert("stream_mode".into(), json!(stream_mode));
        changes.push("stream_mode".to_string());
    }
    if update_string_array(
        config,
        "allowlist",
        &patch.allowlist_add,
        &patch.allowlist_remove,
    )? {
        changes.push("allowlist".to_string());
    }
    if update_string_array(
        config,
        group_allowlist_key(channel_type),
        &patch.group_allowlist_add,
        &patch.group_allowlist_remove,
    )? {
        changes.push("group_allowlist".to_string());
    }
    if let Some(override_patch) = &patch.channel_override {
        ensure_supported(
            channel_type,
            "channel_override",
            supports_model_overrides(channel_type),
        )?;
        apply_model_override_patch(config, "channel_overrides", override_patch)?;
        changes.push(format!("channel_override:{}", override_patch.target_id));
    }
    if let Some(override_patch) = &patch.user_override {
        ensure_supported(
            channel_type,
            "user_override",
            supports_model_overrides(channel_type),
        )?;
        apply_model_override_patch(config, "user_overrides", override_patch)?;
        changes.push(format!("user_override:{}", override_patch.target_id));
    }

    if changes.is_empty() {
        return Err(anyhow!("no channel settings were provided"));
    }

    Ok(changes)
}

fn ensure_supported(channel_type: ChannelType, field: &str, supported: bool) -> Result<()> {
    if supported {
        Ok(())
    } else {
        Err(anyhow!(
            "'{field}' is not supported for channel type '{}'",
            channel_type.as_str()
        ))
    }
}

fn supports_mention_mode(channel_type: ChannelType) -> bool {
    !matches!(channel_type, ChannelType::Whatsapp)
}

fn supports_otp_settings(channel_type: ChannelType) -> bool {
    matches!(
        channel_type,
        ChannelType::Telegram | ChannelType::Discord | ChannelType::Whatsapp
    )
}

fn supports_reply_to_message(channel_type: ChannelType) -> bool {
    matches!(channel_type, ChannelType::Telegram | ChannelType::Discord)
}

fn supports_thread_replies(channel_type: ChannelType) -> bool {
    matches!(channel_type, ChannelType::Slack)
}

fn supports_model_overrides(channel_type: ChannelType) -> bool {
    matches!(
        channel_type,
        ChannelType::Telegram | ChannelType::Discord | ChannelType::Slack
    )
}

fn validate_stream_mode(
    channel_type: ChannelType,
    stream_mode: &ChannelSettingsStreamMode,
) -> Result<()> {
    let valid_values: Option<&[ChannelSettingsStreamMode]> = match channel_type {
        ChannelType::Telegram => Some(&[
            ChannelSettingsStreamMode::EditInPlace,
            ChannelSettingsStreamMode::Off,
        ]),
        ChannelType::Slack => Some(&[
            ChannelSettingsStreamMode::EditInPlace,
            ChannelSettingsStreamMode::Native,
            ChannelSettingsStreamMode::Off,
        ]),
        _ => None,
    };

    match valid_values {
        None => Err(anyhow!(
            "'stream_mode' is not supported for channel type '{}'",
            channel_type.as_str()
        )),
        Some(valid_values) if valid_values.contains(stream_mode) => Ok(()),
        Some(valid_values) => Err(anyhow!(
            "invalid stream_mode '{}' for channel type '{}'; valid values are: {}",
            stream_mode.as_str(),
            channel_type.as_str(),
            valid_values
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn group_allowlist_key(channel_type: ChannelType) -> &'static str {
    match channel_type {
        ChannelType::Telegram | ChannelType::Whatsapp | ChannelType::MsTeams => "group_allowlist",
        ChannelType::Discord => "guild_allowlist",
        ChannelType::Slack => "channel_allowlist",
        _ => "group_allowlist",
    }
}

fn set_optional_string(config: &mut Map<String, Value>, key: &str, value: &Option<String>) {
    match value {
        Some(value) => {
            config.insert(key.to_string(), Value::String(value.clone()));
        },
        None => {
            config.remove(key);
        },
    }
}

fn update_string_array(
    config: &mut Map<String, Value>,
    key: &str,
    additions: &[String],
    removals: &[String],
) -> Result<bool> {
    if additions.is_empty() && removals.is_empty() {
        return Ok(false);
    }

    let entry = config
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let values = entry
        .as_array_mut()
        .ok_or_else(|| anyhow!("'{key}' must be an array"))?;

    let mut items = Vec::with_capacity(values.len());
    for value in values.iter() {
        let item = value
            .as_str()
            .ok_or_else(|| anyhow!("'{key}' must only contain strings"))?;
        items.push(item.to_string());
    }
    let before_snapshot = items.clone();

    for removal in removals {
        let removal_lower = removal.to_lowercase();
        items.retain(|item| item.to_lowercase() != removal_lower);
    }
    for addition in additions {
        let addition_lower = addition.to_lowercase();
        if !items
            .iter()
            .any(|item| item.to_lowercase() == addition_lower)
        {
            items.push(addition.clone());
        }
    }

    let changed = items != before_snapshot;
    *values = items.into_iter().map(Value::String).collect();
    Ok(changed)
}

fn apply_model_override_patch(
    config: &mut Map<String, Value>,
    key: &str,
    patch: &ModelOverridePatch,
) -> Result<()> {
    if patch.target_id.trim().is_empty() {
        return Err(anyhow!("'{key}.target_id' cannot be empty"));
    }

    let entry = config
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let overrides = entry
        .as_object_mut()
        .ok_or_else(|| anyhow!("'{key}' must be an object"))?;

    if patch.clear {
        overrides.remove(&patch.target_id);
        return Ok(());
    }

    if patch.model.is_none() && patch.model_provider.is_none() {
        return Err(anyhow!(
            "'{key}' for '{}' must set 'model' or 'model_provider', or use clear=true",
            patch.target_id
        ));
    }

    let mut override_value = match overrides.get(&patch.target_id) {
        Some(value) => value
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow!("'{key}.{}' must be an object", patch.target_id))?,
        None => Map::new(),
    };

    if let Some(model) = &patch.model {
        set_optional_string(&mut override_value, "model", model);
    }
    if let Some(model_provider) = &patch.model_provider {
        set_optional_string(&mut override_value, "model_provider", model_provider);
    }

    if override_value.is_empty() {
        overrides.remove(&patch.target_id);
    } else {
        overrides.insert(patch.target_id.clone(), Value::Object(override_value));
    }

    Ok(())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::services::{ServiceError, ServiceResult},
        async_trait::async_trait,
        std::collections::HashMap,
        tokio::sync::Mutex,
    };

    struct RecordingChannelService {
        sent: Mutex<Option<Value>>,
        updated: Mutex<Option<Value>>,
        update_result: Value,
    }

    impl RecordingChannelService {
        fn new() -> Self {
            Self {
                sent: Mutex::new(None),
                updated: Mutex::new(None),
                update_result: json!({ "ok": true }),
            }
        }
    }

    #[async_trait]
    impl ChannelService for RecordingChannelService {
        async fn status(&self) -> ServiceResult {
            Ok(json!({}))
        }

        async fn logout(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn send(&self, params: Value) -> ServiceResult {
            *self.sent.lock().await = Some(params.clone());
            Ok(json!({ "ok": true, "echo": params }))
        }

        async fn add(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn remove(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn update(&self, params: Value) -> ServiceResult {
            *self.updated.lock().await = Some(params);
            Ok(self.update_result.clone())
        }

        async fn retry_ownership(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn senders_list(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn sender_approve(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn sender_deny(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }
    }

    struct MemoryChannelStore {
        channels: Mutex<HashMap<(String, String), StoredChannel>>,
    }

    impl MemoryChannelStore {
        fn new(channels: Vec<StoredChannel>) -> Self {
            let mut map = HashMap::new();
            for channel in channels {
                map.insert(
                    (channel.channel_type.clone(), channel.account_id.clone()),
                    channel,
                );
            }
            Self {
                channels: Mutex::new(map),
            }
        }
    }

    #[async_trait]
    impl ChannelStore for MemoryChannelStore {
        async fn list(&self) -> moltis_channels::Result<Vec<StoredChannel>> {
            Ok(self.channels.lock().await.values().cloned().collect())
        }

        async fn get(
            &self,
            channel_type: &str,
            account_id: &str,
        ) -> moltis_channels::Result<Option<StoredChannel>> {
            Ok(self
                .channels
                .lock()
                .await
                .get(&(channel_type.to_string(), account_id.to_string()))
                .cloned())
        }

        async fn upsert(&self, channel: StoredChannel) -> moltis_channels::Result<()> {
            self.channels.lock().await.insert(
                (channel.channel_type.clone(), channel.account_id.clone()),
                channel,
            );
            Ok(())
        }

        async fn delete(
            &self,
            channel_type: &str,
            account_id: &str,
        ) -> moltis_channels::Result<()> {
            self.channels
                .lock()
                .await
                .remove(&(channel_type.to_string(), account_id.to_string()));
            Ok(())
        }
    }

    fn stored_channel(account_id: &str, channel_type: &str, config: Value) -> StoredChannel {
        StoredChannel {
            account_id: account_id.to_string(),
            channel_type: channel_type.to_string(),
            config,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[tokio::test]
    async fn send_message_tool_forwards_params_to_channel_service() {
        let service = Arc::new(RecordingChannelService::new());
        let tool = SendMessageTool::new(service.clone() as Arc<dyn ChannelService>);

        let input = json!({
            "account_id": "bot-alpha",
            "to": "12345",
            "text": "ping",
            "type": "telegram",
            "reply_to": "42",
            "silent": true
        });
        let out = tool
            .execute(input.clone())
            .await
            .expect("send_message execute");

        assert_eq!(out.get("ok").and_then(Value::as_bool), Some(true));
        let sent = service.sent.lock().await.clone().expect("captured payload");
        assert_eq!(sent, input);
    }

    #[tokio::test]
    async fn send_message_tool_propagates_service_errors() {
        struct FailingChannelService;

        #[async_trait]
        impl ChannelService for FailingChannelService {
            async fn status(&self) -> ServiceResult {
                Ok(json!({}))
            }

            async fn logout(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn send(&self, _: Value) -> ServiceResult {
                Err(ServiceError::message("missing 'text' (or alias 'message')"))
            }

            async fn add(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn remove(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn update(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn retry_ownership(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn senders_list(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn sender_approve(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn sender_deny(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }
        }

        let tool = SendMessageTool::new(Arc::new(FailingChannelService));
        let err = tool
            .execute(json!({
                "account_id": "bot-alpha",
                "to": "12345"
            }))
            .await
            .expect_err("expected validation error");
        assert!(err.to_string().contains("missing"));
    }
    #[tokio::test]
    async fn update_channel_settings_merges_patch_and_preserves_secrets() {
        let service = Arc::new(RecordingChannelService::new());
        let store = Arc::new(MemoryChannelStore::new(vec![stored_channel(
            "bot-alpha",
            "telegram",
            json!({
                "token": "secret-token",
                "dm_policy": "allowlist",
                "group_policy": "open",
                "mention_mode": "mention",
                "allowlist": ["alice"],
                "group_allowlist": ["chat-1"],
                "model": "old-model",
                "model_provider": "anthropic",
                "reply_to_message": false
            }),
        )]));
        let tool = UpdateChannelSettingsTool::new(
            service.clone() as Arc<dyn ChannelService>,
            Some(store as Arc<dyn ChannelStore>),
        );

        let out = tool
            .execute(json!({
                "account_id": "bot-alpha",
                "type": "telegram",
                "settings": {
                    "model": "new-model",
                    "allowlist_add": ["bob"],
                    "allowlist_remove": ["alice"],
                    "reply_to_message": true
                }
            }))
            .await
            .expect("update_channel_settings execute");

        assert_eq!(out.get("ok").and_then(Value::as_bool), Some(true));
        let updated = service
            .updated
            .lock()
            .await
            .clone()
            .expect("captured update payload");
        assert_eq!(updated["account_id"], "bot-alpha");
        assert_eq!(updated["type"], "telegram");
        assert_eq!(updated["config"]["token"], "secret-token");
        assert_eq!(updated["config"]["model"], "new-model");
        assert_eq!(updated["config"]["reply_to_message"], true);
        assert_eq!(updated["config"]["allowlist"], json!(["bob"]));
    }

    #[tokio::test]
    async fn update_channel_settings_rejects_unsupported_fields() {
        let service = Arc::new(RecordingChannelService::new());
        let store = Arc::new(MemoryChannelStore::new(vec![stored_channel(
            "wa-main",
            "whatsapp",
            json!({
                "paired": true,
                "allowlist": [],
                "group_allowlist": []
            }),
        )]));
        let tool = UpdateChannelSettingsTool::new(
            service as Arc<dyn ChannelService>,
            Some(store as Arc<dyn ChannelStore>),
        );

        let err = tool
            .execute(json!({
                "account_id": "wa-main",
                "settings": {
                    "mention_mode": "always"
                }
            }))
            .await
            .expect_err("mention_mode should be rejected for WhatsApp");
        assert!(err.to_string().contains("not supported"));
    }

    #[tokio::test]
    async fn update_channel_settings_merges_partial_model_override() {
        let service = Arc::new(RecordingChannelService::new());
        let store = Arc::new(MemoryChannelStore::new(vec![stored_channel(
            "discord-main",
            "discord",
            json!({
                "token": "discord-secret",
                "allowlist": [],
                "guild_allowlist": [],
                "channel_overrides": {
                    "chan-1": {
                        "model": "old-model",
                        "model_provider": "anthropic"
                    }
                }
            }),
        )]));
        let tool = UpdateChannelSettingsTool::new(
            service.clone() as Arc<dyn ChannelService>,
            Some(store as Arc<dyn ChannelStore>),
        );

        tool.execute(json!({
            "account_id": "discord-main",
            "settings": {
                "channel_override": {
                    "target_id": "chan-1",
                    "model": "new-model"
                }
            }
        }))
        .await
        .expect("channel override update");

        let updated = service
            .updated
            .lock()
            .await
            .clone()
            .expect("captured update payload");
        assert_eq!(
            updated["config"]["channel_overrides"]["chan-1"]["model"],
            "new-model"
        );
        assert_eq!(
            updated["config"]["channel_overrides"]["chan-1"]["model_provider"],
            "anthropic"
        );
    }

    #[test]
    fn update_string_array_ignores_noop_changes() {
        let mut config = Map::from_iter([("allowlist".to_string(), json!(["alice", "bob"]))]);

        let changed = update_string_array(&mut config, "allowlist", &[String::from("ALICE")], &[
            String::from("carol"),
        ])
        .expect("noop allowlist update");

        assert!(!changed);
        assert_eq!(config["allowlist"], json!(["alice", "bob"]));
    }

    #[tokio::test]
    async fn update_channel_settings_rejects_invalid_stream_mode_for_supported_channel() {
        let service = Arc::new(RecordingChannelService::new());
        let store = Arc::new(MemoryChannelStore::new(vec![stored_channel(
            "tg-main",
            "telegram",
            json!({
                "token": "telegram-secret",
                "allowlist": [],
                "group_allowlist": []
            }),
        )]));
        let tool = UpdateChannelSettingsTool::new(
            service as Arc<dyn ChannelService>,
            Some(store as Arc<dyn ChannelStore>),
        );

        let err = tool
            .execute(json!({
                "account_id": "tg-main",
                "settings": {
                    "stream_mode": "native"
                }
            }))
            .await
            .expect_err("telegram should reject native stream mode");

        assert!(err.to_string().contains("invalid stream_mode 'native'"));
        assert!(
            err.to_string()
                .contains("valid values are: edit_in_place, off")
        );
    }

    #[tokio::test]
    async fn send_message_tool_schema_lists_matrix_and_slack() {
        let tool = SendMessageTool::new(Arc::new(RecordingChannelService::new()));
        let schema = tool.parameters_schema();
        let channel_types = schema["properties"]["type"]["enum"]
            .as_array()
            .expect("channel type enum");

        assert!(channel_types.iter().any(|value| value == "slack"));
        assert!(channel_types.iter().any(|value| value == "matrix"));
    }
}
