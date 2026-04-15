# Handoff: Reconfigurable Context Window Limits

**Author**: Devola
**Date**: 2026-04-14
**Status**: Planning handoff — no changes made
**Target**: Coding agent to design and implement per-model reconfigurable context windows

---

## 1. Problem Statement

All model context window sizes in Moltis are determined by hardcoded prefix matching in a single Rust function. There is no configuration override. Known inaccuracies exist:

| Model Family | Moltis Hardcoded | Actual Provider Value | Gap |
|---|---|---|---|
| `glm-5`, `glm-5-turbo`, `glm-5.1` | 128,000 | ~202,752 | 74K tokens lost |
| `glm-4.7` | 128,000 | 200,000 | 72K tokens lost |
| `claude-opus-4-6` | 200,000 | 1,000,000 | 800K tokens lost |
| `gemini-2.5-pro` | 1,000,000 | 1,048,576 | ~48K close enough |
| `gemini-1.5-pro` | 1,000,000 | 2,097,152 | 1M tokens lost |

Users cannot override these values from `moltis.toml`.

---

## 2. Current Architecture

### 2.1 The Heuristic Function

**File**: `crates/providers/src/model_capabilities.rs:7-55`

```rust
pub fn context_window_for_model(model_id: &str) -> u32 {
    let model_id = capability_model_id(model_id);
    if model_id.starts_with("codestral") { return 256_000; }
    if model_id.starts_with("claude-")     { return 200_000; }
    if model_id.starts_with("o3") || model_id.starts_with("o4-mini") { return 200_000; }
    if model_id.starts_with("gpt-4") || model_id.starts_with("gpt-5") { return 128_000; }
    if model_id.starts_with("mistral-large") { return 128_000; }
    if model_id.starts_with("gemini-")    { return 1_000_000; }
    if model_id.starts_with("kimi-")      { return 128_000; }
    if model_id.starts_with("MiniMax-")   { return 204_800; }
    if model_id == "glm-4-32b-0414-128k"  { return 128_000; }
    if model_id.starts_with("glm-")       { return 128_000; }
    if model_id.starts_with("qwen3")      { return 128_000; }
    200_000 // default fallback
}
```

### 2.2 Model ID Normalization

**File**: `crates/providers/src/model_id.rs:63-69`

`capability_model_id()` strips provider namespace (`::`) and reasoning suffix (`@reasoning-high`) and takes the last path segment after `/`. Example: `custom-openrouter::openai/gpt-5.2@reasoning-high` → `gpt-5.2`.

### 2.3 The LlmProvider Trait

**File**: `crates/agents/src/model.rs:449-453`

```rust
/// Context window size in tokens for this model.
/// Used to detect when conversation approaches the limit and trigger auto-compact.
fn context_window(&self) -> u32 {
    200_000  // default
}
```

OpenAI-compatible providers override this:
**File**: `crates/providers/src/openai/provider/mod.rs:185-186`
```rust
fn context_window(&self) -> u32 {
    context_window_for_model(&self.model)
}
```

### 2.4 Provider API Metadata (Partial Override)

The OpenAI provider *does* fetch model metadata from the provider API when `model_metadata()` is called:

**File**: `crates/providers/src/openai/provider/mod.rs:219-225`
```rust
// OpenAI uses "context_window", some compat providers use "context_length".
let context_length = body
    .get("context_window")
    .or_else(|| body.get("context_length"))
    .and_then(|v| v.as_u64())
    .map(|v| v as u32)
    .unwrap_or_else(|| self.context_window()); // falls back to heuristic
```

**Result struct** (`crates/agents/src/model.rs:610-613`):
```rust
pub struct ModelMetadata {
    pub id: String,
    pub context_length: u32,
}
```

**However**: The compaction system does NOT use `model_metadata().context_length`. It calls `provider.context_window()` (the heuristic) directly.

### 2.5 Compaction Consumption

**File**: `crates/chat/src/compaction_run/mod.rs:235-255`

```rust
let context_window = if let Some(p) = provider {
    p.context_window()  // ← heuristic, not API metadata
} else {
    // fallback
};
```

### 2.6 Configuration Reference

**File**: `docs/src/configuration-reference.md`

The `chat.compaction` section documents `threshold_percent`, `protect_head`, `protect_tail_min`, `tail_budget_ratio`, `tool_prune_char_threshold`. No `context_window` override key exists.

The `tools.fs` section documents `context_window_tokens` but this only affects `Read` tool byte caps — not the actual model context window.

### 2.7 Existing Tests

**File**: `crates/providers/src/model_capabilities.rs:262-309`

Tests assert the hardcoded values. Any change to the heuristic or addition of config overrides will require updating these tests.

---

## 3. Consumers of `context_window`

These are all the places that read the context window value and would need consideration:

| Consumer | File | How it reads the value |
|---|---|---|
| Compaction trigger | `crates/chat/src/compaction_run/mod.rs` | `provider.context_window()` |
| Recency-preserving compaction | `crates/chat/src/compaction_run/recency_preserving.rs` | passed as parameter |
| Structured compaction | `crates/chat/src/compaction_run/structured.rs` | passed as parameter |
| OpenAI provider trait impl | `crates/providers/src/openai/provider/mod.rs` | calls `context_window_for_model()` |
| Anthropic provider trait impl | `crates/providers/src/anthropic.rs` | calls `context_window_for_model()` |
| Model metadata fetch | `crates/providers/src/openai/provider/mod.rs` | reads API JSON, falls back to heuristic |
| LlmProvider trait default | `crates/agents/src/model.rs` | returns `200_000` |
| Benchmarks | `crates/benchmarks/benches/boot.rs` | calls `context_window_for_model()` |

---

## 4. Design Considerations

### 4.1 Requirements

1. **User-configurable override** — allow `moltis.toml` to specify per-model context windows
2. **Backward compatible** — heuristic remains as fallback when no config is provided
3. **Per-provider or global** — support both `[models.<model_id>]` and `[providers.<name>.models.<model_id>]` scoping
4. **No runtime API dependency** — config should work even when provider metadata endpoint is unavailable
5. **Validate against provider API** — optionally log a warning when config value differs significantly from API-reported value

### 4.2 Proposed Config Shape (Strawman)

```toml
# Global model overrides
[models.claude-opus-4-6]
context_window = 1_000_000

[models.glm-5-turbo]
context_window = 200_000

[models.glm-5.1]
context_window = 200_000

# Provider-scoped override (takes precedence over global)
[providers.zai-code.models.glm-5-turbo]
context_window = 200_000
```

### 4.3 Precedence Order

1. Provider-scoped config override (`[providers.<name>.models.<id>].context_window`)
2. Global config override (`[models.<id>].context_window`)
3. Provider API metadata (`model_metadata().context_length`) — only if API is reachable
4. Hardcoded heuristic (`context_window_for_model()`)
5. Trait default (200,000)

### 4.4 Open Questions

- Should `context_window_tokens` under `[tools.fs]` be renamed/replaced to avoid confusion with the actual model context window?
- Should the compaction system prefer API metadata over the heuristic when available? (Currently it ignores API metadata entirely.)
- Should there be a CLI command or API endpoint to display the effective context window for a given model?
- How to handle model aliases — e.g., `glm-5` vs `glm-5-turbo` vs `glm-5-latest`?

---

## 5. Known Hardcoded Values (Reference)

Current hardcoded values for implementation and test updates:

| Prefix/Match | Hardcoded | Actual (known) |
|---|---|---|
| `codestral` | 256,000 | 256,000 ✅ |
| `claude-` | 200,000 | 200K (Sonnet), 1M (Opus 4.6) ⚠️ |
| `o3`, `o4-mini` | 200,000 | 200,000 ✅ |
| `gpt-4`, `gpt-5` | 128,000 | 128K (4o), 128K (4o-mini), 128K (GPT-5) ✅ |
| `mistral-large` | 128,000 | 128,000 ✅ |
| `gemini-` | 1,000,000 | 1M (Flash), 1M (2.5 Flash/Pro), 2M (1.5 Pro) ⚠️ |
| `kimi-` | 128,000 | 128,000 ✅ |
| `MiniMax-` | 204,800 | 204,800 ✅ |
| `glm-4-32b-0414-128k` | 128,000 | 128,000 ✅ |
| `glm-` (catch-all) | 128,000 | 128K (4.x), 200-203K (5.x) ⚠️ |
| `qwen3` | 128,000 | 128,000 ✅ |
| fallback | 200,000 | — |

---

## 6. Files to Modify

| File | Change |
|---|---|
| `crates/providers/src/model_capabilities.rs` | Add config-aware lookup layer; keep heuristic as fallback |
| `crates/providers/src/lib.rs` | Export any new types |
| `crates/agents/src/model.rs` | Consider adding config-aware `context_window()` on trait or provider impl |
| `crates/chat/src/compaction_run/mod.rs` | May need to accept config-aware context window |
| `crates/providers/src/openai/provider/mod.rs` | Integrate config override with API metadata fallback |
| `crates/providers/src/anthropic.rs` | Integrate config override |
| `docs/src/configuration-reference.md` | Document new config keys |
| `crates/providers/src/model_capabilities.rs` (tests) | Update tests for new behavior |
| Config loading code (find where moltis.toml is parsed) | Parse new `[models.*]` and `[providers.*.models.*]` sections |

---

## 7. Verification Plan

1. **Unit tests**: Override a model's context window via config, verify `context_window_for_model` returns the override
2. **Integration tests**: Set `[models.glm-5-turbo].context_window = 200000`, start a session, verify compaction triggers at ~190K tokens (95%)
3. **Backward compat**: No config provided → behavior identical to current heuristic
4. **API metadata interaction**: Verify precedence order when provider API returns a value and config override also exists
5. **Config validation**: Reject negative values, zero, or absurdly large values (>10M)
