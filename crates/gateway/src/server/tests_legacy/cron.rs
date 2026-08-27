use std::sync::Arc;

use {
    super::common::{DeliveredMessage, RecordingChannelOutbound, cron_delivery_request},
    async_trait::async_trait,
    moltis_channels::{ChannelReplyTarget, ChannelType},
    moltis_common::types::ReplyPayload,
    moltis_sessions::{metadata::SqliteSessionMetadata, store::SessionStore},
    sqlx::sqlite::SqlitePoolOptions,
};

struct CronHistoryFixture {
    _directory: tempfile::TempDir,
    registry: Arc<moltis_channels::ChannelRegistry>,
    metadata: Arc<SqliteSessionMetadata>,
    store: Arc<SessionStore>,
}

impl CronHistoryFixture {
    async fn new(channel_type: ChannelType, account_id: &str) -> Self {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE projects (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        SqliteSessionMetadata::init(&pool).await.unwrap();
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(directory.path().to_path_buf()));
        let registry = Arc::new(moltis_channels::ChannelRegistry::new());
        registry.index_account(account_id, channel_type.as_str());
        Self {
            _directory: directory,
            registry,
            metadata,
            store,
        }
    }

    async fn bind(&self, target: &ChannelReplyTarget, session_key: &str, explicit: bool) {
        self.metadata.upsert(session_key, None).await.unwrap();
        self.metadata
            .set_channel_binding(session_key, Some(serde_json::to_string(target).unwrap()))
            .await;
        if explicit {
            self.metadata
                .set_active_session(
                    target.channel_type.as_str(),
                    &target.account_id,
                    &target.chat_id,
                    target.thread_id.as_deref(),
                    session_key,
                )
                .await;
        }
    }

    fn history(&self, delivery_id: &str) -> crate::server::helpers::CronDeliveryHistory {
        crate::server::helpers::CronDeliveryHistory {
            registry: Arc::clone(&self.registry),
            metadata: Arc::clone(&self.metadata),
            store: Arc::clone(&self.store),
            delivery_id: delivery_id.to_string(),
        }
    }
}

fn reply_target(
    channel_type: ChannelType,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
) -> ChannelReplyTarget {
    ChannelReplyTarget {
        channel_type,
        account_id: account_id.to_string(),
        chat_id: chat_id.to_string(),
        message_id: None,
        thread_id: thread_id.map(str::to_string),
        ack_message_id: None,
    }
}

fn whatsapp_target(chat_id: &str) -> ChannelReplyTarget {
    reply_target(ChannelType::Whatsapp, "bot-main", chat_id, None)
}

struct FailingChannelOutbound;

#[async_trait]
impl moltis_channels::ChannelOutbound for FailingChannelOutbound {
    async fn send_text(
        &self,
        _account_id: &str,
        _to: &str,
        _text: &str,
        _reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        Err(moltis_channels::Error::unavailable("test delivery failure"))
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

struct RemappingChannelOutbound {
    metadata: Arc<SqliteSessionMetadata>,
    target: ChannelReplyTarget,
    session_key: String,
}

#[async_trait]
impl moltis_channels::ChannelOutbound for RemappingChannelOutbound {
    async fn send_text(
        &self,
        _account_id: &str,
        _to: &str,
        _text: &str,
        _reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        self.metadata
            .set_active_session(
                self.target.channel_type.as_str(),
                &self.target.account_id,
                &self.target.chat_id,
                self.target.thread_id.as_deref(),
                &self.session_key,
            )
            .await;
        Ok(())
    }

    async fn send_media(
        &self,
        _account_id: &str,
        _to: &str,
        _reply: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn maybe_deliver_cron_output_sends_to_configured_channel() {
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "Daily digest ready",
        None,
    )
    .await
    .unwrap();

    let delivered = outbound.delivered.lock().await.clone();
    assert_eq!(delivered, vec![DeliveredMessage {
        account_id: "bot-main".to_string(),
        to: "123456".to_string(),
        text: "Daily digest ready".to_string(),
        reply_to: None,
    }]);
}

#[tokio::test]
async fn maybe_deliver_cron_output_skips_blank_messages() {
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "   ",
        None,
    )
    .await
    .unwrap();

    assert!(outbound.delivered.lock().await.is_empty());
}

#[tokio::test]
async fn maybe_deliver_cron_output_skips_when_deliver_is_false() {
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let mut req = cron_delivery_request();
    req.deliver = false;

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "should not be sent",
        None,
    )
    .await
    .unwrap();

    assert!(outbound.delivered.lock().await.is_empty());
}

#[tokio::test]
async fn maybe_deliver_cron_output_fails_when_no_outbound_is_configured() {
    let req = cron_delivery_request();

    let error =
        crate::server::helpers::maybe_deliver_cron_output(None, &req, "Daily digest ready", None)
            .await
            .unwrap_err();
    assert!(error.to_string().contains("outbound is unavailable"));
}

#[tokio::test]
async fn maybe_deliver_cron_output_propagates_channel_failure() {
    let req = cron_delivery_request();
    let outbound: Arc<dyn moltis_channels::ChannelOutbound> = Arc::new(FailingChannelOutbound);

    let error = crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound),
        &req,
        "Daily digest ready",
        None,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("test delivery failure"));
}

#[tokio::test]
async fn bare_whatsapp_number_delivery_is_recorded_once_in_bound_conversation() {
    let fixture = CronHistoryFixture::new(ChannelType::Whatsapp, "bot-main").await;
    let target = whatsapp_target("123456@s.whatsapp.net");
    fixture.bind(&target, "session:whatsapp", true).await;
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();
    let history = fixture.history("run-1");

    for _ in 0..2 {
        crate::server::helpers::maybe_deliver_cron_output(
            Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
            &req,
            "Services total R$ 1.042,30",
            Some(&history),
        )
        .await
        .unwrap();
    }

    assert_eq!(outbound.delivered.lock().await.len(), 1);
    let messages = fixture.store.read("session:whatsapp").await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "assistant");
    assert_eq!(messages[0]["content"], "Services total R$ 1.042,30");
    assert_eq!(messages[0]["run_id"], "cron-delivery:run-1");
    assert_eq!(messages[0]["channel"]["private_context"], false);
    let llm_history = moltis_agents::model::values_to_chat_messages(&messages);
    assert!(matches!(
        &llm_history[0],
        moltis_agents::model::ChatMessage::Assistant { content: Some(content), .. }
            if content == "Services total R$ 1.042,30"
    ));
    assert_eq!(
        fixture
            .metadata
            .get("session:whatsapp")
            .await
            .unwrap()
            .message_count,
        1
    );
}

#[tokio::test]
async fn failed_delivery_is_not_added_to_conversation_history() {
    let fixture = CronHistoryFixture::new(ChannelType::Whatsapp, "bot-main").await;
    let target = whatsapp_target("123456@s.whatsapp.net");
    fixture.bind(&target, "session:whatsapp", true).await;
    let req = cron_delivery_request();
    let history = fixture.history("failed-run");

    let error = crate::server::helpers::maybe_deliver_cron_output(
        Some(Arc::new(FailingChannelOutbound)),
        &req,
        "This was not delivered",
        Some(&history),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("test delivery failure"));
    assert!(
        fixture
            .store
            .read("session:whatsapp")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn delivered_output_does_not_fail_when_history_cannot_be_written() {
    let fixture = CronHistoryFixture::new(ChannelType::Whatsapp, "bot-main").await;
    let target = whatsapp_target("123456@s.whatsapp.net");
    fixture.bind(&target, "session:whatsapp", true).await;
    let blocked_path = fixture._directory.path().join("not-a-directory");
    std::fs::write(&blocked_path, b"file blocks session directory").unwrap();
    let bad_store = Arc::new(SessionStore::new(blocked_path));
    let history = crate::server::helpers::CronDeliveryHistory {
        registry: Arc::clone(&fixture.registry),
        metadata: Arc::clone(&fixture.metadata),
        store: bad_store,
        delivery_id: "history-failure-run".to_string(),
    };
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "The recipient received this",
        Some(&history),
    )
    .await
    .unwrap();

    assert_eq!(outbound.delivered.lock().await.len(), 1);
    assert_eq!(
        fixture
            .metadata
            .get("session:whatsapp")
            .await
            .unwrap()
            .message_count,
        0
    );
}

#[tokio::test]
async fn delivery_without_an_existing_bound_session_stays_out_of_history() {
    let fixture = CronHistoryFixture::new(ChannelType::Whatsapp, "bot-main").await;
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();
    let history = fixture.history("unbound-run");

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "Delivered without a conversation",
        Some(&history),
    )
    .await
    .unwrap();

    assert_eq!(outbound.delivered.lock().await.len(), 1);
    assert!(
        fixture
            .store
            .read(&whatsapp_target("123456@s.whatsapp.net").default_session_key())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn delivery_is_isolated_to_the_exact_recipient() {
    let fixture = CronHistoryFixture::new(ChannelType::Whatsapp, "bot-main").await;
    let intended = whatsapp_target("123456@s.whatsapp.net");
    let other = whatsapp_target("999999@s.whatsapp.net");
    fixture.bind(&intended, "session:intended", true).await;
    fixture.bind(&other, "session:other", true).await;
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();
    let history = fixture.history("isolated-run");

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "Only the intended recipient saw this",
        Some(&history),
    )
    .await
    .unwrap();

    assert_eq!(
        fixture.store.read("session:intended").await.unwrap().len(),
        1
    );
    assert!(
        fixture
            .store
            .read("session:other")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn stale_mapping_to_another_recipient_fails_closed() {
    let fixture = CronHistoryFixture::new(ChannelType::Whatsapp, "bot-main").await;
    let intended = whatsapp_target("123456@s.whatsapp.net");
    let other = whatsapp_target("999999@s.whatsapp.net");
    fixture.bind(&other, "session:other", false).await;
    fixture
        .metadata
        .set_active_session(
            intended.channel_type.as_str(),
            &intended.account_id,
            &intended.chat_id,
            None,
            "session:other",
        )
        .await;
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();
    let history = fixture.history("stale-mapping-run");

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "Delivered, but never written into another recipient's history",
        Some(&history),
    )
    .await
    .unwrap();

    assert_eq!(outbound.delivered.lock().await.len(), 1);
    assert!(
        fixture
            .store
            .read("session:other")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn binding_is_revalidated_after_the_external_send() {
    let fixture = CronHistoryFixture::new(ChannelType::Whatsapp, "bot-main").await;
    let intended = whatsapp_target("123456@s.whatsapp.net");
    let other = whatsapp_target("999999@s.whatsapp.net");
    fixture.bind(&intended, "session:intended", true).await;
    fixture.bind(&other, "session:other", false).await;
    let outbound = Arc::new(RemappingChannelOutbound {
        metadata: Arc::clone(&fixture.metadata),
        target: intended,
        session_key: "session:other".to_string(),
    });
    let req = cron_delivery_request();
    let history = fixture.history("remapped-run");

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "Delivered while the binding changed",
        Some(&history),
    )
    .await
    .unwrap();

    assert!(
        fixture
            .store
            .read("session:intended")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .store
            .read("session:other")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn same_target_session_remap_during_send_is_not_recorded() {
    let fixture = CronHistoryFixture::new(ChannelType::Whatsapp, "bot-main").await;
    let intended = whatsapp_target("123456@s.whatsapp.net");
    fixture.bind(&intended, "session:before-send", true).await;
    fixture.bind(&intended, "session:after-send", false).await;
    let outbound = Arc::new(RemappingChannelOutbound {
        metadata: Arc::clone(&fixture.metadata),
        target: intended,
        session_key: "session:after-send".to_string(),
    });
    let req = cron_delivery_request();
    let history = fixture.history("same-target-remap-run");

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "Delivered while the active session changed",
        Some(&history),
    )
    .await
    .unwrap();

    assert!(
        fixture
            .store
            .read("session:before-send")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .store
            .read("session:after-send")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn telegram_delivery_is_isolated_to_the_exact_topic() {
    let fixture = CronHistoryFixture::new(ChannelType::Telegram, "bot-main").await;
    let topic = reply_target(ChannelType::Telegram, "bot-main", "-100123", Some("42"));
    let base_chat = reply_target(ChannelType::Telegram, "bot-main", "-100123", None);
    fixture.bind(&topic, "session:topic", true).await;
    fixture.bind(&base_chat, "session:base", true).await;
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let mut req = cron_delivery_request();
    req.to = Some("-100123:42".to_string());
    let history = fixture.history("topic-run");

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "Topic-only report",
        Some(&history),
    )
    .await
    .unwrap();

    assert_eq!(fixture.store.read("session:topic").await.unwrap().len(), 1);
    assert!(fixture.store.read("session:base").await.unwrap().is_empty());
}

#[tokio::test]
async fn default_bound_conversation_is_used_without_an_active_override() {
    let fixture = CronHistoryFixture::new(ChannelType::Whatsapp, "bot-main").await;
    let target = whatsapp_target("123456@s.whatsapp.net");
    let default_key = target.default_session_key();
    fixture.bind(&target, &default_key, false).await;
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();
    let history = fixture.history("default-run");

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "Default conversation report",
        Some(&history),
    )
    .await
    .unwrap();

    assert_eq!(fixture.store.read(&default_key).await.unwrap().len(), 1);
}
