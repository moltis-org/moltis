use {
    super::helpers::*,
    crate::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, ToolCall, Usage},
    async_trait::async_trait,
    moltis_common::hooks::{HookAction, HookEvent, HookHandler, HookPayload, HookRegistry},
    std::{
        collections::VecDeque,
        pin::Pin,
        sync::{Arc, Mutex},
    },
    tokio_stream::Stream,
};

struct EndHook {
    payloads: Arc<Mutex<Vec<HookPayload>>>,
}

#[async_trait]
impl HookHandler for EndHook {
    fn name(&self) -> &str {
        "agent-end-recorder"
    }

    fn events(&self) -> &[HookEvent] {
        &[HookEvent::AgentEnd]
    }

    async fn handle(
        &self,
        _event: HookEvent,
        payload: &HookPayload,
    ) -> moltis_common::error::Result<HookAction> {
        self.payloads.lock().unwrap().push(payload.clone());
        // AgentEnd is read-only: a handler cannot block a completed run.
        Ok(HookAction::Block("ignored at completion".into()))
    }
}

struct ScriptedProvider {
    responses: Mutex<VecDeque<anyhow::Result<CompletionResponse>>>,
    streaming: bool,
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "agent-end-test"
    }

    fn id(&self) -> &str {
        "agent-end-test-model"
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        assert!(!self.streaming, "streaming test must use the actual stream");
        self.responses.lock().unwrap().pop_front().unwrap()
    }

    fn stream(
        &self,
        _messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        panic!("runner must use stream_with_tools")
    }

    fn stream_with_tools(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        assert!(self.streaming);
        let response = self.responses.lock().unwrap().pop_front().unwrap();
        let mut events = Vec::new();
        match response {
            Ok(response) => {
                if let Some(text) = response.text {
                    events.push(StreamEvent::Delta(text));
                }
                for (index, call) in response.tool_calls.into_iter().enumerate() {
                    events.push(StreamEvent::ToolCallStart {
                        id: call.id,
                        name: call.name,
                        index,
                        metadata: None,
                    });
                    events.push(StreamEvent::ToolCallArgumentsDelta {
                        index,
                        delta: call.arguments.to_string(),
                    });
                    events.push(StreamEvent::ToolCallComplete { index });
                }
                events.push(StreamEvent::Done(response.usage));
            },
            Err(error) => events.push(StreamEvent::Error(error.to_string())),
        }
        Box::pin(tokio_stream::iter(events))
    }
}

async fn check_agent_end(streaming: bool, with_tools: bool, fail: bool) {
    let mut responses = VecDeque::new();
    let mut tools = ToolRegistry::new();
    if with_tools {
        tools.register(Box::new(EchoTool));
        // Five calls over two rounds distinguishes iterations from tool totals.
        for round in 0..2 {
            responses.push_back(Ok(CompletionResponse {
                text: None,
                tool_calls: (0..round + 2)
                    .map(|index| ToolCall {
                        id: format!("call-{round}-{index}"),
                        name: "echo_tool".into(),
                        arguments: serde_json::json!({"text": format!("{round}-{index}")}),
                        argument_diagnostic: None,
                        metadata: None,
                    })
                    .collect(),
                usage: Usage::default(),
            }));
        }
    }
    let final_text = GH_628_LONG_ANSWER;
    responses.push_back(if fail {
        Err(anyhow::anyhow!("invalid API key"))
    } else {
        Ok(CompletionResponse {
            text: Some(format!("  {final_text}  ")),
            tool_calls: vec![],
            usage: Usage::default(),
        })
    });
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(responses),
        streaming,
    });
    let payloads = Arc::new(Mutex::new(Vec::new()));
    let mut hooks = HookRegistry::new();
    hooks.register(Arc::new(EndHook {
        payloads: Arc::clone(&payloads),
    }));
    let hooks = Some(Arc::new(hooks));
    let context = Some(serde_json::json!({"_session_key": "agent-end-session"}));
    let user = UserContent::text("Please answer the question.");
    let result = if streaming {
        run_agent_loop_streaming(
            provider.clone(),
            &tools,
            "Test bot",
            &user,
            None,
            None,
            context,
            hooks,
            None,
            None,
        )
        .await
    } else {
        run_agent_loop_with_context(
            provider.clone(),
            &tools,
            "Test bot",
            &user,
            None,
            None,
            context,
            hooks,
            None,
        )
        .await
    };
    assert!(provider.responses.lock().unwrap().is_empty());
    let payloads = payloads.lock().unwrap();
    if fail {
        assert!(result.is_err());
        assert!(
            payloads.is_empty(),
            "errors must not fabricate successful totals"
        );
        return;
    }
    let result = result.unwrap();
    let (iterations, tool_calls) = if with_tools {
        (3, 5)
    } else {
        (1, 0)
    };
    assert_eq!(result.text, final_text);
    assert_eq!(result.iterations, iterations);
    assert_eq!(result.tool_calls_made, tool_calls);
    assert_eq!(payloads.len(), 1, "AgentEnd must fire exactly once");
    assert!(matches!(
        &payloads[0],
        HookPayload::AgentEnd { session_key, text, iterations: actual_iterations, tool_calls: actual_calls }
            if session_key == "agent-end-session"
                && text == &result.text
                && *actual_iterations == iterations
                && *actual_calls == tool_calls
    ));
}

#[tokio::test]
async fn agent_end_streaming_without_tools() {
    check_agent_end(true, false, false).await;
}

#[tokio::test]
async fn agent_end_streaming_with_tools() {
    check_agent_end(true, true, false).await;
}

#[tokio::test]
async fn agent_end_non_streaming_without_tools() {
    check_agent_end(false, false, false).await;
}

#[tokio::test]
async fn agent_end_non_streaming_with_tools() {
    check_agent_end(false, true, false).await;
}

#[tokio::test]
async fn agent_end_streaming_error_does_not_dispatch() {
    check_agent_end(true, false, true).await;
    check_agent_end(true, true, true).await;
}

#[tokio::test]
async fn agent_end_non_streaming_error_does_not_dispatch() {
    check_agent_end(false, false, true).await;
    check_agent_end(false, true, true).await;
}
