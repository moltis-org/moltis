use {
    super::*,
    async_trait::async_trait,
    moltis_channels::{ChannelReplyTarget, ChannelType},
    moltis_common::types::ReplyPayload,
    std::sync::{Arc, Mutex},
};

struct PendingTargetRuntime {
    targets: Mutex<Vec<ChannelReplyTarget>>,
    tts: moltis_service_traits::NoopTtsService,
    project: moltis_service_traits::NoopProjectService,
    mcp: moltis_service_traits::NoopMcpService,
}

#[async_trait]
impl ChatRuntime for PendingTargetRuntime {
    async fn broadcast(&self, _topic: &str, _payload: Value) {}

    async fn push_channel_reply(&self, _session_key: &str, target: ChannelReplyTarget) {
        self.targets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(target);
    }

    async fn drain_channel_replies(&self, _session_key: &str) -> Vec<ChannelReplyTarget> {
        std::mem::take(
            &mut *self
                .targets
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }

    async fn peek_channel_replies(&self, _session_key: &str) -> Vec<ChannelReplyTarget> {
        self.targets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    async fn push_channel_status_log(&self, _session_key: &str, _message: String) {}

    async fn drain_channel_status_log(&self, _session_key: &str) -> Vec<String> {
        Vec::new()
    }

    async fn set_run_error(&self, _run_id: &str, _error: String) {}

    async fn active_session_key(&self, _conn_id: &str) -> Option<String> {
        None
    }

    async fn active_project_id(&self, _conn_id: &str) -> Option<String> {
        None
    }

    fn hostname(&self) -> &str {
        "test"
    }

    fn sandbox_router(&self) -> Option<&Arc<moltis_tools::sandbox::SandboxRouter>> {
        None
    }

    fn memory_manager(&self) -> Option<&moltis_memory::runtime::DynMemoryRuntime> {
        None
    }

    async fn cached_location(&self) -> Option<moltis_config::GeoLocation> {
        None
    }

    async fn tts_overrides(
        &self,
        _session_key: &str,
        _channel_key: &str,
    ) -> (
        Option<crate::runtime::TtsOverride>,
        Option<crate::runtime::TtsOverride>,
    ) {
        (None, None)
    }

    fn channel_outbound(&self) -> Option<Arc<dyn moltis_channels::ChannelOutbound>> {
        None
    }

    fn channel_stream_outbound(&self) -> Option<Arc<dyn moltis_channels::ChannelStreamOutbound>> {
        None
    }

    fn tts_service(&self) -> &dyn moltis_service_traits::TtsService {
        &self.tts
    }

    fn project_service(&self) -> &dyn moltis_service_traits::ProjectService {
        &self.project
    }

    fn mcp_service(&self) -> &dyn moltis_service_traits::McpService {
        &self.mcp
    }

    async fn chat_service(&self) -> Arc<dyn moltis_service_traits::ChatService> {
        Arc::new(moltis_service_traits::NoopChatService)
    }

    async fn last_run_error(&self, _run_id: &str) -> Option<String> {
        None
    }

    async fn send_push_notification(
        &self,
        _title: &str,
        _body: &str,
        _url: Option<&str>,
        _session_key: Option<&str>,
    ) -> crate::error::Result<usize> {
        Ok(0)
    }

    async fn ensure_local_model_cached(&self, _model_id: &str) -> crate::error::Result<bool> {
        Ok(false)
    }

    async fn connected_nodes(&self) -> Vec<crate::runtime::ConnectedNodeSummary> {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
struct SentMedia {
    account_id: String,
    to: String,
    payload: ReplyPayload,
    reply_to: Option<String>,
}

#[derive(Debug, Clone)]
struct SentText {
    account_id: String,
    to: String,
    text: String,
    reply_to: Option<String>,
}

#[derive(Default)]
struct RecordingOutbound {
    sent_media: Mutex<Vec<SentMedia>>,
    sent_text: Mutex<Vec<SentText>>,
    fail_media: bool,
    fail_text: bool,
}

#[async_trait]
impl moltis_channels::ChannelOutbound for RecordingOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        if self.fail_text {
            return Err(moltis_channels::Error::unavailable("test failure"));
        }
        self.sent_text
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(SentText {
                account_id: account_id.to_string(),
                to: to.to_string(),
                text: text.to_string(),
                reply_to: reply_to.map(ToString::to_string),
            });
        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        if self.fail_media {
            return Err(moltis_channels::Error::unavailable("test failure"));
        }
        self.sent_media
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(SentMedia {
                account_id: account_id.to_string(),
                to: to.to_string(),
                payload: payload.clone(),
                reply_to: reply_to.map(ToString::to_string),
            });
        Ok(())
    }
}

fn telegram_target() -> ChannelReplyTarget {
    ChannelReplyTarget {
        ack_message_id: None,
        channel_type: ChannelType::Telegram,
        account_id: "telegram-main".into(),
        chat_id: "-100123".into(),
        message_id: Some("42".into()),
        thread_id: Some("7".into()),
    }
}

#[tokio::test]
async fn silent_terminal_response_with_pending_target_is_delivery_failure() {
    for streamed in [false, true] {
        for text in ["", "   "] {
            let target = telegram_target();
            let streamed_target_keys = if streamed {
                HashSet::from([ChannelReplyTargetKey::from(&target)])
            } else {
                HashSet::new()
            };
            let state: Arc<dyn ChatRuntime> = Arc::new(PendingTargetRuntime {
                targets: Mutex::new(vec![target]),
                tts: moltis_service_traits::NoopTtsService,
                project: moltis_service_traits::NoopProjectService,
                mcp: moltis_service_traits::NoopMcpService,
            });

            assert!(
                !deliver_channel_replies(
                    &state,
                    "run-1",
                    "telegram:bot:123",
                    text,
                    ReplyMedium::Text,
                    &streamed_target_keys,
                )
                .await,
                "silent channel response {text:?} (streamed={streamed}) must not count as delivered"
            );
        }
    }
}

#[tokio::test]
async fn silent_web_only_response_remains_successful_without_a_channel_target() {
    let state: Arc<dyn ChatRuntime> = Arc::new(PendingTargetRuntime {
        targets: Mutex::new(Vec::new()),
        tts: moltis_service_traits::NoopTtsService,
        project: moltis_service_traits::NoopProjectService,
        mcp: moltis_service_traits::NoopMcpService,
    });

    assert!(
        deliver_channel_replies(
            &state,
            "run-1",
            "chat:main",
            "",
            ReplyMedium::Text,
            &HashSet::new(),
        )
        .await
    );
}

#[tokio::test]
async fn unavailable_tts_uses_successful_text_as_final_delivery() {
    let outbound = RecordingOutbound::default();
    let target = telegram_target();

    assert!(
        deliver_text_fallback(
            &outbound,
            &target,
            "-100123:7",
            "fallback transcript",
            "",
            Some("42"),
            false,
        )
        .await
    );

    let sent = outbound.sent_text.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].account_id, "telegram-main");
    assert_eq!(sent[0].to, "-100123:7");
    assert_eq!(sent[0].text, "fallback transcript");
    assert_eq!(sent[0].reply_to.as_deref(), Some("42"));
}

#[tokio::test]
async fn failed_text_fallback_is_a_final_delivery_failure() {
    let outbound = RecordingOutbound {
        fail_text: true,
        ..Default::default()
    };
    let target = telegram_target();

    assert!(!deliver_text_fallback(&outbound, &target, "-100123:7", "text", "", None, false).await);
}

#[tokio::test]
async fn generated_image_payload_dispatches_to_telegram_as_media() {
    let outbound = Arc::new(RecordingOutbound::default());
    let targets = vec![telegram_target()];

    assert!(
        dispatch_screenshot_to_targets(
            outbound.clone(),
            targets,
            "data:image/png;base64,cG5n",
            Some("Generated image: fox"),
        )
        .await
    );

    {
        let sent = outbound
            .sent_media
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].account_id, "telegram-main");
        assert_eq!(sent[0].to, "-100123:7");
        assert_eq!(sent[0].reply_to.as_deref(), Some("42"));
        assert_eq!(sent[0].payload.text, "Generated image: fox");
        let Some(media) = sent[0].payload.media.as_ref() else {
            panic!("media payload");
        };
        assert_eq!(media.mime_type, "image/png");
        assert_eq!(media.url, "data:image/png;base64,cG5n");
    }

    let failing = Arc::new(RecordingOutbound {
        fail_media: true,
        ..Default::default()
    });
    assert!(
        !dispatch_screenshot_to_targets(
            failing,
            vec![ChannelReplyTarget {
                message_id: None,
                thread_id: None,
                ..telegram_target()
            }],
            "data:image/png;base64,cG5n",
            None,
        )
        .await
    );
}

#[tokio::test]
async fn generated_image_payload_dispatches_to_matrix_as_media() {
    let outbound = Arc::new(RecordingOutbound::default());
    let targets = vec![ChannelReplyTarget {
        ack_message_id: None,
        channel_type: ChannelType::Matrix,
        account_id: "matrix-main".into(),
        chat_id: "!room:example.org".into(),
        message_id: Some("$event".into()),
        thread_id: None,
    }];

    dispatch_screenshot_to_targets(
        outbound.clone(),
        targets,
        "data:image/webp;base64,d2VicA==",
        Some("Generated image: logo"),
    )
    .await;

    let sent = outbound
        .sent_media
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].account_id, "matrix-main");
    assert_eq!(sent[0].to, "!room:example.org");
    assert_eq!(sent[0].reply_to.as_deref(), Some("$event"));
    assert_eq!(sent[0].payload.text, "Generated image: logo");
    let Some(media) = sent[0].payload.media.as_ref() else {
        panic!("media payload");
    };
    assert_eq!(media.mime_type, "image/webp");
    assert_eq!(media.url, "data:image/webp;base64,d2VicA==");
}
