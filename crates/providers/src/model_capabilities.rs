//! Model capability heuristics: context window, tool support, vision, reasoning.

use crate::model_id::capability_model_id;
use moltis_config::schema::ModelOverride;

/// Extract a `HashMap<String, u32>` of model ID → context window from
/// a `HashMap<String, ModelOverride>`, filtering out entries without a
/// `context_window` value.
#[must_use]
pub fn extract_cw_overrides(
    overrides: &std::collections::HashMap<String, ModelOverride>,
) -> std::collections::HashMap<String, u32> {
    overrides
        .iter()
        .filter_map(|(k, v)| v.context_window.map(|cw| (k.clone(), cw)))
        .collect()
}

/// Return the known context window size (in tokens) for a model ID.
/// Falls back to 200,000 for unknown models.
#[must_use]
pub fn context_window_for_model(model_id: &str) -> u32 {
    context_window_for_model_inner(model_id)
}

/// Return the context window size for a model ID, respecting config overrides.
///
/// Precedence (highest to lowest):
/// 1. Provider-scoped config override (`provider_overrides[model_id].context_window`)
/// 2. Global config override (`global_overrides[model_id].context_window`)
/// 3. Hardcoded heuristic ([`context_window_for_model_inner`])
///
/// Both override maps use the **normalized** model ID (after
/// [`capability_model_id`] processing) as the lookup key.
///
/// Note: This function accepts `HashMap<String, u32>` (not the config crate's
/// `ModelOverride`) to keep the providers crate independent of the config crate.
/// Callers are responsible for extracting the `u32` from `ModelOverride.context_window`.
///
/// When no config is provided, this is equivalent to [`context_window_for_model`].
#[must_use]
pub fn context_window_for_model_with_config(
    model_id: &str,
    global_overrides: &std::collections::HashMap<String, u32>,
    provider_overrides: &std::collections::HashMap<String, u32>,
) -> u32 {
    let normalized = capability_model_id(model_id);

    // 1. Provider-scoped override (highest precedence)
    if let Some(&cw) = provider_overrides.get(normalized) {
        return cw;
    }
    // 2. Global override
    if let Some(&cw) = global_overrides.get(normalized) {
        return cw;
    }
    // 3. Hardcoded heuristic
    context_window_for_model_inner(model_id)
}

/// Inner heuristic — kept private so callers go through the public wrappers.
fn context_window_for_model_inner(model_id: &str) -> u32 {
    let model_id = capability_model_id(model_id);
    // Codestral has the largest window at 256k.
    if model_id.starts_with("codestral") {
        return 256_000;
    }
    // Claude models: 200k.
    if model_id.starts_with("claude-") {
        return 200_000;
    }
    // OpenAI o3/o4-mini: 200k.
    if model_id.starts_with("o3") || model_id.starts_with("o4-mini") {
        return 200_000;
    }
    // GPT-4o, GPT-4-turbo, GPT-5 series: 128k.
    if model_id.starts_with("gpt-4") || model_id.starts_with("gpt-5") {
        return 128_000;
    }
    // Mistral Large: 128k.
    if model_id.starts_with("mistral-large") {
        return 128_000;
    }
    // Gemini: 1M context.
    if model_id.starts_with("gemini-") {
        return 1_000_000;
    }
    // Kimi K2.5: 128k.
    if model_id.starts_with("kimi-") {
        return 128_000;
    }
    // MiniMax M2/M2.1/M2.5/M2.7: 204,800.
    if model_id.starts_with("MiniMax-") {
        return 204_800;
    }
    // Z.AI GLM-4-32B: 128k.
    if model_id == "glm-4-32b-0414-128k" {
        return 128_000;
    }
    // Z.AI GLM-5/4.7/4.6/4.5 series: 128k.
    if model_id.starts_with("glm-") {
        return 128_000;
    }
    // Qwen3 series (Qwen3, Qwen3-Coder): 128k.
    if model_id.starts_with("qwen3") {
        return 128_000;
    }
    // Default fallback.
    200_000
}

/// Returns `false` for model IDs that are clearly not chat-completion models
/// (image generators, TTS, speech-to-text, embeddings, moderation, etc.).
///
/// Provider APIs like OpenAI's `/v1/models` return every model in the account.
/// Since no capability metadata is exposed we filter by well-known prefixes and
/// patterns. This is applied both at discovery time and at display time so that
/// non-chat models never appear in the UI.
pub fn is_chat_capable_model(model_id: &str) -> bool {
    let id = capability_model_id(model_id);
    const NON_CHAT_PREFIXES: &[&str] = &[
        "dall-e",
        "gpt-image",
        "chatgpt-image",
        "gpt-audio",
        "tts-",
        "whisper",
        "text-embedding",
        "claude-embedding",
        "claude-embeddings",
        "omni-moderation",
        "moderation-",
        "sora",
        // Google Gemini non-chat models
        "imagen-",
        "gemini-embedding",
        "learnlm-",
        "gemma-3n-",
        // Z.AI non-chat models
        "glm-image",
        "glm-asr",
        "glm-ocr",
        "cogvideo",
        "cogview",
        "vidu",
        "autoglm-phone",
    ];
    for prefix in NON_CHAT_PREFIXES {
        if id.starts_with(prefix) {
            return false;
        }
    }
    // TTS / audio-only / realtime / transcription variants
    if id.contains("-tts") || id.contains("-audio-") || id.ends_with("-audio") {
        return false;
    }
    if id.contains("-realtime-") || id.ends_with("-realtime") {
        return false;
    }
    if id.contains("-transcribe") {
        return false;
    }
    // Gemini live (real-time dialogue) and image-generation variants
    if id.contains("-live-") || id.contains("-image-") {
        return false;
    }
    true
}

/// Check if a model supports tool/function calling.
///
/// Most modern chat models support tools, but legacy completions-only models
/// (e.g. `babbage-002`, `davinci-002`) and non-chat models do not.
/// This is checked per-model rather than per-provider so that providers
/// exposing mixed catalogs report accurate tool support.
pub fn supports_tools_for_model(model_id: &str) -> bool {
    let id = capability_model_id(model_id);
    // Legacy completions-only models — no tool support
    if id.starts_with("babbage") || id.starts_with("davinci") {
        return false;
    }
    // Non-chat model families — never support tools
    if id.starts_with("dall-e")
        || id.starts_with("gpt-image")
        || id.starts_with("tts-")
        || id.starts_with("whisper")
        || id.starts_with("text-embedding")
        || id.starts_with("claude-embedding")
        || id.starts_with("claude-embeddings")
        || id.starts_with("omni-moderation")
    {
        return false;
    }
    // Default: assume tool support for modern chat models
    true
}

/// Check if a model supports vision (image inputs).
///
/// Vision-capable models can process images in tool results and user messages.
/// When true, the runner sends images as multimodal content blocks rather than
/// stripping them from the context.
///
/// Uses a deny-list approach: most modern LLMs support vision, so unknown
/// models default to `true`. The consequence of a false positive (sending
/// images to a text-only model) is an API error — visible and diagnosable.
/// The consequence of a false negative (stripping images from a capable model)
/// is a silent failure that confuses users.
pub fn supports_vision_for_model(model_id: &str) -> bool {
    let id = capability_model_id(model_id);

    // ── Known text-only models ──────────────────────────────────────
    // Code-focused models
    if id.starts_with("codestral") {
        return false;
    }
    // Legacy OpenAI models without vision
    if id.starts_with("gpt-3.5") || id.starts_with("text-") || id.starts_with("gpt-4-") {
        // gpt-4-turbo and gpt-4-vision variants support vision
        if id.starts_with("gpt-4-turbo") || id.starts_with("gpt-4-vision") {
            return true;
        }
        return false;
    }
    // Z.AI GLM text-only models (vision variants contain 'v' suffix)
    if id.starts_with("glm-") && !id.contains('v') {
        return false;
    }

    // ── Default: assume vision support ──────────────────────────────
    true
}

/// Check if a model supports reasoning/extended thinking.
///
/// Reasoning-capable models can use the `reasoning_effort` configuration
/// to control the depth of extended thinking. This is used by the UI and
/// validation to inform users when reasoning_effort is set on a model
/// that doesn't support it.
pub fn supports_reasoning_for_model(model_id: &str) -> bool {
    let id = capability_model_id(model_id);
    // Anthropic Claude Opus 4.5+ and Sonnet 4.5+
    if id.starts_with("claude-opus-4-5")
        || id.starts_with("claude-sonnet-4-5")
        || id.starts_with("claude-opus-4-6")
        || id.starts_with("claude-sonnet-4-6")
    {
        return true;
    }
    // Claude 3.7 Sonnet supports extended thinking
    if id.starts_with("claude-3-7-sonnet") {
        return true;
    }
    // OpenAI o-series reasoning models
    if id.starts_with("o1") || id.starts_with("o3") || id.starts_with("o4") {
        return true;
    }
    // Gemini 2.5+ with thinking (2.5 Flash/Pro, 3 Flash, 3.1 Pro)
    if id.starts_with("gemini-2.5") || id.starts_with("gemini-3") {
        return true;
    }
    // OpenAI GPT-5.x models support reasoning_effort
    if id.starts_with("gpt-5") {
        return true;
    }
    // DeepSeek R1 / reasoning models
    if id.contains("deepseek-r1") || id.contains("deepseek-reasoner") {
        return true;
    }
    false
}

/// Capabilities that a model is known to support.
///
/// Populated at registration time from the pattern-matching heuristics.
/// Carried on `ModelInfo` so downstream code can check capabilities
/// without a provider instance or re-running the heuristic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct ModelCapabilities {
    /// Supports OpenAI-style function/tool calling.
    pub tools: bool,
    /// Supports image/vision inputs.
    pub vision: bool,
    /// Supports extended thinking / reasoning effort.
    pub reasoning: bool,
}

impl ModelCapabilities {
    /// Infer capabilities from the model ID using the pattern-matching heuristics.
    #[must_use]
    pub fn infer(model_id: &str) -> Self {
        Self {
            tools: supports_tools_for_model(model_id),
            vision: supports_vision_for_model(model_id),
            reasoning: supports_reasoning_for_model(model_id),
        }
    }
}

/// Info about an available model.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub display_name: String,
    /// Unix timestamp from the provider API (e.g. OpenAI `created` field).
    /// `None` for static catalog entries.
    pub created_at: Option<i64>,
    /// Flagged by the provider as a recommended/flagship model.
    pub recommended: bool,
    /// Model capabilities, resolved at registration time.
    pub capabilities: ModelCapabilities,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn context_window_for_known_models() {
        assert_eq!(
            context_window_for_model("claude-sonnet-4-20250514"),
            200_000
        );
        assert_eq!(
            context_window_for_model("claude-opus-4-5-20251101"),
            200_000
        );
        assert_eq!(context_window_for_model("gpt-4o"), 128_000);
        assert_eq!(context_window_for_model("gpt-4o-mini"), 128_000);
        assert_eq!(context_window_for_model("gpt-4-turbo"), 128_000);
        assert_eq!(context_window_for_model("o3"), 200_000);
        assert_eq!(context_window_for_model("o3-mini"), 200_000);
        assert_eq!(context_window_for_model("o4-mini"), 200_000);
        assert_eq!(context_window_for_model("codestral-latest"), 256_000);
        assert_eq!(context_window_for_model("mistral-large-latest"), 128_000);
        assert_eq!(context_window_for_model("gemini-2.0-flash"), 1_000_000);
        assert_eq!(context_window_for_model("kimi-k2.5"), 128_000);
        // Z.AI GLM models
        assert_eq!(context_window_for_model("glm-5"), 128_000);
        assert_eq!(context_window_for_model("glm-4.7"), 128_000);
        assert_eq!(context_window_for_model("glm-4.7-flash"), 128_000);
        assert_eq!(context_window_for_model("glm-4.6"), 128_000);
        assert_eq!(context_window_for_model("glm-4.5"), 128_000);
        assert_eq!(context_window_for_model("glm-4-32b-0414-128k"), 128_000);
        assert_eq!(
            context_window_for_model("custom-openrouter::openai/gpt-5.2"),
            128_000
        );
    }

    #[test]
    fn context_window_fallback_for_unknown_model() {
        assert_eq!(context_window_for_model("some-unknown-model"), 200_000);
    }

    #[test]
    fn qwen3_context_window() {
        assert_eq!(context_window_for_model("qwen3.6-plus"), 128_000);
        assert_eq!(context_window_for_model("qwen3.5-plus"), 128_000);
        assert_eq!(context_window_for_model("qwen3-max-2026-01-23"), 128_000);
        assert_eq!(context_window_for_model("qwen3-coder-next"), 128_000);
        assert_eq!(context_window_for_model("qwen3-coder-plus"), 128_000);
    }

    #[test]
    fn supports_vision_for_known_models() {
        // Claude models support vision
        assert!(supports_vision_for_model("claude-sonnet-4-20250514"));
        assert!(supports_vision_for_model("claude-opus-4-5-20251101"));
        assert!(supports_vision_for_model("claude-3-haiku-20240307"));

        // GPT-4o variants support vision
        assert!(supports_vision_for_model("gpt-4o"));
        assert!(supports_vision_for_model("gpt-4o-mini"));
        assert!(supports_vision_for_model("openrouter::openai/gpt-4o"));

        // GPT-4 turbo and vision variants support vision
        assert!(supports_vision_for_model("gpt-4-turbo"));
        assert!(supports_vision_for_model("gpt-4-vision-preview"));

        // GPT-5 supports vision
        assert!(supports_vision_for_model("gpt-5.2-codex"));

        // o3/o4 series supports vision
        assert!(supports_vision_for_model("o3"));
        assert!(supports_vision_for_model("o3-mini"));
        assert!(supports_vision_for_model("o4-mini"));

        // Gemini supports vision
        assert!(supports_vision_for_model("gemini-2.0-flash"));
        assert!(supports_vision_for_model(
            "custom-openrouter::google/gemini-2.0-flash"
        ));

        // Z.AI vision models
        assert!(supports_vision_for_model("glm-4.6v"));
        assert!(supports_vision_for_model("glm-4.6v-flash"));
        assert!(supports_vision_for_model("glm-4.5v"));

        // Mistral vision-capable models
        assert!(supports_vision_for_model("mistral-large-latest"));
        assert!(supports_vision_for_model("mistral-medium-2505"));
        assert!(supports_vision_for_model("mistral-small-latest"));
        assert!(supports_vision_for_model("pixtral-large-latest"));
        assert!(supports_vision_for_model("pixtral-12b-2409"));

        // Qwen vision models
        assert!(supports_vision_for_model("qwen-vl-max"));
        assert!(supports_vision_for_model("qwen2.5-vl-72b"));
        assert!(supports_vision_for_model("qwen3-vl-8b"));

        // Unknown models default to vision support (better to try and fail
        // with an API error than to silently strip images)
        assert!(supports_vision_for_model("some-unknown-model"));
        assert!(supports_vision_for_model("kimi-k2.5"));
    }

    #[test]
    fn supports_vision_false_for_non_vision_models() {
        // Codestral is code-focused, no vision
        assert!(!supports_vision_for_model("codestral-latest"));

        // Legacy OpenAI models without vision
        assert!(!supports_vision_for_model("gpt-3.5-turbo"));
        assert!(!supports_vision_for_model("text-davinci-003"));
        assert!(!supports_vision_for_model("gpt-4-0613"));

        // Z.AI text-only models - no vision
        assert!(!supports_vision_for_model("glm-5"));
        assert!(!supports_vision_for_model("glm-4.7"));
        assert!(!supports_vision_for_model("glm-4.5"));
    }

    #[test]
    fn is_chat_capable_filters_non_chat_models() {
        // Chat-capable models pass
        assert!(is_chat_capable_model("gpt-5.2"));
        assert!(is_chat_capable_model("gpt-4o"));
        assert!(is_chat_capable_model("o4-mini"));
        assert!(is_chat_capable_model("chatgpt-4o-latest"));

        // Non-chat models are rejected
        assert!(!is_chat_capable_model("dall-e-3"));
        assert!(!is_chat_capable_model("gpt-image-1-mini"));
        assert!(!is_chat_capable_model("chatgpt-image-latest"));
        assert!(!is_chat_capable_model("gpt-audio"));
        assert!(!is_chat_capable_model("tts-1"));
        assert!(!is_chat_capable_model("gpt-4o-mini-tts"));
        assert!(!is_chat_capable_model("gpt-4o-mini-tts-2025-12-15"));
        assert!(!is_chat_capable_model("gpt-4o-audio-preview"));
        assert!(!is_chat_capable_model("gpt-4o-realtime-preview"));
        assert!(!is_chat_capable_model("gpt-4o-mini-transcribe"));
        assert!(!is_chat_capable_model("sora"));
        assert!(!is_chat_capable_model("claude-embeddings-v1"));

        // Google Gemini non-chat models
        assert!(!is_chat_capable_model("imagen-3.0-generate-002"));
        assert!(!is_chat_capable_model("gemini-embedding-exp"));
        assert!(!is_chat_capable_model("learnlm-1.5-pro-experimental"));
        assert!(!is_chat_capable_model("gemma-3n-e4b-it"));
        // Gemma instruction-tuned models ARE chat-capable
        assert!(is_chat_capable_model("gemma-3-27b-it"));
        assert!(is_chat_capable_model("gemma-4"));
        // Gemini live/image variants are not chat models
        assert!(!is_chat_capable_model("gemini-3.1-flash-live-preview"));
        assert!(!is_chat_capable_model("gemini-3.1-flash-image-preview"));
        // Gemini chat models pass
        assert!(is_chat_capable_model("gemini-2.0-flash"));
        assert!(is_chat_capable_model("gemini-2.5-flash-preview-05-20"));
        assert!(is_chat_capable_model("gemini-3-flash-preview"));
        assert!(is_chat_capable_model("gemini-3.1-pro-preview"));
        assert!(is_chat_capable_model("gemini-3.1-flash-lite-preview"));

        // Z.AI non-chat models
        assert!(!is_chat_capable_model("glm-image"));
        assert!(!is_chat_capable_model("glm-asr-2512"));
        assert!(!is_chat_capable_model("glm-ocr"));
        assert!(!is_chat_capable_model("cogvideox-3"));
        assert!(!is_chat_capable_model("cogview-4"));
        assert!(!is_chat_capable_model("vidu"));
        assert!(!is_chat_capable_model("autoglm-phone-multilingual"));
        // Z.AI chat models pass
        assert!(is_chat_capable_model("glm-5"));
        assert!(is_chat_capable_model("glm-4.7"));
        assert!(is_chat_capable_model("glm-4.6v"));

        // Works with namespaced model IDs too
        assert!(is_chat_capable_model("openai::gpt-5.2"));
        assert!(is_chat_capable_model("custom-openrouter::openai/gpt-5.2"));
        assert!(is_chat_capable_model(
            "custom-openrouter::anthropic/claude-sonnet-4-20250514"
        ));
        assert!(!is_chat_capable_model("openai::dall-e-3"));
        assert!(!is_chat_capable_model("openai::gpt-image-1-mini"));
        assert!(!is_chat_capable_model("openai::gpt-4o-mini-tts"));
        assert!(!is_chat_capable_model(
            "custom-openrouter::openai/gpt-image-1-mini"
        ));
    }

    #[test]
    fn supports_tools_for_chat_models() {
        // Modern chat models support tools
        assert!(supports_tools_for_model("gpt-5.2"));
        assert!(supports_tools_for_model("gpt-4o"));
        assert!(supports_tools_for_model("gpt-4o-mini"));
        assert!(supports_tools_for_model("o3"));
        assert!(supports_tools_for_model("o4-mini"));
        assert!(supports_tools_for_model("chatgpt-4o-latest"));
        assert!(supports_tools_for_model("claude-sonnet-4-20250514"));
        assert!(supports_tools_for_model("gemini-2.0-flash"));
        assert!(supports_tools_for_model("codestral-latest"));
        assert!(supports_tools_for_model(
            "custom-openrouter::openai/gpt-5.2"
        ));
    }

    #[test]
    fn supports_tools_false_for_legacy_and_non_chat_models() {
        // Legacy completions-only models
        assert!(!supports_tools_for_model("babbage-002"));
        assert!(!supports_tools_for_model("davinci-002"));

        // Non-chat model families
        assert!(!supports_tools_for_model("dall-e-3"));
        assert!(!supports_tools_for_model("gpt-image-1"));
        assert!(!supports_tools_for_model("tts-1"));
        assert!(!supports_tools_for_model("tts-1-hd"));
        assert!(!supports_tools_for_model("whisper-1"));
        assert!(!supports_tools_for_model("text-embedding-3-large"));
        assert!(!supports_tools_for_model("claude-embeddings-v1"));
        assert!(!supports_tools_for_model("omni-moderation-latest"));
        assert!(!supports_tools_for_model(
            "custom-openrouter::openai/text-embedding-3-large"
        ));
    }

    #[test]
    fn supports_reasoning_for_known_models() {
        // Models that support reasoning
        assert!(supports_reasoning_for_model("claude-opus-4-5-20251101"));
        assert!(supports_reasoning_for_model("claude-sonnet-4-5-20250929"));
        assert!(supports_reasoning_for_model("claude-3-7-sonnet-20250219"));
        assert!(supports_reasoning_for_model("o3"));
        assert!(supports_reasoning_for_model("o3-mini"));
        assert!(supports_reasoning_for_model("o1"));
        assert!(supports_reasoning_for_model("o1-mini"));
        assert!(supports_reasoning_for_model("gemini-2.5-flash"));
        assert!(supports_reasoning_for_model("gemini-3-flash-preview"));
        assert!(supports_reasoning_for_model("gemini-3.1-pro-preview"));
        assert!(supports_reasoning_for_model("deepseek-r1"));
        assert!(supports_reasoning_for_model("gpt-5.4"));
        assert!(supports_reasoning_for_model("gpt-5.4-mini"));
        assert!(supports_reasoning_for_model("gpt-5"));
        assert!(supports_reasoning_for_model("gpt-5-mini"));
        assert!(supports_reasoning_for_model("gpt-5.2"));

        // Models that don't support reasoning
        assert!(!supports_reasoning_for_model("gemini-2.0-flash"));
        assert!(!supports_reasoning_for_model("claude-sonnet-4-20250514"));
        assert!(!supports_reasoning_for_model("gpt-4o"));
        assert!(!supports_reasoning_for_model("claude-3-haiku-20240307"));
    }

    #[test]
    fn supports_vision_for_all_claude_variants() {
        let claude_models = [
            "claude-3-opus-20240229",
            "claude-3-sonnet-20240229",
            "claude-3-haiku-20240307",
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
            "claude-opus-4-5-20251101",
            "claude-sonnet-4-5-20250929",
            "claude-haiku-4-5-20251001",
            "claude-3-7-sonnet-20250219",
        ];
        for model in claude_models {
            assert!(
                supports_vision_for_model(model),
                "expected {} to support vision",
                model
            );
        }
    }

    #[test]
    fn supports_vision_for_all_gpt4o_variants() {
        let gpt4o_models = [
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4o-2024-05-13",
            "gpt-4o-2024-08-06",
            "gpt-4o-audio-preview",
            "gpt-4o-mini-2024-07-18",
        ];
        for model in gpt4o_models {
            assert!(
                supports_vision_for_model(model),
                "expected {} to support vision",
                model
            );
        }
    }

    #[test]
    fn supports_vision_for_gpt5_series() {
        let gpt5_models = [
            "gpt-5",
            "gpt-5-turbo",
            "gpt-5.2-codex",
            "gpt-5.2",
            "gpt-5-preview",
        ];
        for model in gpt5_models {
            assert!(
                supports_vision_for_model(model),
                "expected {} to support vision",
                model
            );
        }
    }

    #[test]
    fn supports_vision_for_o3_o4_series() {
        let reasoning_models = ["o3", "o3-mini", "o3-preview", "o4", "o4-mini", "o4-preview"];
        for model in reasoning_models {
            assert!(
                supports_vision_for_model(model),
                "expected {} to support vision",
                model
            );
        }
    }

    #[test]
    fn supports_vision_for_gemini_variants() {
        let gemini_models = [
            "gemini-1.0-pro-vision",
            "gemini-1.5-pro",
            "gemini-1.5-flash",
            "gemini-2.0-flash",
            "gemini-2.0-pro",
            "gemini-3-flash-preview",
            "gemini-3.1-pro-preview",
            "gemini-3.1-flash-lite-preview",
            "gemini-ultra",
        ];
        for model in gemini_models {
            assert!(
                supports_vision_for_model(model),
                "expected {} to support vision",
                model
            );
        }
    }

    #[test]
    fn no_vision_for_text_only_models() {
        let text_only_models = [
            "codestral-latest",
            "gpt-3.5-turbo",
            "text-davinci-003",
            "gpt-4-0613",
            "glm-5",
            "glm-4.5",
        ];
        for model in text_only_models {
            assert!(
                !supports_vision_for_model(model),
                "expected {} to NOT support vision",
                model
            );
        }
    }

    #[test]
    fn vision_for_previously_excluded_models() {
        let now_vision = [
            "mistral-large-latest",
            "mistral-small-latest",
            "mistral-medium-2505",
            "pixtral-large-latest",
            "kimi-k2.5",
            "llama-4-scout-17b-16e-instruct",
            "MiniMax-M2.1",
            "qwen-vl-max",
            "qwen2.5-vl-72b",
            "deepseek-chat",
        ];
        for model in now_vision {
            assert!(
                supports_vision_for_model(model),
                "expected {} to support vision (default-allow)",
                model
            );
        }
    }

    #[test]
    fn vision_denylist_is_case_sensitive() {
        assert!(supports_vision_for_model("CODESTRAL-LATEST"));
        assert!(supports_vision_for_model("GPT-3.5-TURBO"));
    }

    #[test]
    fn vision_default_true_for_unknown_prefixes() {
        assert!(supports_vision_for_model("my-claude-model"));
        assert!(supports_vision_for_model("custom-gpt-4o-wrapper"));
        assert!(supports_vision_for_model("not-gemini-model"));
        assert!(supports_vision_for_model("totally-new-model-2026"));
    }

    // ── Config override tests ─────────────────────────────────────────────

    fn empty_map() -> std::collections::HashMap<String, u32> {
        std::collections::HashMap::new()
    }

    #[test]
    fn config_override_returns_heuristic_when_no_overrides() {
        // With empty overrides, should behave identically to the heuristic.
        assert_eq!(
            context_window_for_model_with_config(
                "claude-sonnet-4-20250514",
                &empty_map(),
                &empty_map()
            ),
            200_000,
        );
        assert_eq!(
            context_window_for_model_with_config("gpt-4o", &empty_map(), &empty_map()),
            128_000,
        );
        assert_eq!(
            context_window_for_model_with_config("some-unknown-model", &empty_map(), &empty_map()),
            200_000,
        );
    }

    #[test]
    fn global_override_takes_precedence_over_heuristic() {
        let mut global = empty_map();
        global.insert("claude-opus-4-6".into(), 1_000_000);
        assert_eq!(
            context_window_for_model_with_config("claude-opus-4-6", &global, &empty_map()),
            1_000_000,
        );
    }

    #[test]
    fn provider_override_takes_precedence_over_global() {
        let mut global = empty_map();
        global.insert("glm-5-turbo".into(), 150_000);
        let mut provider = empty_map();
        provider.insert("glm-5-turbo".into(), 200_000);
        assert_eq!(
            context_window_for_model_with_config("glm-5-turbo", &global, &provider),
            200_000, // provider-scoped wins
        );
    }

    #[test]
    fn config_override_uses_normalized_model_id() {
        let mut global = empty_map();
        // The override key should match the *normalized* ID
        global.insert("gpt-5.2".into(), 256_000);
        assert_eq!(
            context_window_for_model_with_config(
                "custom-openrouter::openai/gpt-5.2@reasoning-high",
                &global,
                &empty_map(),
            ),
            256_000,
        );
    }

    #[test]
    fn config_override_does_not_affect_other_models() {
        let mut global = empty_map();
        global.insert("claude-opus-4-6".into(), 1_000_000);
        // Claude Sonnet should still use heuristic
        assert_eq!(
            context_window_for_model_with_config("claude-sonnet-4-20250514", &global, &empty_map()),
            200_000,
        );
    }
}

#[cfg(test)]
mod tests_cw_overrides {
    use super::*;
    use std::collections::HashMap;

    /// Verify provider-scoped override wins over global and heuristic.
    #[test]
    fn provider_override_wins() {
        let global: HashMap<String, u32> =
            vec![("claude-sonnet-4-20250514".into(), 300_000)].into_iter().collect();
        let provider: HashMap<String, u32> =
            vec![("claude-sonnet-4-20250514".into(), 999_000)].into_iter().collect();

        let cw = context_window_for_model_with_config(
            "claude-sonnet-4-20250514",
            &global,
            &provider,
        );
        assert_eq!(cw, 999_000);
    }

    /// Verify global override wins over heuristic when no provider override.
    #[test]
    fn global_override_wins_over_heuristic() {
        let global: HashMap<String, u32> =
            vec![("claude-sonnet-4-20250514".into(), 500_000)].into_iter().collect();
        let provider: HashMap<String, u32> = HashMap::new();

        let cw = context_window_for_model_with_config(
            "claude-sonnet-4-20250514",
            &global,
            &provider,
        );
        assert_eq!(cw, 500_000);
    }

    /// Verify empty maps fall through to heuristic.
    #[test]
    fn empty_maps_use_heuristic() {
        let cw = context_window_for_model_with_config(
            "claude-sonnet-4-20250514",
            &HashMap::new(),
            &HashMap::new(),
        );
        // Heuristic for claude-* is 200_000
        assert_eq!(cw, 200_000);
    }

    /// Verify extract_cw_overrides filters out None entries.
    #[test]
    fn extract_cw_overrides_filters_none() {
        use moltis_config::schema::ModelOverride;

        let mut overrides = HashMap::new();
        overrides.insert(
            "claude-opus-4-20250514".into(),
            ModelOverride { context_window: Some(1_000_000) },
        );
        overrides.insert(
            "gpt-4o".into(),
            ModelOverride { context_window: None },
        );

        let extracted = extract_cw_overrides(&overrides);
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted.get("claude-opus-4-20250514"), Some(&1_000_000));
    }

    /// Verify extract_cw_overrides returns empty map for empty input.
    #[test]
    fn extract_cw_overrides_empty() {
        let extracted = extract_cw_overrides(&HashMap::new());
        assert!(extracted.is_empty());
    }
}

