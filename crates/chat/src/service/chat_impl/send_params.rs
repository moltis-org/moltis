use {serde_json::Value, tracing::debug};

use {
    moltis_config::schema::AgentToolControls,
    moltis_service_traits::ServiceError,
    moltis_sessions::{ContentBlock, MessageContent},
    moltis_tools::policy::ToolPolicy,
};

use crate::{
    message::infer_reply_medium,
    types::{AssistantTurnOutput, ReplyMedium},
};

pub(super) struct SendParams {
    pub(super) history_limits: Option<(usize, usize)>,
    pub(super) text: String,
    pub(super) message_content: MessageContent,
    pub(super) desired_reply_medium: ReplyMedium,
    pub(super) conn_id: Option<String>,
    pub(super) explicit_model: Option<String>,
    pub(super) tool_controls: AgentToolControls,
    pub(super) request_tool_policy: Option<ToolPolicy>,
    pub(super) ephemeral: bool,
    pub(super) stream_only: bool,
}

pub(super) fn parse(params: &Value, has_tools: bool) -> Result<SendParams, ServiceError> {
    let history_limits = params
        .get("_history_limits")
        .and_then(Value::as_object)
        .and_then(|limits| {
            let max_messages = usize::try_from(limits.get("max_messages")?.as_u64()?).ok()?;
            let max_bytes = usize::try_from(limits.get("max_bytes")?.as_u64()?).ok()?;
            Some((max_messages, max_bytes))
        });
    let (text, message_content) = parse_message_content(params)?;
    let desired_reply_medium = infer_reply_medium(params, &text);
    let conn_id = params
        .get("_conn_id")
        .and_then(Value::as_str)
        .map(String::from);
    let explicit_model = params
        .get("model")
        .and_then(Value::as_str)
        .map(String::from);
    let tool_controls = AgentToolControls::from_tool_context(Some(params));
    let request_tool_policy = params
        .get("_tool_policy")
        .cloned()
        .map(serde_json::from_value::<ToolPolicy>)
        .transpose()
        .map_err(|error| format!("invalid '_tool_policy' parameter: {error}"))?;
    let ephemeral = params
        .get("_ephemeral")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let explicit_stream_only = params
        .get("stream_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stream_only = explicit_stream_only || !has_tools;
    debug!(
        explicit_stream_only,
        has_tools, stream_only, "send() mode decision"
    );

    Ok(SendParams {
        history_limits,
        text,
        message_content,
        desired_reply_medium,
        conn_id,
        explicit_model,
        tool_controls,
        request_tool_policy,
        ephemeral,
        stream_only,
    })
}

fn parse_message_content(params: &Value) -> Result<(String, MessageContent), ServiceError> {
    let Some(content) = params.get("content") else {
        let text = params
            .get("text")
            .or_else(|| params.get("message"))
            .and_then(Value::as_str)
            .ok_or_else(|| "missing 'text', 'message', or 'content' parameter".to_string())?
            .to_string();
        return Ok((text.clone(), MessageContent::Text(text)));
    };

    let text = content
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|block| block.get("text").and_then(Value::as_str))
        })
        .unwrap_or("[Image]")
        .to_string();
    let blocks = content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| match block.get("type")?.as_str()? {
                    "text" => Some(ContentBlock::text(block.get("text")?.as_str()?.to_string())),
                    "image_url" => Some(ContentBlock::ImageUrl {
                        image_url: moltis_sessions::message::ImageUrl {
                            url: block.get("image_url")?.get("url")?.as_str()?.to_string(),
                        },
                    }),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((text, MessageContent::Multimodal(blocks)))
}

pub(super) fn turn_result(output: &AssistantTurnOutput) -> Value {
    serde_json::json!({
        "text": output.text,
        "inputTokens": output.input_tokens,
        "outputTokens": output.output_tokens,
        "cacheReadTokens": output.cache_read_tokens,
        "cacheWriteTokens": output.cache_write_tokens,
        "durationMs": output.duration_ms,
        "requestInputTokens": output.request_input_tokens,
        "requestOutputTokens": output.request_output_tokens,
        "requestCacheReadTokens": output.request_cache_read_tokens,
        "requestCacheWriteTokens": output.request_cache_write_tokens,
    })
}
