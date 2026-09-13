#![allow(clippy::unwrap_used)]

use {
    super::{tests::registry, *},
    crate::{runtime::TtsOverride, service::ActiveAssistantDraft},
    moltis_agents::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, Usage},
    serde_json::Value,
    std::{collections::HashMap, pin::Pin, sync::Mutex},
};

#[derive(Default)]
struct Runtime {
    broadcasts: Mutex<Vec<Value>>,
    error: Mutex<Option<String>>,
    targets: Mutex<Vec<moltis_channels::ChannelReplyTarget>>,
    outbound: Arc<Outbound>,
    tts: Tts,
}

#[derive(Default)]
struct Outbound {
    texts: Mutex<Vec<String>>,
    streams: Mutex<Vec<moltis_channels::StreamEvent>>,
}

#[async_trait::async_trait]
impl moltis_channels::ChannelOutbound for Outbound {
    async fn send_text(
        &self,
        _: &str,
        _: &str,
        text: &str,
        _: Option<&str>,
    ) -> moltis_channels::Result<()> {
        self.texts.lock().unwrap().push(text.into());
        Ok(())
    }

    async fn send_media(
        &self,
        _: &str,
        _: &str,
        _: &moltis_common::types::ReplyPayload,
        _: Option<&str>,
    ) -> moltis_channels::Result<()> {
        panic!("unexpected media delivery")
    }
}

#[async_trait::async_trait]
impl moltis_channels::ChannelStreamOutbound for Outbound {
    async fn send_stream(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        mut stream: moltis_channels::StreamReceiver,
    ) -> moltis_channels::Result<()> {
        while let Some(event) = stream.recv().await {
            self.streams.lock().unwrap().push(event);
        }
        Ok(())
    }
}

#[derive(Default)]
struct Tts(Mutex<Vec<Value>>);

#[async_trait::async_trait]
impl moltis_service_traits::TtsService for Tts {
    async fn status(&self) -> moltis_service_traits::ServiceResult {
        Ok(serde_json::json!({"enabled": true}))
    }

    async fn convert(&self, params: Value) -> moltis_service_traits::ServiceResult {
        self.0.lock().unwrap().push(params);
        // Exercise the normal text fallback without writing media files.
        Err("test synthesis failure".into())
    }

    async fn providers(&self) -> moltis_service_traits::ServiceResult {
        panic!("unused")
    }

    async fn enable(&self, _: Value) -> moltis_service_traits::ServiceResult {
        panic!("unused")
    }

    async fn disable(&self) -> moltis_service_traits::ServiceResult {
        panic!("unused")
    }

    async fn set_provider(&self, _: Value) -> moltis_service_traits::ServiceResult {
        panic!("unused")
    }
}

#[derive(Default)]
struct Tool(Arc<Mutex<usize>>);

#[async_trait::async_trait]
impl moltis_agents::tool_registry::AgentTool for Tool {
    fn name(&self) -> &str {
        "lifecycle_test"
    }

    fn description(&self) -> &str {
        "Return a test result"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _: Value) -> anyhow::Result<Value> {
        *self.0.lock().unwrap() += 1;
        Ok(serde_json::json!({"ok": true}))
    }
}

#[async_trait::async_trait]
impl ChatRuntime for Runtime {
    async fn broadcast(&self, _: &str, payload: Value) {
        self.broadcasts.lock().unwrap().push(payload);
    }

    async fn push_channel_reply(&self, _: &str, _: moltis_channels::ChannelReplyTarget) {}

    async fn drain_channel_replies(&self, _: &str) -> Vec<moltis_channels::ChannelReplyTarget> {
        std::mem::take(&mut *self.targets.lock().unwrap())
    }

    async fn peek_channel_replies(&self, _: &str) -> Vec<moltis_channels::ChannelReplyTarget> {
        self.targets.lock().unwrap().clone()
    }

    async fn push_channel_status_log(&self, _: &str, _: String) {}

    async fn drain_channel_status_log(&self, _: &str) -> Vec<String> {
        vec![]
    }

    async fn set_run_error(&self, _: &str, error: String) {
        *self.error.lock().unwrap() = Some(error);
    }

    async fn active_session_key(&self, _: &str) -> Option<String> {
        None
    }

    async fn active_project_id(&self, _: &str) -> Option<String> {
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

    async fn tts_overrides(&self, _: &str, _: &str) -> (Option<TtsOverride>, Option<TtsOverride>) {
        (None, None)
    }

    fn channel_outbound(&self) -> Option<Arc<dyn moltis_channels::ChannelOutbound>> {
        Some(self.outbound.clone())
    }

    fn channel_stream_outbound(&self) -> Option<Arc<dyn moltis_channels::ChannelStreamOutbound>> {
        Some(self.outbound.clone())
    }

    fn tts_service(&self) -> &dyn moltis_service_traits::TtsService {
        &self.tts
    }

    fn project_service(&self) -> &dyn moltis_service_traits::ProjectService {
        &moltis_service_traits::NoopProjectService
    }

    fn mcp_service(&self) -> &dyn moltis_service_traits::McpService {
        &moltis_service_traits::NoopMcpService
    }

    async fn chat_service(&self) -> Arc<dyn moltis_service_traits::ChatService> {
        panic!("unused")
    }

    async fn last_run_error(&self, _: &str) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    async fn send_push_notification(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> crate::error::Result<usize> {
        Ok(0)
    }

    async fn ensure_local_model_cached(&self, _: &str) -> crate::error::Result<bool> {
        Ok(false)
    }

    async fn connected_nodes(&self) -> Vec<crate::runtime::ConnectedNodeSummary> {
        vec![]
    }
}

struct Provider {
    events: Mutex<Vec<StreamEvent>>,
}

#[async_trait::async_trait]
impl LlmProvider for Provider {
    fn supports_tools(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "test"
    }

    fn id(&self) -> &str {
        "test"
    }

    async fn complete(&self, _: &[ChatMessage], _: &[Value]) -> anyhow::Result<CompletionResponse> {
        anyhow::bail!("stream only")
    }

    fn stream(
        &self,
        _: Vec<ChatMessage>,
    ) -> Pin<Box<dyn tokio_stream::Stream<Item = StreamEvent> + Send + '_>> {
        let mut events = self.events.lock().unwrap();
        let end = events
            .iter()
            .position(|event| matches!(event, StreamEvent::Done(_)))
            .map_or(events.len(), |index| index + 1);
        Box::pin(tokio_stream::iter(events.drain(..end).collect::<Vec<_>>()))
    }
}

async fn execute(
    hooks: Option<HookRegistry>,
    events: Vec<StreamEvent>,
) -> (Option<AssistantTurnOutput>, Arc<Runtime>, bool, Value) {
    execute_mode(hooks, events, None).await
}

async fn execute_mode(
    hooks: Option<HookRegistry>,
    events: Vec<StreamEvent>,
    tools: Option<ReplyMedium>,
) -> (Option<AssistantTurnOutput>, Arc<Runtime>, bool, Value) {
    let runtime = Arc::new(Runtime::default());
    let state: Arc<dyn ChatRuntime> = runtime.clone();
    let terminal = Arc::new(RwLock::new(HashSet::new()));
    let drafts = Arc::new(RwLock::new(HashMap::from([(
        "session".into(),
        ActiveAssistantDraft::new("run", "test", "test", None),
    )])));
    let persona = PromptPersona {
        config: Default::default(),
        identity: Default::default(),
        user: Default::default(),
        soul_text: None,
        boot_text: None,
        agents_text: None,
        tools_text: None,
        guidelines_text: None,
        memory_text: None,
        memory_status: PromptMemoryStatus {
            style: Default::default(),
            mode: Default::default(),
            write_mode: Default::default(),
            snapshot_active: false,
            present: false,
            chars: 0,
            path: None,
            file_source: None,
        },
    };
    let provider = Arc::new(Provider {
        events: Mutex::new(events),
    });
    let output = if let Some(medium) = tools {
        runtime
            .targets
            .lock()
            .unwrap()
            .push(moltis_channels::ChannelReplyTarget {
                ack_message_id: None,
                channel_type: moltis_channels::ChannelType::Telegram,
                account_id: "test".into(),
                chat_id: "123".into(),
                message_id: None,
                thread_id: None,
            });
        let tool_calls = Arc::new(Mutex::new(0));
        let mut registry = moltis_agents::tool_registry::ToolRegistry::new();
        registry.register(Box::new(Tool(tool_calls.clone())));
        let output = crate::run_with_tools::run_with_tools(
            persona,
            &state,
            &Arc::new(RwLock::new(Default::default())),
            "run",
            provider,
            "test",
            &Arc::new(RwLock::new(registry)),
            &moltis_agents::UserContent::Text("hello".into()),
            "test",
            &[],
            "session",
            "main",
            medium,
            None,
            None,
            0,
            &[],
            hooks.map(Arc::new),
            None,
            None,
            None,
            false,
            None,
            None,
            None,
            Some(drafts.clone()),
            &Arc::new(RwLock::new(HashMap::new())),
            &terminal,
            None,
            None,
            true,
        )
        .await;
        assert_eq!(*tool_calls.lock().unwrap(), 1);
        output
    } else {
        crate::streaming::run_streaming(
            persona,
            &state,
            &Arc::new(RwLock::new(Default::default())),
            "run",
            provider,
            "test",
            &moltis_agents::UserContent::Text("hello".into()),
            "test",
            &[],
            "session",
            "main",
            ReplyMedium::Text,
            None,
            0,
            &[],
            None,
            None,
            None,
            None,
            Some(drafts.clone()),
            &terminal,
            false,
            hooks.map(Arc::new),
        )
        .await
    };
    let committed = terminal.read().await.contains("run");
    let draft = drafts
        .read()
        .await
        .get("session")
        .unwrap()
        .to_persisted_message()
        .to_value();
    (output, runtime, committed, draft)
}

fn success_events() -> Vec<StreamEvent> {
    vec![
        StreamEvent::Delta("original".into()),
        StreamEvent::ReasoningDelta("private reasoning".into()),
        StreamEvent::Done(Usage {
            output_tokens: 1,
            ..Default::default()
        }),
    ]
}

fn tool_events() -> Vec<StreamEvent> {
    let mut events = vec![
        StreamEvent::Delta("original pre-tool draft".into()),
        StreamEvent::ReasoningDelta("private reasoning".into()),
        StreamEvent::ToolCallStart {
            id: "call-1".into(),
            name: "lifecycle_test".into(),
            index: 0,
            metadata: None,
        },
        StreamEvent::ToolCallArgumentsDelta {
            index: 0,
            delta: "{}".into(),
        },
        StreamEvent::ToolCallComplete { index: 0 },
        StreamEvent::Done(Usage {
            input_tokens: 3,
            output_tokens: 2,
            ..Default::default()
        }),
    ];
    events.extend(success_events());
    events
}

#[tokio::test]
async fn message_lifecycle_tools_rewrite_reaches_transport_without_preview_leaks() {
    for medium in [ReplyMedium::Text, ReplyMedium::Voice] {
        let (hooks, seen) = registry(
            HookAction::ModifyPayload(serde_json::json!({"content": "approved"})),
            false,
        );
        let (output, runtime, committed, draft) =
            execute_mode(Some(hooks), tool_events(), Some(medium)).await;
        let output = output.unwrap();
        assert_eq!(output.text, "approved");
        let final_payload = output.final_broadcast.as_ref().unwrap();
        assert_eq!(final_payload["text"], "approved");
        assert_eq!(final_payload["iterations"], 2);
        assert_eq!(final_payload["toolCallsMade"], 1);
        let persisted = crate::service::build_persisted_assistant_message(
            output,
            None,
            None,
            None,
            Some("run".into()),
        )
        .to_value();
        assert_eq!(persisted["content"], "approved");
        assert!(committed);
        assert_eq!(*runtime.outbound.texts.lock().unwrap(), ["approved"]);
        assert!(runtime.outbound.streams.lock().unwrap().is_empty());
        let tts = runtime.tts.0.lock().unwrap();
        if medium == ReplyMedium::Voice {
            assert!(!tts.is_empty());
            assert!(tts.iter().all(|request| request["text"] == "approved"));
        } else {
            assert!(tts.is_empty());
        }
        for visible in [
            serde_json::to_value(&*runtime.broadcasts.lock().unwrap()).unwrap(),
            draft,
        ] {
            let visible = visible.to_string();
            assert!(!visible.contains("original"), "{visible}");
            assert!(!visible.contains("private reasoning"), "{visible}");
        }
        let seen = seen.lock().unwrap();
        assert!(matches!(&seen[..], [
            HookPayload::AgentEnd { text, iterations: 2, tool_calls: 1, .. },
            HookPayload::MessageSending { content, .. },
        ] if text == "original" && content == text));
    }
}

#[tokio::test]
async fn message_lifecycle_tools_block_prevents_reply_and_tts_delivery() {
    for medium in [ReplyMedium::Text, ReplyMedium::Voice] {
        let (hooks, seen) = registry(HookAction::Block("policy".into()), false);
        let (output, runtime, committed, draft) =
            execute_mode(Some(hooks), tool_events(), Some(medium)).await;
        assert!(output.is_none());
        assert!(committed);
        assert!(
            runtime
                .error
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("policy")
        );
        assert!(runtime.tts.0.lock().unwrap().is_empty());
        assert!(runtime.outbound.streams.lock().unwrap().is_empty());
        // Approved channel senders still receive the policy error, never the reply.
        let texts = runtime.outbound.texts.lock().unwrap();
        assert_eq!(texts.len(), 1);
        assert!(texts[0].contains("policy"));
        let broadcasts = runtime.broadcasts.lock().unwrap();
        assert_eq!(
            broadcasts
                .iter()
                .filter(|event| event["state"] == "error")
                .count(),
            1
        );
        assert!(
            !broadcasts
                .iter()
                .any(|event| event["state"] == "final" || event["state"] == "delta")
        );
        for visible in [
            serde_json::to_value(&*broadcasts).unwrap(),
            serde_json::to_value(&*texts).unwrap(),
            draft,
        ] {
            let visible = visible.to_string();
            assert!(!visible.contains("original"), "{visible}");
            assert!(!visible.contains("private reasoning"), "{visible}");
        }
        assert!(matches!(&seen.lock().unwrap()[..], [
            HookPayload::AgentEnd { text, iterations: 2, tool_calls: 1, .. },
            HookPayload::MessageSending { content, .. },
        ] if text == "original" && content == text));
    }
}

#[tokio::test]
async fn message_lifecycle_stream_rewrite_suppresses_previews_and_orders_agent_end() {
    let (hooks, seen) = registry(
        HookAction::ModifyPayload(serde_json::json!({"content": "approved"})),
        false,
    );
    let (output, runtime, committed, draft) = execute(Some(hooks), success_events()).await;
    let output = output.unwrap();
    assert_eq!(output.text, "approved");
    assert_eq!(output.final_broadcast.as_ref().unwrap()["text"], "approved");
    let persisted = crate::service::build_persisted_assistant_message(
        output,
        None,
        None,
        None,
        Some("run".into()),
    )
    .to_value();
    assert_eq!(persisted["content"], "approved");
    assert!(committed);
    assert!(runtime.broadcasts.lock().unwrap().is_empty());
    assert!(!draft.to_string().contains("original"));
    assert!(!draft.to_string().contains("private reasoning"));
    let seen = seen.lock().unwrap();
    assert!(
        matches!(&seen[..], [HookPayload::AgentEnd { text, iterations: 1, tool_calls: 0, .. }, HookPayload::MessageSending { content, .. }] if text == "original" && content == text)
    );
}

#[tokio::test]
async fn message_lifecycle_stream_block_only_publishes_error() {
    let (hooks, seen) = registry(HookAction::Block("policy".into()), false);
    let (output, runtime, committed, draft) = execute(Some(hooks), success_events()).await;
    assert!(output.is_none());
    assert!(committed);
    assert!(
        runtime
            .error
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("policy")
    );
    let broadcasts = runtime.broadcasts.lock().unwrap();
    assert_eq!(broadcasts.len(), 1);
    assert_eq!(broadcasts[0]["state"], "error");
    assert!(!draft.to_string().contains("original"));
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn message_lifecycle_stream_failure_has_no_completion_hooks() {
    for events in [
        vec![StreamEvent::Error("bad request".into())],
        vec![],
        vec![StreamEvent::Done(Usage::default())],
    ] {
        let (hooks, seen) = registry(HookAction::Continue, false);
        let (output, ..) = execute(Some(hooks), events).await;
        assert!(output.is_none());
        assert!(seen.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn message_lifecycle_stream_without_subscribers_keeps_deltas() {
    let (output, runtime, _, draft) = execute(None, success_events()).await;
    assert_eq!(output.unwrap().text, "original");
    assert!(
        runtime
            .broadcasts
            .lock()
            .unwrap()
            .iter()
            .any(|event| event["state"] == "delta")
    );
    assert!(draft.to_string().contains("original"));
}
