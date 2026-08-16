use std::{collections::HashSet, sync::Arc};

use {
    super::*,
    async_trait::async_trait,
    moltis_channels::{
        ChannelRegistry, ChannelType,
        plugin::{
            ChannelOutbound, ChannelPlugin, ChannelStreamOutbound, ChannelThreadContext,
            StreamReceiver, ThreadMessage,
        },
    },
    moltis_common::types::ReplyPayload,
    moltis_connectors::ProjectionConfig,
    secrecy::Secret,
    serde_json::Value,
};

#[derive(Default)]
struct FakeChannelHistory;

#[async_trait]
impl ChannelThreadContext for FakeChannelHistory {
    async fn fetch_thread_messages(
        &self,
        _account_id: &str,
        _channel_id: &str,
        _thread_id: &str,
        limit: usize,
    ) -> moltis_channels::Result<Vec<ThreadMessage>> {
        Ok(vec![ThreadMessage {
            message_id: "message-1".to_owned(),
            sender_id: "user-1".to_owned(),
            is_bot: false,
            text: "Quarterly planning".to_owned(),
            timestamp: "2026-08-12T10:00:00Z".to_owned(),
        }]
        .into_iter()
        .take(limit)
        .collect())
    }
}

#[async_trait]
impl ChannelOutbound for FakeChannelHistory {
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

#[async_trait]
impl ChannelStreamOutbound for FakeChannelHistory {
    async fn send_stream(
        &self,
        _account_id: &str,
        _to: &str,
        _reply_to: Option<&str>,
        _stream: StreamReceiver,
    ) -> moltis_channels::Result<()> {
        Ok(())
    }
}

struct FakeChannelPlugin {
    accounts: HashSet<String>,
    history: Arc<FakeChannelHistory>,
}

impl FakeChannelPlugin {
    fn new() -> Self {
        Self {
            accounts: HashSet::new(),
            history: Arc::new(FakeChannelHistory),
        }
    }
}

#[async_trait]
impl ChannelPlugin for FakeChannelPlugin {
    fn id(&self) -> &str {
        "slack"
    }

    fn name(&self) -> &str {
        "Slack"
    }

    async fn start_account(
        &mut self,
        account_id: &str,
        _config: Value,
    ) -> moltis_channels::Result<()> {
        self.accounts.insert(account_id.to_owned());
        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> moltis_channels::Result<()> {
        self.accounts.remove(account_id);
        Ok(())
    }

    fn outbound(&self) -> Option<&dyn ChannelOutbound> {
        Some(self.history.as_ref())
    }

    fn status(&self) -> Option<&dyn moltis_channels::plugin::ChannelStatus> {
        None
    }

    fn has_account(&self, account_id: &str) -> bool {
        self.accounts.contains(account_id)
    }

    fn account_ids(&self) -> Vec<String> {
        self.accounts.iter().cloned().collect()
    }

    async fn account_config(
        &self,
        _account_id: &str,
    ) -> Option<Box<dyn moltis_channels::config_view::ChannelConfigView>> {
        None
    }

    async fn update_account_config(
        &self,
        _account_id: &str,
        _config: Value,
    ) -> moltis_channels::Result<()> {
        Ok(())
    }

    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        self.history.clone()
    }

    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        self.history.clone()
    }

    fn thread_context(&self) -> Option<&dyn ChannelThreadContext> {
        Some(self.history.as_ref())
    }

    fn shared_thread_context(&self) -> Option<Arc<dyn ChannelThreadContext>> {
        Some(self.history.clone())
    }
}

#[test]
fn channel_history_configs_are_typed_bounded_and_secret_free() {
    let config = ChannelHistoryDatasetConfigView {
        schema_version: 1,
        channel_id: "C123".to_owned(),
        thread_id: "1712345678.000100".to_owned(),
        limit: 100,
    };
    validate_channel_dataset_config(&config, Some(30)).unwrap();
    assert!(
        validate_channel_dataset_config(
            &ChannelHistoryDatasetConfigView {
                limit: 201,
                ..config.clone()
            },
            None
        )
        .is_err()
    );

    let account = Account {
        id: "connector-1".to_owned(),
        kind: ConnectorKind::ChannelHistory,
        source_key: None,
        name: "Slack thread".to_owned(),
        config: serde_json::to_value(ChannelHistoryAccountConfig {
            channel_type: ChannelType::Slack,
            channel_account_id: "slack-work".to_owned(),
        })
        .unwrap(),
        enabled: true,
        created_at: OffsetDateTime::now_utc(),
        updated_at: OffsetDateTime::now_utc(),
    };
    let view = account_view(account).unwrap();
    assert_eq!(view.channel_type, Some(ChannelType::Slack));
    assert_eq!(view.channel_account_id.as_deref(), Some("slack-work"));
    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains("password"));
    assert!(!encoded.contains("token"));
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn channel_history_reuses_registry_fetch_and_commits_native_message_ids() {
    let (_temp, manager) = manager().await;
    let mut registry = ChannelRegistry::new();
    registry
        .register(Arc::new(tokio::sync::RwLock::new(FakeChannelPlugin::new())))
        .await;
    registry
        .start_account("slack", "slack-work", serde_json::json!({}))
        .await
        .unwrap();
    manager
        .configure_channel_registry(Arc::new(registry))
        .unwrap();

    let account = manager
        .add_account(AccountCreateRequest {
            kind: ConnectorKind::ChannelHistory,
            name: "Slack planning".to_owned(),
            server_url: String::new(),
            username: String::new(),
            password: Secret::new(String::new()),
            channel_type: Some(ChannelType::Slack),
            channel_account_id: Some("slack-work".to_owned()),
            himalaya_account_name: None,
            himalaya_backend: None,
            timeout_seconds: 30,
            allow_insecure_http: false,
            allow_private_network: false,
            enabled: true,
        })
        .await
        .unwrap();
    let dataset = manager
        .add_dataset(DatasetCreateRequest {
            account_id: account.id,
            name: "Planning thread".to_owned(),
            instruction: "Keep recent Slack planning messages".to_owned(),
            config: ConnectorDatasetConfigView::ChannelHistory(ChannelHistoryDatasetConfigView {
                schema_version: 1,
                channel_id: "C123".to_owned(),
                thread_id: "1712345678.000100".to_owned(),
                limit: 100,
            }),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
        })
        .await
        .unwrap();

    let manager = Arc::new(manager);
    let run = manager.sync_dataset(&dataset.id).await.unwrap();
    assert_eq!(run.active, 1);
    let items = manager
        .query_items(&dataset.id, ItemQueryRequest::default())
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].remote_id, "message-1");
    assert_eq!(items[0].body_json["text"], "Quarterly planning");
    assert_eq!(items[0].body_json["channelType"], "slack");
}
