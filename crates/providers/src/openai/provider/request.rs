use std::collections::{HashMap, HashSet};

use tracing::warn;

use {crate::raw_model_id, moltis_agents::model::ChatMessage};

use super::OpenAiProvider;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SystemMessageRewriteStrategy {
    None,
    MergeLeadingSystem,
    InlineIntoFirstUser,
}

impl OpenAiProvider {
    /// Returns `true` when this provider targets an Anthropic model via
    /// OpenRouter, which supports prompt caching when `cache_control`
    /// breakpoints are present in the message payload.
    fn is_openrouter_anthropic(&self) -> bool {
        self.base_url.contains("openrouter.ai") && self.model.starts_with("anthropic/")
    }

    /// For OpenRouter Anthropic models, inject `cache_control` breakpoints
    /// on the system message and the last user message to enable prompt
    /// caching passthrough to Anthropic.
    pub(super) fn apply_openrouter_cache_control(&self, messages: &mut [serde_json::Value]) {
        if !self.is_openrouter_anthropic()
            || matches!(self.cache_retention, moltis_config::CacheRetention::None)
        {
            return;
        }

        let cache_control = serde_json::json!({ "type": "ephemeral" });

        // Add cache_control to the system message content.
        for msg in messages.iter_mut() {
            if msg.get("role").and_then(serde_json::Value::as_str) != Some("system") {
                continue;
            }
            match msg.get_mut("content") {
                Some(content) if content.is_string() => {
                    let text = content.as_str().unwrap_or_default().to_string();
                    msg["content"] = serde_json::json!([{
                        "type": "text",
                        "text": text,
                        "cache_control": cache_control
                    }]);
                },
                Some(content) if content.is_array() => {
                    if let Some(last) = content.as_array_mut().and_then(|a| a.last_mut()) {
                        last["cache_control"] = cache_control.clone();
                    }
                },
                _ => {},
            }
            break;
        }

        // Add cache_control to the last user message.
        if let Some(last_user) = messages
            .iter_mut()
            .rev()
            .find(|m| m.get("role").and_then(serde_json::Value::as_str) == Some("user"))
        {
            match last_user.get_mut("content") {
                Some(content) if content.is_string() => {
                    let text = content.as_str().unwrap_or_default().to_string();
                    last_user["content"] = serde_json::json!([{
                        "type": "text",
                        "text": text,
                        "cache_control": cache_control
                    }]);
                },
                Some(content) if content.is_array() => {
                    if let Some(last) = content.as_array_mut().and_then(|a| a.last_mut()) {
                        last["cache_control"] = cache_control;
                    }
                },
                _ => {},
            }
        }
    }

    /// Returns `true` when tool schemas should use OpenAI strict mode.
    ///
    /// Strict mode is an OpenAI-specific feature that adds `additionalProperties:
    /// false` and forces all properties into the `required` array (making
    /// originally-optional ones nullable via array-form types like
    /// `["boolean", "null"]`).
    ///
    /// The `strict_tools` config field overrides auto-detection when set.
    /// When unset, providers whose backends reject array-form types default to
    /// non-strict: OpenRouter (proxies to Google, Anthropic, Meta, etc.),
    /// Gemini direct, and Vertex AI (`googleapis.com`).
    pub(super) fn needs_strict_tools(&self) -> bool {
        if let Some(explicit) = self.strict_tools_override {
            return explicit;
        }
        if self.base_url.contains("openrouter.ai") {
            return false;
        }
        if self.provider_name.eq_ignore_ascii_case("gemini")
            || self.base_url.contains("googleapis.com")
        {
            return false;
        }
        true
    }

    fn requires_reasoning_content_on_tool_messages(&self) -> bool {
        self.provider_name.eq_ignore_ascii_case("moonshot")
            || self.base_url.contains("moonshot.ai")
            || self.base_url.contains("moonshot.cn")
            || self.model.starts_with("kimi-")
    }

    /// Some providers (e.g. MiniMax) reject `role: "system"` in the messages
    /// array. System content must be extracted and prepended to the first user
    /// message instead (MiniMax silently ignores a top-level `"system"` field).
    fn rejects_system_role(&self) -> bool {
        self.model.starts_with("MiniMax-")
            || self.provider_name.eq_ignore_ascii_case("minimax")
            || self.base_url.to_ascii_lowercase().contains("minimax")
    }

    fn is_custom_openai_compatible_provider(&self) -> bool {
        self.provider_name.starts_with("custom-")
    }

    fn is_alibaba_qwen_backend(&self) -> bool {
        self.provider_name.eq_ignore_ascii_case("alibaba-coding")
            || self.provider_name.eq_ignore_ascii_case("alibaba")
            || self.provider_name.eq_ignore_ascii_case("dashscope-coding")
            || self.base_url.contains("dashscope.aliyuncs.com")
            || self.base_url.contains("alibabacloud.com")
    }

    fn is_qwen_single_system_backend(&self) -> bool {
        self.provider_name.eq_ignore_ascii_case("ollama")
            || self.provider_name.to_ascii_lowercase().contains("ollama")
            || self.is_custom_openai_compatible_provider()
            || self.is_alibaba_qwen_backend()
    }

    /// Some backends ship chat templates that only accept a single system
    /// message at the front of the conversation. Qwen-based OpenAI-compatible
    /// backends commonly behave this way (e.g. llama.cpp chat templates).
    fn requires_single_leading_system_message(&self) -> bool {
        raw_model_id(&self.model)
            .to_ascii_lowercase()
            .contains("qwen")
            && self.is_qwen_single_system_backend()
    }

    fn system_message_rewrite_strategy(&self) -> SystemMessageRewriteStrategy {
        if self.rejects_system_role() {
            return SystemMessageRewriteStrategy::InlineIntoFirstUser;
        }
        if self.requires_single_leading_system_message() {
            return SystemMessageRewriteStrategy::MergeLeadingSystem;
        }
        SystemMessageRewriteStrategy::None
    }

    /// Rewrite system messages for providers with stricter chat template rules.
    ///
    /// MiniMax's `/v1/chat/completions` endpoint returns error 2013 for
    /// `role: "system"` entries and silently ignores a top-level `"system"`
    /// field. The only reliable way to deliver the system prompt is to
    /// inline it into the first user message.
    ///
    /// Qwen-based OpenAI-compatible backends often only accept a single system
    /// message at the very front. For those, join all system messages with
    /// blank lines and emit exactly one leading `role: "system"` message.
    ///
    /// Must be called on the request body **after** it is fully assembled.
    pub(super) fn apply_system_prompt_rewrite(&self, body: &mut serde_json::Value) {
        let rewrite_strategy = self.system_message_rewrite_strategy();
        if matches!(rewrite_strategy, SystemMessageRewriteStrategy::None) {
            return;
        }
        let Some(messages) = body
            .get_mut("messages")
            .and_then(serde_json::Value::as_array_mut)
        else {
            return;
        };
        let mut system_parts = Vec::new();
        messages.retain(|msg| {
            if msg.get("role").and_then(serde_json::Value::as_str) == Some("system") {
                if let Some(content) = msg.get("content").and_then(serde_json::Value::as_str)
                    && !content.is_empty()
                {
                    system_parts.push(content.to_string());
                } else if msg.get("content").is_some() {
                    warn!(
                        ?rewrite_strategy,
                        "system message has non-string content; it will be dropped"
                    );
                }
                return false;
            }
            true
        });
        if system_parts.is_empty() {
            return;
        }
        let system_text = system_parts.join("\n\n");

        if matches!(
            rewrite_strategy,
            SystemMessageRewriteStrategy::MergeLeadingSystem
        ) {
            messages.insert(
                0,
                serde_json::json!({
                    "role": "system",
                    "content": system_text,
                }),
            );
            return;
        }

        // Find the first user message and prepend system content to it.
        let system_block =
            format!("[System Instructions]\n{system_text}\n[End System Instructions]\n\n");
        if let Some(first_user) = messages
            .iter_mut()
            .find(|m| m.get("role").and_then(serde_json::Value::as_str) == Some("user"))
        {
            match first_user.get("content").cloned() {
                Some(serde_json::Value::String(s)) => {
                    first_user["content"] = serde_json::Value::String(format!("{system_block}{s}"));
                },
                Some(serde_json::Value::Array(mut arr)) => {
                    // Multimodal content (text + images): prepend as a text block.
                    arr.insert(
                        0,
                        serde_json::json!({ "type": "text", "text": system_block }),
                    );
                    first_user["content"] = serde_json::Value::Array(arr);
                },
                _ => {
                    first_user["content"] = serde_json::Value::String(system_block);
                },
            }
        } else {
            // No user message yet (e.g. probe); insert a synthetic user message.
            messages.insert(
                0,
                serde_json::json!({
                    "role": "user",
                    "content": format!("[System Instructions]\n{system_text}\n[End System Instructions]")
                }),
            );
        }
    }

    pub(super) fn serialize_messages_for_request(
        &self,
        messages: &[ChatMessage],
    ) -> Vec<serde_json::Value> {
        let needs_reasoning_content = self.requires_reasoning_content_on_tool_messages();
        let mut remapped_tool_call_ids = HashMap::new();
        let mut used_tool_call_ids = HashSet::new();
        let mut out = Vec::with_capacity(messages.len());

        for message in messages {
            let mut value = message.to_openai_value();

            if let Some(tool_calls) = value
                .get_mut("tool_calls")
                .and_then(serde_json::Value::as_array_mut)
            {
                for tool_call in tool_calls {
                    let Some(tool_call_id) =
                        tool_call.get("id").and_then(serde_json::Value::as_str)
                    else {
                        continue;
                    };
                    let mapped_id = assign_openai_tool_call_id(
                        tool_call_id,
                        &mut remapped_tool_call_ids,
                        &mut used_tool_call_ids,
                    );
                    tool_call["id"] = serde_json::Value::String(mapped_id);
                }
            } else if value.get("role").and_then(serde_json::Value::as_str) == Some("tool")
                && let Some(tool_call_id) = value
                    .get("tool_call_id")
                    .and_then(serde_json::Value::as_str)
            {
                let mapped_id = remapped_tool_call_ids
                    .get(tool_call_id)
                    .cloned()
                    .unwrap_or_else(|| {
                        assign_openai_tool_call_id(
                            tool_call_id,
                            &mut remapped_tool_call_ids,
                            &mut used_tool_call_ids,
                        )
                    });
                value["tool_call_id"] = serde_json::Value::String(mapped_id);
            }

            if needs_reasoning_content {
                let is_assistant =
                    value.get("role").and_then(serde_json::Value::as_str) == Some("assistant");
                let has_tool_calls = value
                    .get("tool_calls")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|calls| !calls.is_empty());

                if is_assistant && has_tool_calls {
                    let reasoning_content = value
                        .get("content")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();

                    if value.get("content").is_none() {
                        value["content"] = serde_json::Value::String(String::new());
                    }

                    if value.get("reasoning_content").is_none() {
                        value["reasoning_content"] = serde_json::Value::String(reasoning_content);
                    }
                }
            }

            out.push(value);
        }

        out
    }
}

const OPENAI_MAX_TOOL_CALL_ID_LEN: usize = 40;

fn short_stable_hash(value: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn base_openai_tool_call_id(raw: &str) -> String {
    let mut cleaned: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if cleaned.is_empty() {
        cleaned = "call".to_string();
    }

    if cleaned.len() <= OPENAI_MAX_TOOL_CALL_ID_LEN {
        return cleaned;
    }

    let hash = short_stable_hash(raw);
    let keep = OPENAI_MAX_TOOL_CALL_ID_LEN.saturating_sub(hash.len() + 1);
    cleaned.truncate(keep);
    if cleaned.is_empty() {
        return format!("call-{hash}");
    }
    format!("{cleaned}-{hash}")
}

fn disambiguate_tool_call_id(base: &str, nonce: usize) -> String {
    let suffix = format!("-{nonce}");
    let keep = OPENAI_MAX_TOOL_CALL_ID_LEN.saturating_sub(suffix.len());

    let mut value = base.to_string();
    if value.len() > keep {
        value.truncate(keep);
    }
    if value.is_empty() {
        value = "call".to_string();
        if value.len() > keep {
            value.truncate(keep);
        }
    }
    format!("{value}{suffix}")
}

fn assign_openai_tool_call_id(
    raw: &str,
    remapped_tool_call_ids: &mut HashMap<String, String>,
    used_tool_call_ids: &mut HashSet<String>,
) -> String {
    if let Some(existing) = remapped_tool_call_ids.get(raw) {
        return existing.clone();
    }

    let base = base_openai_tool_call_id(raw);
    let mut candidate = base.clone();
    let mut nonce = 1usize;
    while used_tool_call_ids.contains(&candidate) {
        candidate = disambiguate_tool_call_id(&base, nonce);
        nonce = nonce.saturating_add(1);
    }

    used_tool_call_ids.insert(candidate.clone());
    remapped_tool_call_ids.insert(raw.to_string(), candidate.clone());
    candidate
}

#[cfg(test)]
mod tests {
    use secrecy::Secret;

    use super::*;

    fn provider(model: &str, provider_name: &str, base_url: &str) -> OpenAiProvider {
        OpenAiProvider::new_with_name(
            Secret::new("test-key".to_string()),
            model.to_string(),
            base_url.to_string(),
            provider_name.to_string(),
        )
    }

    fn body_messages(body: &serde_json::Value) -> &[serde_json::Value] {
        let Some(messages) = body.get("messages").and_then(serde_json::Value::as_array) else {
            panic!("messages should be an array");
        };
        messages
    }

    #[test]
    fn system_message_rewrite_qwen_merges_multiple_messages_into_one_leading_message() {
        let provider = provider(
            "qwen3:0.6b",
            "custom-ollama-qwen",
            "http://127.0.0.1:11435/v1",
        );
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "hi"},
                {"role": "system", "content": "The current user datetime is 2026-04-15 18:22:00 UTC."},
                {"role": "user", "content": "what time is it?"}
            ]
        });

        provider.apply_system_prompt_rewrite(&mut body);

        let messages = body_messages(&body);
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(
            messages[0]["content"],
            "You are a helpful assistant.\n\nThe current user datetime is 2026-04-15 18:22:00 UTC."
        );
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[3]["role"], "user");
    }

    #[test]
    fn system_message_rewrite_minimax_inlines_messages_into_first_user_message() {
        let provider = provider("MiniMax-M2.7", "minimax", "https://api.minimax.io/v1");
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "hello"},
                {"role": "system", "content": "The current user datetime is 2026-04-15 18:22:00 UTC."}
            ]
        });

        provider.apply_system_prompt_rewrite(&mut body);

        let messages = body_messages(&body);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(
            messages[0]["content"],
            "[System Instructions]\nYou are a helpful assistant.\n\nThe current user datetime is 2026-04-15 18:22:00 UTC.\n[End System Instructions]\n\nhello"
        );
    }

    #[test]
    fn system_message_rewrite_default_openai_request_is_unchanged() {
        let provider = provider("gpt-4o-mini", "openai", "https://api.openai.com/v1");
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "sys1"},
                {"role": "user", "content": "hello"},
                {"role": "system", "content": "sys2"}
            ]
        });

        provider.apply_system_prompt_rewrite(&mut body);

        let messages = body_messages(&body);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "system");
    }

    #[test]
    fn system_message_rewrite_qwen_model_on_openai_provider_is_unchanged() {
        let provider = provider("qwen3-coder-plus", "openai", "https://api.openai.com/v1");
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "sys1"},
                {"role": "user", "content": "hello"},
                {"role": "system", "content": "sys2"}
            ]
        });

        provider.apply_system_prompt_rewrite(&mut body);

        let messages = body_messages(&body);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "sys1");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "system");
        assert_eq!(messages[2]["content"], "sys2");
    }

    #[test]
    fn system_message_rewrite_alibaba_qwen_merges_multiple_messages_into_one_leading_message() {
        let provider = provider(
            "qwen3.5-plus",
            "alibaba-coding",
            "https://coding-intl.dashscope.aliyuncs.com/v1",
        );
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "sys1"},
                {"role": "user", "content": "hello"},
                {"role": "system", "content": "sys2"}
            ]
        });

        provider.apply_system_prompt_rewrite(&mut body);

        let messages = body_messages(&body);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "sys1\n\nsys2");
        assert_eq!(messages[1]["role"], "user");
    }
}
