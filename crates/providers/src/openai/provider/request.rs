use std::collections::{HashMap, HashSet};

use tracing::warn;

use {crate::raw_model_id, moltis_agents::model::ChatMessage};

use {
    super::OpenAiProvider,
    crate::openai::{CacheControlPolicy, SystemMessageRewriteStrategy},
};

impl OpenAiProvider {
    /// For OpenRouter Anthropic models, inject `cache_control` breakpoints
    /// on the system message and the last user message to enable prompt
    /// caching passthrough to Anthropic.
    pub(super) fn apply_openrouter_cache_control(&self, messages: &mut [serde_json::Value]) {
        if !matches!(
            self.capabilities.cache_control_policy,
            CacheControlPolicy::OpenRouterAnthropic
        ) || !self.model.starts_with("anthropic/")
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
        if !self.capabilities.non_strict_tools_model_prefixes.is_empty() {
            let raw_model = raw_model_id(&self.model).to_ascii_lowercase();
            if self
                .capabilities
                .non_strict_tools_model_prefixes
                .iter()
                .any(|prefix| raw_model.starts_with(prefix))
            {
                return false;
            }
        }
        self.capabilities.default_strict_tools
    }

    fn requires_reasoning_content_on_tool_messages(&self) -> bool {
        if let Some(explicit) = self.reasoning_content_override {
            return explicit;
        }
        if self.capabilities.default_reasoning_content_on_tool_messages {
            return true;
        }
        let raw_model = raw_model_id(&self.model).to_ascii_lowercase();
        self.capabilities
            .reasoning_content_model_prefixes
            .iter()
            .any(|prefix| raw_model.starts_with(prefix))
    }

    fn requires_gemini_tool_call_extra_content(&self) -> bool {
        self.capabilities.requires_gemini_tool_call_extra_content
    }

    /// Whether this provider rejects `null` in JSON Schema `enum` arrays.
    ///
    /// Fireworks AI returns 400 "could not translate the enum None" when
    /// any tool schema contains `null` in an `enum` array. For these
    /// providers, `strip_null_from_typed_enums` is applied after strict-mode
    /// patching so type-level nullability (`["string", "null"]`) remains
    /// but the redundant null is removed from enum arrays (issue #848).
    fn rejects_null_in_enums(&self) -> bool {
        self.capabilities.rejects_null_in_enums
    }

    fn omits_strict_tool_field(&self) -> bool {
        self.capabilities.omits_strict_tool_field
    }

    /// Convert raw tool schemas into the provider-compatible Chat
    /// Completions format, applying all provider-specific post-processing.
    ///
    /// Centralises strict-mode patching, null-enum stripping, and any
    /// future provider quirks so callers (streaming, completion) don't
    /// duplicate the logic.
    pub(super) fn prepare_chat_tools(&self, tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
        let mut converted = crate::openai_compat::to_openai_tools(tools, self.needs_strict_tools());

        if self.rejects_null_in_enums() {
            for tool in &mut converted {
                if let Some(params) = tool.pointer_mut("/function/parameters") {
                    crate::openai_compat::strip_null_from_typed_enums(params);
                }
            }
        }
        if self.omits_strict_tool_field() {
            for tool in &mut converted {
                if let Some(function) = tool
                    .get_mut("function")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    function.remove("strict");
                }
            }
        }

        converted
    }

    /// Some backends ship chat templates that only accept a single system
    /// message at the front of the conversation. Qwen-based OpenAI-compatible
    /// backends commonly behave this way (e.g. llama.cpp chat templates).
    fn requires_single_leading_system_message(&self) -> bool {
        if !self.capabilities.qwen_models_require_single_leading_system {
            return false;
        }
        raw_model_id(&self.model)
            .to_ascii_lowercase()
            .contains("qwen")
    }

    fn system_message_rewrite_strategy(&self) -> SystemMessageRewriteStrategy {
        if self.requires_single_leading_system_message() {
            return SystemMessageRewriteStrategy::MergeLeadingSystem;
        }
        self.capabilities.system_message_rewrite
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
        let needs_gemini_tool_call_extra_content = self.requires_gemini_tool_call_extra_content();
        let strip_name = !self.capabilities.supports_user_name;
        let mut remapped_tool_call_ids = HashMap::new();
        let mut used_tool_call_ids = HashSet::new();
        let mut out = Vec::with_capacity(messages.len());

        for message in messages {
            let assistant_reasoning = match message {
                ChatMessage::Assistant { reasoning, .. } => reasoning.as_deref(),
                _ => None,
            };
            let mut value = message.to_openai_value();

            // Strip the `name` field for providers that reject it entirely.
            if strip_name && let Some(obj) = value.as_object_mut() {
                obj.remove("name");
            }

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

                    if needs_gemini_tool_call_extra_content
                        && let Some(thought_signature) = tool_call
                            .as_object_mut()
                            .and_then(|obj| obj.remove("thought_signature"))
                    {
                        tool_call["extra_content"]["google"]["thought_signature"] =
                            thought_signature;
                    }
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
                    let reasoning_content = assistant_reasoning
                        .filter(|reasoning| !reasoning.trim().is_empty())
                        .map(str::to_string)
                        .or_else(|| {
                            value
                                .get("content")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_default();

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
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::openai::{OpenAiProviderCapabilities, ReasoningEffortPolicy};

    use secrecy::Secret;

    use super::*;

    fn next_test_secret_id() -> u64 {
        static NEXT_TEST_SECRET_ID: AtomicU64 = AtomicU64::new(1);
        NEXT_TEST_SECRET_ID.fetch_add(1, Ordering::Relaxed)
    }

    fn generated_api_key() -> Secret<String> {
        Secret::new(format!("k{:016x}", next_test_secret_id()))
    }

    fn provider(model: &str, provider_name: &str, base_url: &str) -> OpenAiProvider {
        OpenAiProvider::new_with_name(
            generated_api_key(),
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
        )
        .with_capabilities(OpenAiProviderCapabilities {
            qwen_models_require_single_leading_system: true,
            ..OpenAiProviderCapabilities::DEFAULT
        });
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
        let provider = provider("MiniMax-M2.7", "minimax", "https://api.minimax.io/v1")
            .with_capabilities(OpenAiProviderCapabilities {
                system_message_rewrite: SystemMessageRewriteStrategy::InlineIntoFirstUser,
                ..OpenAiProviderCapabilities::DEFAULT
            });
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
        )
        .with_capabilities(OpenAiProviderCapabilities {
            qwen_models_require_single_leading_system: true,
            ..OpenAiProviderCapabilities::DEFAULT
        });
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

    // ── strict_tools and reasoning_content overrides (issue #810) ───

    #[test]
    fn strict_tools_override_false_disables_strict() {
        let p = OpenAiProvider::new_with_name(
            generated_api_key(),
            "accounts/fireworks/routers/kimi-k2p5-turbo".into(),
            "https://api.fireworks.ai/inference/v1".into(),
            "fireworks".into(),
        )
        .with_strict_tools(false);
        assert!(
            !p.needs_strict_tools(),
            "strict_tools_override=false must disable strict tools (issue #810)"
        );
    }

    #[test]
    fn reasoning_content_override_true_enables_reasoning() {
        let p = OpenAiProvider::new_with_name(
            generated_api_key(),
            "accounts/fireworks/routers/kimi-k2p5-turbo".into(),
            "https://api.fireworks.ai/inference/v1".into(),
            "fireworks".into(),
        )
        .with_reasoning_content(true);
        assert!(
            p.requires_reasoning_content_on_tool_messages(),
            "reasoning_content_override=true must enable reasoning_content (issue #810)"
        );
    }

    #[test]
    fn fireworks_native_model_defaults_to_strict_tools() {
        let p = provider(
            "accounts/fireworks/models/glm-5p1",
            "fireworks",
            "https://api.fireworks.ai/inference/v1",
        );
        assert!(
            p.needs_strict_tools(),
            "Native Fireworks models should use strict tools by default"
        );
    }

    #[test]
    fn fireworks_rejects_null_in_enums() {
        let p = provider(
            "accounts/fireworks/models/glm-5p1",
            "fireworks",
            "https://api.fireworks.ai/inference/v1",
        )
        .with_capabilities(OpenAiProviderCapabilities {
            rejects_null_in_enums: true,
            ..OpenAiProviderCapabilities::DEFAULT
        });
        assert!(
            p.rejects_null_in_enums(),
            "Fireworks should reject null in enums (issue #848)"
        );
    }

    #[test]
    fn fireworks_rejects_null_in_enums_from_provider_flag() {
        let p = provider(
            "accounts/fireworks/routers/kimi-k2p5-turbo",
            "fireworks",
            "https://api.fireworks.ai/inference/v1",
        )
        .with_capabilities(OpenAiProviderCapabilities {
            rejects_null_in_enums: true,
            ..OpenAiProviderCapabilities::DEFAULT
        });
        assert!(
            p.rejects_null_in_enums(),
            "Fireworks provider flag should reject null in enums (issue #848)"
        );
    }

    #[test]
    fn openai_allows_null_in_enums() {
        let p = provider("gpt-4o", "openai", "https://api.openai.com/v1");
        assert!(
            !p.rejects_null_in_enums(),
            "OpenAI should allow null in enums (issue #712)"
        );
    }

    #[test]
    fn gemini_serializes_thought_signature_as_extra_content() {
        let p = provider(
            "gemini-3.1-flash-lite",
            "gemini",
            "https://generativelanguage.googleapis.com/v1beta/openai",
        )
        .with_capabilities(OpenAiProviderCapabilities {
            requires_gemini_tool_call_extra_content: true,
            ..OpenAiProviderCapabilities::DEFAULT
        });
        let mut metadata = serde_json::Map::new();
        metadata.insert("thought_signature".to_string(), serde_json::json!("sig123"));
        let messages =
            p.serialize_messages_for_request(&[ChatMessage::assistant_with_tools(None, vec![
                moltis_agents::model::ToolCall {
                    id: "call_1".to_string(),
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "London"}),
                    argument_diagnostic: None,
                    metadata: Some(metadata),
                },
            ])]);

        let tool_call = &messages[0]["tool_calls"][0];
        assert!(tool_call.get("thought_signature").is_none());
        assert_eq!(
            tool_call["extra_content"]["google"]["thought_signature"],
            "sig123"
        );
    }

    #[test]
    fn custom_provider_with_gemini_url_does_not_get_gemini_extra_content() {
        let p = provider(
            "gemini-3.1-flash-lite",
            "custom-gemini",
            "https://generativelanguage.googleapis.com/v1beta/openai",
        );
        let mut metadata = serde_json::Map::new();
        metadata.insert("thought_signature".to_string(), serde_json::json!("sig123"));

        let messages =
            p.serialize_messages_for_request(&[ChatMessage::assistant_with_tools(None, vec![
                moltis_agents::model::ToolCall {
                    id: "call_1".to_string(),
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "London"}),
                    argument_diagnostic: None,
                    metadata: Some(metadata),
                },
            ])]);

        let tool_call = &messages[0]["tool_calls"][0];
        assert_eq!(tool_call["thought_signature"], "sig123");
        assert!(tool_call.get("extra_content").is_none());
    }

    #[test]
    fn openrouter_cache_control_is_capability_driven() {
        let p = provider(
            "anthropic/claude-sonnet-4-20250514",
            "aliased-openrouter",
            "https://example.invalid/v1",
        )
        .with_capabilities(OpenAiProviderCapabilities {
            cache_control_policy: CacheControlPolicy::OpenRouterAnthropic,
            ..OpenAiProviderCapabilities::DEFAULT
        });
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "hello"}),
        ];

        p.apply_openrouter_cache_control(&mut messages);

        assert_eq!(
            messages[0]["content"][0]["cache_control"],
            serde_json::json!({"type": "ephemeral"})
        );
        assert_eq!(
            messages[1]["content"][0]["cache_control"],
            serde_json::json!({"type": "ephemeral"})
        );
    }

    #[test]
    fn custom_provider_with_openrouter_url_does_not_get_cache_control() {
        let p = provider(
            "anthropic/claude-sonnet-4-20250514",
            "custom-openrouter",
            "https://openrouter.ai/api/v1",
        );
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "hello"}),
        ];

        p.apply_openrouter_cache_control(&mut messages);

        assert_eq!(messages[0]["content"], "sys");
        assert_eq!(messages[1]["content"], "hello");
    }

    #[test]
    fn fireworks_native_model_no_reasoning_content() {
        let p = provider(
            "accounts/fireworks/models/glm-5p1",
            "fireworks",
            "https://api.fireworks.ai/inference/v1",
        );
        assert!(
            !p.requires_reasoning_content_on_tool_messages(),
            "Native Fireworks models should not add reasoning_content"
        );
    }

    #[test]
    fn moonshot_direct_auto_detects_reasoning_content() {
        let p = provider("kimi-k2.5", "moonshot", "https://api.moonshot.ai/v1").with_capabilities(
            OpenAiProviderCapabilities {
                default_reasoning_content_on_tool_messages: true,
                ..OpenAiProviderCapabilities::DEFAULT
            },
        );
        assert!(p.requires_reasoning_content_on_tool_messages());
    }

    #[test]
    fn deepseek_v4_auto_detects_reasoning_content() {
        let p = provider("deepseek-v4-flash", "deepseek", "https://api.deepseek.com")
            .with_capabilities(OpenAiProviderCapabilities {
                reasoning_content_model_prefixes: &["deepseek-v4"],
                ..OpenAiProviderCapabilities::DEFAULT
            });
        assert!(
            p.requires_reasoning_content_on_tool_messages(),
            "DeepSeek V4 thinking-mode tool calls require reasoning_content replay (issue #959)"
        );
    }

    #[test]
    fn deepseek_non_v4_does_not_auto_detect_reasoning_content() {
        let p = provider("deepseek-chat", "deepseek", "https://api.deepseek.com")
            .with_capabilities(OpenAiProviderCapabilities {
                reasoning_content_model_prefixes: &["deepseek-v4"],
                ..OpenAiProviderCapabilities::DEFAULT
            });
        assert!(
            !p.requires_reasoning_content_on_tool_messages(),
            "DeepSeek reasoning_content replay should stay scoped to V4 thinking models"
        );
    }

    #[test]
    fn deepseek_v4_reasoning_effort_enables_thinking_and_maps_xhigh_to_max() {
        let mut p = provider("deepseek-v4-pro", "deepseek", "https://api.deepseek.com")
            .with_capabilities(OpenAiProviderCapabilities {
                reasoning_effort_policy: ReasoningEffortPolicy::DeepSeek,
                ..OpenAiProviderCapabilities::DEFAULT
            });
        p.reasoning_effort = Some(moltis_agents::model::ReasoningEffort::ExtraHigh);
        let mut body = serde_json::json!({
            "model": "deepseek-v4-pro",
            "messages": [{"role": "user", "content": "hello"}],
        });

        p.apply_reasoning_effort_chat(&mut body);

        assert_eq!(body["reasoning_effort"], "max");
        assert_eq!(body["thinking"], serde_json::json!({ "type": "enabled" }));
    }

    #[test]
    fn deepseek_v4_reasoning_effort_maps_lower_levels_to_high() {
        for effort in [
            moltis_agents::model::ReasoningEffort::Minimal,
            moltis_agents::model::ReasoningEffort::Low,
            moltis_agents::model::ReasoningEffort::Medium,
            moltis_agents::model::ReasoningEffort::High,
        ] {
            let mut p = provider("deepseek-v4-flash", "deepseek", "https://api.deepseek.com")
                .with_capabilities(OpenAiProviderCapabilities {
                    reasoning_effort_policy: ReasoningEffortPolicy::DeepSeek,
                    ..OpenAiProviderCapabilities::DEFAULT
                });
            p.reasoning_effort = Some(effort);
            assert_eq!(p.reasoning_effort_str(), Some("high"));
        }
    }

    // ── Wire-format tests: verify serialized request body (issue #810) ──

    /// Kimi router with strict_tools=false must NOT emit `"strict": true` in
    /// the serialized tool schemas. This is the actual payload that caused the
    /// 400 error in issue #810.
    #[test]
    fn kimi_router_tool_schema_omits_strict_field() {
        use crate::openai_compat::to_openai_tools;

        let p = provider(
            "accounts/fireworks/routers/kimi-k2p5-turbo",
            "fireworks",
            "https://api.fireworks.ai/inference/v1",
        )
        .with_strict_tools(false);

        let tools = vec![serde_json::json!({
            "name": "get_weather",
            "description": "Get weather",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": { "type": "string" }
                },
                "required": ["location"]
            }
        })];

        let serialized = to_openai_tools(&tools, p.needs_strict_tools());
        assert_eq!(serialized.len(), 1);

        let strict_val = serialized[0]["function"]["strict"].as_bool();
        assert_eq!(
            strict_val,
            Some(false),
            "Kimi router tools must have strict=false, got: {:?}",
            serialized[0]
        );
    }

    #[test]
    fn nearai_tool_schema_omits_strict_field() {
        let p = provider(
            "zai-org/GLM-5.1-FP8",
            "nearai",
            "https://cloud-api.near.ai/v1",
        )
        .with_capabilities(OpenAiProviderCapabilities {
            omits_strict_tool_field: true,
            reasoning_effort_policy: ReasoningEffortPolicy::Unsupported,
            ..OpenAiProviderCapabilities::DEFAULT
        });
        let tools = vec![serde_json::json!({
            "name": "get_weather",
            "description": "Get weather",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": { "type": "string" }
                }
            }
        })];

        let converted = p.prepare_chat_tools(&tools);
        assert!(
            converted[0]["function"].get("strict").is_none(),
            "NEAR AI Cloud should not receive the unsupported strict tool field"
        );
    }

    #[test]
    fn custom_provider_with_nearai_url_does_not_omit_strict_field() {
        let p = provider(
            "zai-org/GLM-5.1-FP8",
            "custom-nearai",
            "https://cloud-api.near.ai/v1",
        );
        let tools = vec![serde_json::json!({
            "name": "get_weather",
            "description": "Get weather",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": { "type": "string" }
                }
            }
        })];

        let converted = p.prepare_chat_tools(&tools);

        assert_eq!(converted[0]["function"]["strict"], true);
    }

    /// Kimi router with reasoning_content=true must inject `reasoning_content`
    /// into assistant messages that carry tool calls. Without this, the Kimi
    /// backend rejects the multi-turn request.
    #[test]
    fn kimi_router_injects_reasoning_content_on_tool_call_messages() {
        let p = provider(
            "accounts/fireworks/routers/kimi-k2p5-turbo",
            "fireworks",
            "https://api.fireworks.ai/inference/v1",
        )
        .with_reasoning_content(true);

        let messages = vec![
            ChatMessage::user("What's the weather?"),
            ChatMessage::assistant_with_tools(Some("thinking about weather".to_string()), vec![
                moltis_agents::model::ToolCall {
                    id: "call_123".to_string(),
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Berlin"}),
                    argument_diagnostic: None,
                    metadata: None,
                },
            ]),
            ChatMessage::tool("call_123", r#"{"temperature": 20}"#),
        ];

        let serialized = p.serialize_messages_for_request(&messages);
        assert_eq!(serialized.len(), 3);

        let assistant_msg = &serialized[1];
        assert_eq!(assistant_msg["role"], "assistant");
        assert!(
            assistant_msg.get("reasoning_content").is_some(),
            "assistant tool-call message must have reasoning_content, got: {assistant_msg}"
        );
    }

    #[test]
    fn deepseek_v4_replays_persisted_tool_reasoning_content() {
        let p = provider("deepseek-v4-flash", "deepseek", "https://api.deepseek.com")
            .with_capabilities(OpenAiProviderCapabilities {
                reasoning_content_model_prefixes: &["deepseek-v4"],
                ..OpenAiProviderCapabilities::DEFAULT
            });
        let persisted = vec![
            serde_json::json!({"role": "user", "content": "What is the weather?"}),
            serde_json::json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_959",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"location\":\"Berlin\"}"
                    }
                }]
            }),
            serde_json::json!({
                "role": "tool_result",
                "tool_call_id": "call_959",
                "tool_name": "get_weather",
                "success": true,
                "result": {"temperature": 20},
                "reasoning": "Need live weather before answering."
            }),
            serde_json::json!({"role": "assistant", "content": "It is 20 C."}),
            serde_json::json!({"role": "user", "content": "What about tomorrow?"}),
        ];
        let messages = moltis_agents::model::values_to_chat_messages(&persisted);

        let serialized = p.serialize_messages_for_request(&messages);

        let Some(assistant_tool_message) = serialized.iter().find(|message| {
            message.get("role").and_then(serde_json::Value::as_str) == Some("assistant")
                && message.get("tool_calls").is_some()
        }) else {
            panic!("assistant tool-call message should be serialized");
        };
        assert_eq!(
            assistant_tool_message["reasoning_content"],
            "Need live weather before answering."
        );
        assert_eq!(assistant_tool_message["content"], "");
    }

    /// Mistral provider must strip the `name` field from user messages.
    #[test]
    fn mistral_provider_strips_user_name() {
        let p = provider(
            "mistral-small-latest",
            "mistral",
            "https://api.mistral.ai/v1",
        )
        .with_capabilities(OpenAiProviderCapabilities {
            supports_user_name: false,
            ..OpenAiProviderCapabilities::DEFAULT
        });
        assert!(!p.capabilities.supports_user_name);

        let messages = vec![ChatMessage::user_named("hello", "rokku")];
        let serialized = p.serialize_messages_for_request(&messages);
        assert_eq!(serialized.len(), 1);
        assert!(
            serialized[0].get("name").is_none(),
            "Mistral must not have name field, got: {}",
            serialized[0]
        );
    }

    /// MiniMax rejects chat histories containing inconsistent user `name` values.
    #[test]
    fn minimax_provider_strips_user_names_from_group_chat_history() {
        let p = provider("MiniMax-M2.7", "minimax", "https://api.minimax.io/v1").with_capabilities(
            OpenAiProviderCapabilities {
                supports_user_name: false,
                ..OpenAiProviderCapabilities::DEFAULT
            },
        );
        assert!(!p.capabilities.supports_user_name);

        let messages = vec![
            ChatMessage::user_named("hello", "Alice"),
            ChatMessage::assistant("hi"),
            ChatMessage::user_named("jumping in", "Bob"),
        ];
        let serialized = p.serialize_messages_for_request(&messages);

        assert_eq!(serialized.len(), 3);
        assert!(serialized[0].get("name").is_none());
        assert!(serialized[2].get("name").is_none());
    }

    /// OpenAI provider must preserve the (sanitized) `name` field.
    #[test]
    fn openai_provider_preserves_user_name() {
        let p = provider("gpt-4o", "openai", "https://api.openai.com/v1");
        assert!(p.capabilities.supports_user_name);

        let messages = vec![ChatMessage::user_named("hello", "Alice")];
        let serialized = p.serialize_messages_for_request(&messages);
        assert_eq!(serialized[0]["name"], "Alice");
    }

    #[test]
    fn openai_provider_trims_base_url_and_api_key_edges() {
        let p = OpenAiProvider::new_with_name(
            Secret::new(" test-key\n".to_string()),
            "gpt-4o".to_string(),
            " https://api.openai.com/v1/ \n".to_string(),
            "openai".to_string(),
        );

        assert_eq!(
            p.chat_completions_url(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(p.responses_sse_url(), "https://api.openai.com/v1/responses");
        assert_eq!(p.bearer_auth_header(), "Bearer test-key");
    }

    /// Explicit capabilities can override the default.
    #[test]
    fn supports_user_name_can_be_overridden() {
        let p = provider("gpt-4o", "openai", "https://api.openai.com/v1").with_capabilities(
            OpenAiProviderCapabilities {
                supports_user_name: false,
                ..OpenAiProviderCapabilities::DEFAULT
            },
        );

        let messages = vec![ChatMessage::user_named("hello", "Alice")];
        let serialized = p.serialize_messages_for_request(&messages);
        assert!(
            serialized[0].get("name").is_none(),
            "name should be stripped when supports_user_name=false"
        );
    }

    /// Native Fireworks model (no overrides) must NOT inject reasoning_content.
    #[test]
    fn fireworks_native_model_no_reasoning_content_in_serialized_messages() {
        let p = provider(
            "accounts/fireworks/models/glm-5p1",
            "fireworks",
            "https://api.fireworks.ai/inference/v1",
        )
        .with_capabilities(OpenAiProviderCapabilities {
            rejects_null_in_enums: true,
            ..OpenAiProviderCapabilities::DEFAULT
        });

        let messages = vec![
            ChatMessage::user("What's the weather?"),
            ChatMessage::assistant_with_tools(Some("let me check".to_string()), vec![
                moltis_agents::model::ToolCall {
                    id: "call_456".to_string(),
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Paris"}),
                    argument_diagnostic: None,
                    metadata: None,
                },
            ]),
            ChatMessage::tool("call_456", r#"{"temperature": 15}"#),
        ];

        let serialized = p.serialize_messages_for_request(&messages);
        let assistant_msg = &serialized[1];
        assert!(
            assistant_msg.get("reasoning_content").is_none(),
            "native Fireworks model must NOT have reasoning_content, got: {assistant_msg}"
        );
    }
}
