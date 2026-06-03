#![allow(clippy::unwrap_used)]

use {super::*, crate::channel_events::commands::formatting::unique_providers};

use {
    crate::{
        auth::{AuthMode, ResolvedAuth},
        services::{ChannelService, GatewayServices, ServiceResult},
    },
    async_trait::async_trait,
    moltis_channels::{
        ChannelOutbound, ChannelPlugin, ChannelRegistry, ChannelStatus, ChannelStreamOutbound,
        ChannelType,
        store::{ChannelStore, StoredChannel},
    },
    moltis_common::types::ReplyPayload,
    serde_json::Value,
    std::{collections::HashMap, sync::Arc},
    tokio::sync::RwLock,
};

struct MemoryChannelStore {
    channels: HashMap<(String, String), StoredChannel>,
}

#[async_trait]
impl ChannelStore for MemoryChannelStore {
    async fn list(&self) -> moltis_channels::Result<Vec<StoredChannel>> {
        Ok(self.channels.values().cloned().collect())
    }

    async fn get(
        &self,
        channel_type: &str,
        account_id: &str,
    ) -> moltis_channels::Result<Option<StoredChannel>> {
        Ok(self
            .channels
            .get(&(channel_type.to_string(), account_id.to_string()))
            .cloned())
    }

    async fn upsert(&self, _channel: StoredChannel) -> moltis_channels::Result<()> {
        Ok(())
    }

    async fn delete(&self, _channel_type: &str, _account_id: &str) -> moltis_channels::Result<()> {
        Ok(())
    }
}

struct StatusPanicsChannelService;

#[async_trait]
impl ChannelService for StatusPanicsChannelService {
    async fn status(&self) -> ServiceResult {
        panic!("channel status should not be probed for stored config")
    }

    async fn logout(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn send(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn add(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn remove(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn update(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn retry_ownership(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn senders_list(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn sender_approve(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn sender_deny(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }
}

struct ConfigPlugin {
    id: String,
    accounts: std::sync::Mutex<HashMap<String, Value>>,
}

impl ConfigPlugin {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            accounts: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ChannelPlugin for ConfigPlugin {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.id
    }

    async fn start_account(
        &mut self,
        account_id: &str,
        config: Value,
    ) -> moltis_channels::Result<()> {
        self.accounts
            .lock()
            .unwrap()
            .insert(account_id.to_string(), config);
        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> moltis_channels::Result<()> {
        self.accounts.lock().unwrap().remove(account_id);
        Ok(())
    }

    fn outbound(&self) -> Option<&dyn ChannelOutbound> {
        None
    }

    fn status(&self) -> Option<&dyn ChannelStatus> {
        None
    }

    fn has_account(&self, account_id: &str) -> bool {
        self.accounts.lock().unwrap().contains_key(account_id)
    }

    fn account_ids(&self) -> Vec<String> {
        self.accounts.lock().unwrap().keys().cloned().collect()
    }

    fn account_config(
        &self,
        _account_id: &str,
    ) -> Option<Box<dyn moltis_channels::config_view::ChannelConfigView>> {
        None
    }

    fn update_account_config(
        &self,
        account_id: &str,
        config: Value,
    ) -> moltis_channels::Result<()> {
        if let Some(account_config) = self.accounts.lock().unwrap().get_mut(account_id) {
            *account_config = config;
            Ok(())
        } else {
            Err(moltis_channels::Error::UnknownAccount {
                account_id: account_id.to_string(),
            })
        }
    }

    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(NullOutbound)
    }

    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(NullStreamOutbound)
    }

    fn account_config_json(&self, account_id: &str) -> Option<Value> {
        self.accounts.lock().unwrap().get(account_id).cloned()
    }
}

struct NullOutbound;

#[async_trait]
impl ChannelOutbound for NullOutbound {
    async fn send_text(
        &self,
        _account_id: &str,
        _to: &str,
        _text: &str,
        _reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        Ok(())
    }

    async fn send_media(
        &self,
        _account_id: &str,
        _to: &str,
        _payload: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        Ok(())
    }
}

struct NullStreamOutbound;

#[async_trait]
impl ChannelStreamOutbound for NullStreamOutbound {
    async fn send_stream(
        &self,
        _account_id: &str,
        _to: &str,
        _reply_to: Option<&str>,
        mut stream: moltis_channels::StreamReceiver,
    ) -> moltis_channels::Result<()> {
        while stream.recv().await.is_some() {}
        Ok(())
    }
}

fn test_state_with_channel_services(
    store: Option<Arc<dyn ChannelStore>>,
    registry: Option<Arc<ChannelRegistry>>,
) -> Arc<GatewayState> {
    let mut services = GatewayServices::noop();
    if let Some(store) = store {
        services = services.with_channel_store(store);
    }
    if let Some(registry) = registry {
        services = services.with_channel_registry(registry);
    }
    services.channel = Arc::new(StatusPanicsChannelService);

    GatewayState::new(
        ResolvedAuth {
            mode: AuthMode::Token,
            token: None,
            password: None,
        },
        services,
    )
}

fn test_state_with_channel_store(store: Arc<dyn ChannelStore>) -> Arc<GatewayState> {
    test_state_with_channel_services(Some(store), None)
}

fn test_state_with_channel_store_and_registry(
    store: Arc<dyn ChannelStore>,
    registry: Arc<ChannelRegistry>,
) -> Arc<GatewayState> {
    test_state_with_channel_services(Some(store), Some(registry))
}

async fn test_registry_with_account_config(config: Value) -> Arc<ChannelRegistry> {
    let mut registry = ChannelRegistry::new();
    registry
        .register(Arc::new(RwLock::new(ConfigPlugin::new("telegram"))))
        .await;
    let registry = Arc::new(registry);
    registry
        .start_account("telegram", "bot1", config)
        .await
        .unwrap();
    registry
}

async fn sqlite_session_metadata() -> SqliteSessionMetadata {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    moltis_projects::run_migrations(&pool).await.unwrap();
    SqliteSessionMetadata::init(&pool).await.unwrap();
    SqliteSessionMetadata::new(pool)
}

#[test]
fn channel_event_serialization() {
    let event = ChannelEvent::InboundMessage {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        peer_id: "123".into(),
        username: Some("alice".into()),
        sender_name: Some("Alice".into()),
        message_count: Some(5),
        access_granted: true,
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["kind"], "inbound_message");
    assert_eq!(json["channel_type"], "telegram");
    assert_eq!(json["account_id"], "bot1");
    assert_eq!(json["peer_id"], "123");
    assert_eq!(json["username"], "alice");
    assert_eq!(json["sender_name"], "Alice");
    assert_eq!(json["message_count"], 5);
    assert_eq!(json["access_granted"], true);
}

#[test]
fn channel_session_key_format() {
    let target = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "12345".into(),
        message_id: None,
        thread_id: None,
        sender_id: None,
        activity_log: ActivityLogMode::All,
    };
    assert_eq!(default_channel_session_key(&target), "telegram:bot1:12345");
}

#[test]
fn channel_session_key_group() {
    let target = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "-100999".into(),
        message_id: None,
        thread_id: None,
        sender_id: None,
        activity_log: ActivityLogMode::All,
    };
    assert_eq!(
        default_channel_session_key(&target),
        "telegram:bot1:-100999"
    );
}

#[test]
fn channel_session_key_forum_topic() {
    let target = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "-100999".into(),
        message_id: None,
        thread_id: Some("42".into()),
        sender_id: None,
        activity_log: ActivityLogMode::All,
    };
    assert_eq!(
        default_channel_session_key(&target),
        "telegram:bot1:-100999:42"
    );
}

#[test]
fn channel_activity_log_resolution_prefers_user_channel_account_and_fallback() {
    let config = serde_json::json!({
        "activity_log": "errors_only",
        "channel_overrides": {
            "-100999": { "activity_log": "off" }
        },
        "user_overrides": {
            "123": { "activity_log": "all" },
            "456": { "activity_log": "verbose" }
        }
    });

    assert_eq!(
        resolve_channel_activity_log_from_config(&config, "-100999", Some("123")),
        ActivityLogMode::All
    );
    assert_eq!(
        resolve_channel_activity_log_from_config(&config, "-100999", Some("789")),
        ActivityLogMode::Off
    );
    assert_eq!(
        resolve_channel_activity_log_from_config(&config, "-100111", Some("789")),
        ActivityLogMode::ErrorsOnly
    );
    assert_eq!(
        resolve_channel_activity_log_from_config(&config, "-100111", Some("456")),
        ActivityLogMode::ErrorsOnly
    );
    assert_eq!(
        resolve_channel_activity_log_from_config(&serde_json::json!({}), "chat", None),
        ActivityLogMode::All
    );
}

#[tokio::test]
async fn channel_activity_log_resolution_reads_store_without_status_probe() {
    let store = MemoryChannelStore {
        channels: HashMap::from([(
            ("telegram".to_string(), "bot1".to_string()),
            StoredChannel {
                account_id: "bot1".to_string(),
                channel_type: "telegram".to_string(),
                config: serde_json::json!({
                    "activity_log": "errors_only",
                    "channel_overrides": {
                        "-100999": { "activity_log": "off" }
                    },
                    "user_overrides": {
                        "123": { "activity_log": "all" }
                    }
                }),
                created_at: 1,
                updated_at: 1,
            },
        )]),
    };
    let state = test_state_with_channel_store(Arc::new(store));
    let target = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "-100999".into(),
        message_id: None,
        thread_id: None,
        sender_id: Some("789".into()),
        activity_log: ActivityLogMode::All,
    };

    let mode = resolve_channel_activity_log(&state, &target, target.sender_id.as_deref()).await;

    assert_eq!(mode, ActivityLogMode::Off);
}

#[tokio::test]
async fn channel_activity_log_resolution_falls_back_to_live_config_without_status_probe() {
    let store = MemoryChannelStore {
        channels: HashMap::new(),
    };
    let registry = test_registry_with_account_config(serde_json::json!({
        "activity_log": "errors_only",
        "channel_overrides": {
            "-100999": { "activity_log": "off" }
        },
        "user_overrides": {
            "123": { "activity_log": "all" }
        }
    }))
    .await;
    let state = test_state_with_channel_store_and_registry(Arc::new(store), registry);
    let target = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "-100111".into(),
        message_id: None,
        thread_id: None,
        sender_id: Some("789".into()),
        activity_log: ActivityLogMode::All,
    };

    let mode = resolve_channel_activity_log(&state, &target, target.sender_id.as_deref()).await;

    assert_eq!(mode, ActivityLogMode::ErrorsOnly);
}

#[tokio::test]
async fn channel_activity_log_resolution_prefers_live_config_for_active_accounts() {
    let store = MemoryChannelStore {
        channels: HashMap::from([(
            ("telegram".to_string(), "bot1".to_string()),
            StoredChannel {
                account_id: "bot1".to_string(),
                channel_type: "telegram".to_string(),
                config: serde_json::json!({
                    "activity_log": "all",
                    "channel_overrides": {
                        "-100999": { "activity_log": "all" }
                    }
                }),
                created_at: 1,
                updated_at: 1,
            },
        )]),
    };
    let registry = test_registry_with_account_config(serde_json::json!({
        "activity_log": "errors_only",
        "channel_overrides": {
            "-100999": { "activity_log": "off" }
        }
    }))
    .await;
    let state = test_state_with_channel_store_and_registry(Arc::new(store), registry);
    let target = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "-100999".into(),
        message_id: None,
        thread_id: None,
        sender_id: Some("789".into()),
        activity_log: ActivityLogMode::All,
    };

    let mode = resolve_channel_activity_log(&state, &target, target.sender_id.as_deref()).await;

    assert_eq!(mode, ActivityLogMode::Off);
}

#[tokio::test]
async fn channel_session_defaults_fall_back_to_live_config_without_status_probe() {
    let store = MemoryChannelStore {
        channels: HashMap::new(),
    };
    let registry = test_registry_with_account_config(serde_json::json!({
        "model": "toml-model",
        "agent_id": "toml-agent",
        "channel_overrides": {
            "-100999": {
                "model": "channel-model",
                "agent_id": "channel-agent"
            }
        }
    }))
    .await;
    let state = test_state_with_channel_store_and_registry(Arc::new(store), registry);
    let target = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "-100999".into(),
        message_id: None,
        thread_id: None,
        sender_id: Some("789".into()),
        activity_log: ActivityLogMode::All,
    };

    let defaults =
        resolve_channel_session_defaults(&state, &target, target.sender_id.as_deref()).await;

    assert_eq!(defaults.model.as_deref(), Some("channel-model"));
    assert_eq!(defaults.agent_id.as_deref(), Some("channel-agent"));
}

#[tokio::test]
async fn handle_sessions_refreshes_target_binding_sender_id() {
    let metadata = sqlite_session_metadata().await;
    let state = GatewayState::new(
        ResolvedAuth {
            mode: AuthMode::Token,
            token: None,
            password: None,
        },
        GatewayServices::noop(),
    );

    let current_binding = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        chat_id: "-100999".into(),
        message_id: Some("1".into()),
        thread_id: None,
        sender_id: Some("old-user".into()),
        activity_log: ActivityLogMode::All,
    };
    let target_binding = ChannelReplyTarget {
        message_id: Some("2".into()),
        sender_id: Some("previous-user".into()),
        ..current_binding.clone()
    };
    metadata
        .upsert("telegram:bot1:-100999", Some("Session 1".into()))
        .await
        .unwrap();
    metadata
        .set_channel_binding(
            "telegram:bot1:-100999",
            Some(serde_json::to_string(&current_binding).unwrap()),
        )
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    metadata
        .upsert("session:old", Some("Session 2".into()))
        .await
        .unwrap();
    metadata
        .set_channel_binding(
            "session:old",
            Some(serde_json::to_string(&target_binding).unwrap()),
        )
        .await;

    let reply_to = ChannelReplyTarget {
        message_id: Some("99".into()),
        sender_id: Some("fresh-user".into()),
        ..current_binding
    };

    let result = commands::session_handlers::handle_sessions(
        &state,
        &metadata,
        "telegram:bot1:-100999",
        &reply_to,
        "2",
    )
    .await
    .unwrap();

    assert_eq!(result, "Switched to: Session 2");
    let updated = metadata.get("session:old").await.unwrap();
    let binding: ChannelReplyTarget =
        serde_json::from_str(&updated.channel_binding.unwrap()).unwrap();
    assert_eq!(binding.sender_id.as_deref(), Some("fresh-user"));
    assert_eq!(binding.message_id.as_deref(), Some("2"));
}

#[test]
fn channel_event_serialization_nulls() {
    let event = ChannelEvent::InboundMessage {
        channel_type: ChannelType::Telegram,
        account_id: "bot1".into(),
        peer_id: "123".into(),
        username: None,
        sender_name: None,
        message_count: None,
        access_granted: false,
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["kind"], "inbound_message");
    assert!(json["username"].is_null());
    assert_eq!(json["access_granted"], false);
}

#[test]
fn shell_mode_rewrite_plain_text() {
    assert_eq!(
        rewrite_for_shell_mode("uname -a").as_deref(),
        Some("/sh uname -a")
    );
}

#[test]
fn shell_mode_rewrite_skips_control_commands() {
    assert!(rewrite_for_shell_mode("/context").is_none());
    assert!(rewrite_for_shell_mode("/attach").is_none());
    assert!(rewrite_for_shell_mode("/sh uname -a").is_none());
}

#[test]
fn peek_and_stop_are_control_commands() {
    assert!(is_channel_control_command_name("peek"));
    assert!(is_channel_control_command_name("stop"));
    assert!(is_channel_control_command_name("attach"));
    assert!(is_channel_control_command_name("approvals"));
    assert!(is_channel_control_command_name("approve"));
    assert!(is_channel_control_command_name("deny"));
}

#[test]
fn shell_mode_rewrite_skips_peek_and_stop() {
    assert!(rewrite_for_shell_mode("/peek").is_none());
    assert!(rewrite_for_shell_mode("/stop").is_none());
}

// ── unique_providers ───────────────────────────────────────────

/// Regression test for GitHub issue #637: providers must be deduplicated
/// even when duplicates are not adjacent in the model list. Prior to the
/// fix, a bare `Vec::dedup` left non-consecutive duplicates in place,
/// surfacing as duplicate Telegram `/model` inline keyboard buttons.
#[test]
fn unique_providers_dedups_non_adjacent() {
    let models = vec![
        serde_json::json!({"id": "gpt-4o", "provider": "openai"}),
        serde_json::json!({"id": "claude-3.5", "provider": "anthropic"}),
        serde_json::json!({"id": "gpt-4o-mini", "provider": "openai"}),
        serde_json::json!({"id": "gemini-pro", "provider": "google"}),
        serde_json::json!({"id": "claude-3.7", "provider": "anthropic"}),
    ];
    let providers = unique_providers(&models);
    assert_eq!(providers, vec!["anthropic", "google", "openai"]);
}

#[test]
fn unique_providers_sorted_alphabetically() {
    let models = vec![
        serde_json::json!({"id": "m1", "provider": "zeta"}),
        serde_json::json!({"id": "m2", "provider": "alpha"}),
        serde_json::json!({"id": "m3", "provider": "mu"}),
    ];
    assert_eq!(unique_providers(&models), vec!["alpha", "mu", "zeta"]);
}

#[test]
fn unique_providers_skips_entries_without_provider() {
    let models = vec![
        serde_json::json!({"id": "m1"}),
        serde_json::json!({"id": "m2", "provider": "openai"}),
        serde_json::json!({"id": "m3", "provider": Value::Null}),
    ];
    assert_eq!(unique_providers(&models), vec!["openai"]);
}

#[test]
fn unique_providers_empty_input() {
    assert!(unique_providers(&[]).is_empty());
}

#[test]
fn attachable_session_filter_skips_archived_and_cron_sessions() {
    let archived = SessionEntry {
        id: "1".into(),
        key: "session:archived".into(),
        label: None,
        created_at: 0,
        updated_at: 0,
        message_count: 0,
        last_seen_message_count: 0,
        project_id: None,
        archived: true,
        worktree_branch: None,
        sandbox_enabled: None,
        sandbox_image: None,
        sandbox_backend: None,
        channel_binding: None,
        parent_session_key: None,
        fork_point: None,
        mcp_disabled: None,
        preview: None,
        agent_id: None,
        mode_id: None,
        model: None,
        node_id: None,
        external_agent_kind: None,
        external_session_id: None,
        version: 0,
    };
    let cron = SessionEntry {
        key: "cron:heartbeat".into(),
        archived: false,
        ..archived.clone()
    };
    let normal = SessionEntry {
        key: "session:normal".into(),
        archived: false,
        ..archived.clone()
    };

    assert!(!is_attachable_session(&cron));
    assert!(!is_attachable_session(&archived));
    assert!(is_attachable_session(&normal));
}

#[test]
fn format_attachable_sessions_shows_session_keys_when_labels_are_present() {
    let sessions = vec![
        SessionEntry {
            id: "1".into(),
            key: "main".into(),
            label: None,
            created_at: 0,
            updated_at: 0,
            message_count: 3,
            last_seen_message_count: 0,
            project_id: None,
            archived: false,
            worktree_branch: None,
            sandbox_enabled: None,
            sandbox_image: None,
            sandbox_backend: None,
            channel_binding: None,
            parent_session_key: None,
            fork_point: None,
            mcp_disabled: None,
            preview: None,
            agent_id: None,
            mode_id: None,
            model: None,
            node_id: None,
            external_agent_kind: None,
            external_session_id: None,
            version: 0,
        },
        SessionEntry {
            id: "2".into(),
            key: "session:abc".into(),
            label: Some("Build Fix".into()),
            created_at: 0,
            updated_at: 0,
            message_count: 12,
            last_seen_message_count: 0,
            project_id: None,
            archived: false,
            worktree_branch: None,
            sandbox_enabled: None,
            sandbox_image: None,
            sandbox_backend: None,
            channel_binding: None,
            parent_session_key: None,
            fork_point: None,
            mcp_disabled: None,
            preview: None,
            agent_id: None,
            mode_id: None,
            model: None,
            node_id: None,
            external_agent_kind: None,
            external_session_id: None,
            version: 0,
        },
    ];

    let rendered = format_attachable_sessions_list(&sessions, "session:abc");
    assert!(rendered.contains("1. main (3 msgs)"));
    assert!(rendered.contains("2. Build Fix [session:abc] (12 msgs) *"));
    assert!(rendered.contains("Use /attach N to move an existing session to this chat."));
}

#[test]
fn format_pending_approvals_renders_numbered_commands() {
    let approvals = vec![
        PendingApprovalView {
            id: "1".into(),
            command: "git status".into(),
            session_key: Some("session:a".into()),
        },
        PendingApprovalView {
            id: "2".into(),
            command: "rm -rf /tmp/build".into(),
            session_key: Some("session:a".into()),
        },
    ];

    let rendered = format_pending_approvals_list(&approvals);
    assert!(rendered.contains("1. `git status`"));
    assert!(rendered.contains("2. `rm -rf /tmp/build`"));
    assert!(rendered.contains("Use /approve N or /deny N."));
}

#[test]
fn channel_session_defaults_use_sender_override_for_group_commands() {
    let config = serde_json::json!({
        "model": "default-model",
        "agent_id": "default-agent",
        "channel_overrides": {
            "group-1": {
                "model": "channel-model",
                "agent_id": "channel-agent"
            }
        },
        "user_overrides": {
            "user-42": {
                "model": "user-model",
                "agent_id": "user-agent"
            }
        }
    });

    let defaults =
        resolve_channel_session_defaults_from_config(&config, "group-1", Some("user-42"));
    assert_eq!(defaults.model.as_deref(), Some("user-model"));
    assert_eq!(defaults.agent_id.as_deref(), Some("user-agent"));
}

#[test]
fn channel_session_defaults_use_chat_id_for_dm_commands() {
    let config = serde_json::json!({
        "model": "default-model",
        "agent_id": "default-agent",
        "user_overrides": {
            "dm-1": {
                "model": "dm-model",
                "agent_id": "dm-agent"
            }
        }
    });

    let defaults = resolve_channel_session_defaults_from_config(&config, "dm-1", Some("dm-1"));
    assert_eq!(defaults.model.as_deref(), Some("dm-model"));
    assert_eq!(defaults.agent_id.as_deref(), Some("dm-agent"));
}
