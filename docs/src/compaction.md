# Compaction

When a chat session fills up the model's context window, Moltis compacts the
conversation so the agent can keep working. Compaction replaces (or rewrites)
older messages with a summary so the retry fits inside the context budget.

The **compaction mode** you choose controls the cost/fidelity trade-off:
faster and free, or slower and higher quality. Moltis supports four modes
out of the box.

## Why compaction matters

A long coding session can produce thousands of messages, tool calls, and
tool results. Every turn has to re-send the full history, so eventually you
hit the context window limit and the provider rejects the request. Without
compaction the session is dead — you lose the thread of work. With
compaction the agent keeps going.

The trade-off: any compaction strategy loses *some* information. The
different modes choose different things to preserve.

## The four modes

### `deterministic` (default)

**Zero LLM calls, instant, offline.** Inspects the message history directly
and builds a structured summary: message counts, tool names, file-path
mentions, recent user requests, keyword-matched pending work, and a
head-3 + tail-5 timeline. Replaces the entire history with that one summary
as a single user message.

**Strengths**
- Free (no tokens, no network).
- Deterministic output — two runs on the same history produce the same
  summary, so debugging and testing is easy.
- Works offline and on air-gapped deployments.
- Zero prompt-injection surface (the LLM never sees the raw history).

**Weaknesses**
- Can't preserve decisions or reasoning chains — it's a navigation index,
  not a narrative.
- "Pending work" relies on keyword matches (`todo`, `next`, `pending`…)
  which miss naturally-phrased follow-ups.
- Middle history is dropped entirely; only the head and tail show up in the
  timeline preview.
- Tail context is not preserved verbatim, so the LLM retry doesn't see the
  most-recent turns word-for-word.

**Pick this when** the session is a short chat channel, you're cost-sensitive,
or you're running without network access. This is the default so new
installs don't silently spend tokens on compaction.

### `recency_preserving`

**Zero LLM calls, higher fidelity.** Keeps the first few messages (system
prompt + first exchange) and the most recent ~20 % of the context window
verbatim. The middle region is collapsed into a short marker message
(`"N earlier messages were elided… Recent messages are preserved verbatim
below."`) and any bulky tool-result content in the retained slice is
replaced with a placeholder so a single multi-KB tool output can't blow
through the context window on its own.

**Strengths**
- No LLM cost, still offline-safe.
- Tail context is preserved verbatim, so the retry sees the most-recent
  turns exactly as they happened.
- Keeps reasoning chains from the middle that don't depend on tool output.
- Repairs orphaned tool-call / tool-result pairs so strict providers
  (Anthropic, OpenAI strict mode) don't reject the retry.

**Weaknesses**
- Cannot merge redundant discussions (same topic raised five times
  survives five times).
- No semantic understanding of what's important.

**Pick this when** you want a significant quality boost over `deterministic`
without paying any tokens — the best free option for most agentic coding
sessions.

### `structured`

**Head + LLM structured summary + tail.** The highest-fidelity mode. Same
head and tail boundary logic as `recency_preserving`, but the middle is
summarised with a single LLM call using a structured template:

```
## Goal
## Constraints & Preferences
## Progress
### Done / ### In Progress / ### Blocked
## Key Decisions
## Relevant Files
## Next Steps
## Critical Context
```

This is the same convention used by `hermes-agent`'s `ContextCompressor`
and `openclaw`'s compaction safeguard. Iterative re-compaction preserves
and updates prior summary sections (work moves from *In Progress* to
*Done* as it completes).

**Strengths**
- Highest fidelity — preserves reasoning, decisions, and cross-session
  context.
- Supports a cheap auxiliary summary model via `summary_model`, so you can
  run a big frontier model for coding and a small fast model for
  compaction.
- Automatic fallback to `recency_preserving` on LLM failure, so compaction
  never silently drops information.

**Weaknesses**
- Costs a summary LLM call per compaction.
- Quality depends on the summary model's instruction-following.

**Pick this when** session quality matters more than per-compaction cost —
e.g. long agentic coding sessions where losing a decision would mean
re-doing hours of work.

### `llm_replace`

**Replace entire history with a single LLM-generated summary.** The
pre-PR-#653 behaviour: stream a plain-text summary from the session's
provider, replace the history with one user message containing the
summary. No head/tail preservation.

**Strengths**
- Maximum token reduction — the retry sees one message, period.
- Works with any provider that supports streaming.

**Weaknesses**
- Loses recent turns verbatim (strictly worse than `structured` for the
  same cost).
- No structured template, so summary quality varies with the model.
- No automatic fallback — an LLM failure aborts compaction.

**Pick this when** you need the smallest possible post-compaction history
and `structured` isn't available yet (or you explicitly want the old
behaviour).

## Comparison at a glance

| Feature | `deterministic` | `recency_preserving` | `structured` | `llm_replace` |
|---|---|---|---|---|
| LLM calls | 0 | 0 | 1 | 1 |
| Token cost | none | none | medium | medium |
| Latency | ~0 ms | ~0 ms | 1–10 s | 1–10 s |
| Head preserved verbatim | ✗ | ✓ | ✓ | ✗ |
| Tail preserved verbatim | ✗ | ✓ (token-budget) | ✓ (token-budget) | ✗ |
| Middle strategy | drop | prune tool output | LLM summary | drop |
| Decisions / rationale | ✗ | partial | ✓ | partial |
| Iterative re-compaction | merge | N/A | template update | re-summarise |
| Tool-pair integrity | N/A | ✓ | ✓ | N/A |
| Fallback on LLM failure | N/A | N/A | → `recency_preserving` | abort |
| Works offline | ✓ | ✓ | ✗ | ✗ |
| Deterministic | ✓ | ✓ | ✗ | ✗ |
| Status | shipped | shipped | shipped | shipped |

## Configuration

All compaction settings live under `[chat.compaction]` in `moltis.toml`:

```toml
[chat.compaction]
mode = "deterministic"              # "deterministic" | "recency_preserving" | "structured" | "llm_replace"
threshold_percent = 0.95            # Auto-compact trigger AND tail-budget multiplier. See below.
protect_head = 3                    # Head messages kept verbatim (recency/structured).
protect_tail_min = 20               # Minimum tail messages kept verbatim (recency/structured).
tail_budget_ratio = 0.20            # Tail size as fraction of threshold_percent × context_window.
tool_prune_char_threshold = 200     # Middle tool results longer than this get placeholder-replaced.
summary_model = "openrouter/google/gemini-2.5-flash"   # RESERVED — see note below.
max_summary_tokens = 4096           # RESERVED — see note below.
show_settings_hint = true           # Show "Change chat.compaction.mode in moltis.toml…" footer.
```

### Hiding the settings hint

By default, every compaction notice (web UI compact card and channel
messages) includes a short footer pointing at the config key:

> *Change `chat.compaction.mode` in `moltis.toml` (or the web UI settings
> panel) to pick a different compaction strategy.*

Once you know the setting exists, this becomes noise. Set:

```toml
[chat.compaction]
show_settings_hint = false
```

The mode and token counts still ship with every compaction notice — only
the footer is stripped.

All fields are optional. When a field is omitted the default shown above is
used. `deterministic` mode ignores every field except `mode` and
`threshold_percent`.

### Picking a threshold

`threshold_percent` serves two related purposes:

1. **Auto-compact trigger.** When the estimated next request would
   exceed `threshold_percent × context_window` tokens, `send()` fires a
   compaction pre-emptively. On a 200 K model with the default `0.95`,
   compaction starts when the session reaches 190 K tokens. (The
   default matches the pre-PR-#653 hardcoded trigger so upgrades are
   behaviour-neutral.)
2. **Tail-budget multiplier.** For `recency_preserving` and `structured`
   modes, the size of the verbatim tail is
   `threshold_percent × tail_budget_ratio × context_window`. With the
   defaults that's 200 K × 0.95 × 0.20 = 38 K tokens of tail preserved.

Both uses move together: lowering `threshold_percent` compacts earlier
**and** shrinks the preserved tail, which is usually what you want on a
tight context window.

- Lower values (≈ 0.5) compact more aggressively and leave more headroom for
  a new burst of tool calls.
- Higher values (≈ 0.9) delay compaction as long as possible but risk
  blowing through the window on a single large tool result. The config
  validator clamps the effective value to `0.95` as an upper bound so
  auto-compact can't be accidentally disabled.

Manual `chat.compact` RPC calls ignore `threshold_percent` for the
trigger check and compact whatever's there, but still use it for the
tail-budget math inside recency-preserving and structured modes.

### Picking a summary model

> ⚠️ **`summary_model` / `max_summary_tokens` are reserved for a
> follow-up** — beads issue **moltis-8me**. They're present in the
> config schema so you can start setting them today, but the
> `structured` and `llm_replace` strategies currently **ignore** them
> and always use the session's primary provider. Setting either field
> to a non-default value triggers a one-shot runtime WARN that names
> the fields and the tracking issue so you're not billed for the wrong
> model without warning.

When the auxiliary-model subsystem lands, `summary_model` will take
a provider-qualified model identifier understood by the provider
registry (e.g. `"openrouter/google/gemini-2.5-flash"`,
`"anthropic/claude-3-5-haiku-20241022"`). Leave it unset to reuse the
session's primary model.

Small fast models are usually the right choice for compaction: they're
cheap, respond in seconds, and are good at instruction-following on a
structured template. Reserve the frontier model for the actual coding work.

## Migration notes

Upgrading from a pre-PR-#653 install? The default changed from implicit
LLM compaction to `deterministic`. If you want the old behaviour back,
set:

```toml
[chat.compaction]
mode = "llm_replace"
```

No other changes needed — `llm_replace` uses the session's primary model
just like the pre-PR behaviour did.

## Tracking issues

- Epic: **moltis-dxw** — pluggable compaction modes
- **moltis-g37** — config scaffolding, docs, `llm_replace` mode ✓
- **moltis-h0c** — `recency_preserving` mode ✓
- **moltis-aff** — `structured` mode ✓
- **moltis-8me** — auxiliary-model subsystem for cheap summary models *(follow-up, lets users route compaction to a cheap auxiliary model instead of the session's primary model)*

## Further reading

- `hermes-agent/agent/context_compressor.py` — reference implementation of
  the head + LLM summary + tail strategy that inspired `structured` mode.
- `openclaw/src/agents/compaction.ts` + `pi-hooks/compaction-safeguard.ts`
  — LLM compaction with quality auditing and tool-pair repair.
- `crates/chat/src/compaction.rs` — current `deterministic` mode
  implementation.
