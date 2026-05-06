# Hermes Feature Parity Analysis

Date: 2026-04-29
Source: "15 Hermes Agent features you've never touched" article

## Full Comparison Table

| # | Hermes Feature | How Hermes does it | Moltis Status | Verdict | Notes |
|---|---------------|-------------------|---------------|---------|-------|
| 1 | **SOUL.md / /personality** | `SOUL.md` in `~/.hermes/` loaded at boot as system prompt slot #1. `/personality` switches named personas mid-session. 14 built-in personalities (kawaii, pirate, shakespeare, etc.) plus custom in `config.yaml`. | Per-agent `SOUL.md` in workspace dirs (`crates/config/src/loader/workspace.rs`). `/mode` overlays with 9+ presets (concise, technical, creative, teacher, plan, build, review, research, elevated). | **Already done** | Moltis SOUL.md + `/mode` system is equivalent. |
| 2 | **MEMORY.md + USER.md** | Two bounded markdown files (~800 + ~500 tokens). Agent uses `memory` tool with add/replace/remove actions. FTS5 session search across all past conversations. External memory providers (Honcho, Mem0, etc.). MemoryManager orchestrates built-in + one external provider. | Full memory system in `crates/memory/`: file sync, chunking, OpenAI/local embeddings, FTS5 keyword search, hybrid vector+keyword search with reranking. Per-agent scoped memory via `AgentScopedMemoryWriter`. | **Already done** | Moltis memory system is more sophisticated (hybrid search with embeddings). |
| 3 | **/insights [days]** | `agent/insights.py` (158KB). Reads sessions table, computes token/cost/tool stats, model/platform breakdowns, bar charts. Works across CLI and messaging platforms. | Metrics infrastructure exists in `crates/metrics/` (Prometheus + JSON snapshots + SQLite time-series store). No user-facing `/insights` command. | **Built in this PR** | Added `/insights [days]` command that queries `SqliteMetricsStore` and aggregates completions, tokens, errors, tool usage, per-provider breakdowns. |
| 4 | **/snapshot** | Archive `~/.hermes/` config+state to `state-snapshots/` dir. Create/restore/prune snapshots. Excludes transient files. | No config snapshot mechanism. | **Skipped** | Low value. Moltis config is in version-controlled `moltis.toml`, state in SQLite with migrations. Git already provides this. |
| 5 | **/branch (fork)** | `/branch [name]` creates independent copy of current session. New session ID, optional name. Session DB tracks lineage. | `BranchSessionTool` in `crates/tools/src/branch_session.rs`. Fork session at a message index, new session inherits parent's model and project. | **Already done** | |
| 6 | **/rollback** | Shadow git repos in `~/.hermes/checkpoints/`. Triggered per-turn before file-mutating ops. `/rollback`, `/rollback <N>`, `/rollback diff <N>`, `/rollback <N> <file>`. 30s git timeout, 50K file limit. | `crates/tools/src/checkpoints.rs` — versioned file backups with manifest tracking, filtering by path pattern, listing with timestamps. | **Already done** | |
| 7 | **/btw** | `/background` (aliases `/bg`, `/btw`). Queues prompt in separate background session. No tools, no persistence to main conversation. Results appear as panel when done. Can run multiple concurrent background sessions. | Not implemented. | **Built in this PR** | Added `/btw <question>` — direct LLM call with last 20 messages as context, no tools, not persisted. Returns inline. |
| 8 | **/steer and /queue** | Three busy input modes: `interrupt` (default), `queue`, `steer`. `/steer` appends text to last tool result in current turn (after next tool batch). `/queue` queues message for next turn. `/busy` toggles mode. | Not implemented. | **Built in this PR** | `/steer <text>` — injects guidance via `SteerInbox` (Arc<Mutex<Vec<String>>>) polled by a background task, drained between agent loop iterations. `/queue <message>` — delegates to existing `LiveChatService` message queue. |
| 9 | **/yolo, /fast, /reasoning** | `/yolo` skips dangerous-command approvals (session-scoped). `/fast` toggles OpenAI Priority Processing / Anthropic Fast Mode. `/reasoning [level]` sets reasoning effort (none/minimal/low/medium/high/xhigh). | `ReasoningEffort` enum exists (Low/Medium/High) in `crates/config/src/schema.rs`. No `/yolo` or `/fast`. | **Partially built** | Added `/fast [on|off|status]` toggle. `/reasoning` already works via config. `/yolo` deliberately omitted — security-sensitive, needs careful design. |
| 10 | **/model --provider --global** | `/model <name>` switches session. `--global` persists to config. `--provider` switches provider. Modal picker if no args. Lists authenticated providers (up to 50 models). | `/model` command in gateway dispatch (`control_handlers.rs`). Switches model per-session. Provider selection via numbered list. | **Already done** | |
| 11 | **Auxiliary models** | Central auxiliary client router in `auxiliary_client.py`. Auto-detection fallback chains for text tasks (7-step) and vision (7-step). Per-task overrides in `config.yaml` under `auxiliary:`. Credit exhaustion fallback. | All tasks use primary model. `chat.compaction.summary_model` field exists but documented as "not wired yet" (tracked by `moltis-8me`). | **Config built in this PR** | Added `[auxiliary]` config section with `compaction`, `title_generation`, `vision` fields. Schema map + template documented. Provider resolution wiring is the next step. |
| 12 | **17-platform gateway** | 35 Python files. Telegram, Discord, Slack, WhatsApp, Signal, Email, SMS, Matrix, Mattermost, Feishu, DingTalk, WeChat, WeCom, QQ Bot, Yuanbao, Home Assistant, BlueBubbles, plus API server and webhook. | Discord (`crates/discord/`), Slack (`crates/slack/`), Matrix (`crates/matrix/`), MS Teams (`crates/msteams/`), Telegram (`crates/telegram/`), WhatsApp (`crates/whatsapp/`), Signal (`crates/signal/`), Home Assistant (`crates/home-assistant/`). Channel plugin architecture. | **Already done (keep expanding)** | 8+ platforms with unified `ChannelPlugin` trait. Adding more is incremental. |
| 13 | **/voice** | CLI push-to-talk (Ctrl+B), Telegram voice, Discord voice channels. STT via multiple backends. TTS playback via sounddevice. Termux/WSL/Docker detection. | Voice config schema in `crates/config/src/schema/voice.rs`. Multi-provider TTS (OpenAI, ElevenLabs, Google, Piper, Coqui) and STT (Whisper, Groq, Deepgram, Google, Mistral, ElevenLabs, sherpa-onnx). | **Already done** | Moltis has more provider options than Hermes. |
| 14 | **Cron + /webhook-subscriptions** | File-based cron scheduler with fcntl locking. `/cron` command for CRUD. Delivery to 18+ platforms. Dynamic webhook subscriptions in `webhook_subscriptions.json`. HMAC validation, hot-reload. No outgoing HTTP POST to arbitrary URLs. | Full cron in `crates/cron/`. Full webhook system in `crates/webhooks/` with SQLite store, worker, dedup, rate limiting, HMAC validation. Bundled skill for management. | **Already done** | Both cron and webhook subscriptions fully implemented. Neither Hermes nor Moltis has outgoing webhooks to arbitrary URLs (not needed — agent can use `web_fetch` tool). |
| 15 | **Skills as slash commands** | 102 bundled skills across 25 categories + 73 optional skills. YAML frontmatter, template substitution, inline shell blocks. Registry integration, online publishing, security scanning. `/<skill-name>` invocation. | Skills system in `crates/skills/` with discovery, YAML manifests, enable/trust state, clawHub integration. | **Already done** | |

## Summary

| Category | Count |
|----------|-------|
| Already implemented in Moltis | 10 |
| Built in this PR | 5 commands + auxiliary config |
| Deliberately skipped | 1 (/snapshot) |
| Partially done, needs follow-up | 1 (/yolo — security design needed) |

## Implementation Details (this PR)

### /btw — Ephemeral side question
- **File**: `crates/gateway/src/channel_events/commands/quick_actions.rs`
- Resolves provider from `GatewayInner.llm_providers` (same pattern as session summary)
- Reads last 20 messages from session store for context
- Single `provider.complete()` call with no tools
- Response returned inline, nothing persisted

### /fast — Fast/priority mode toggle
- **Files**: `crates/gateway/src/state.rs` (state), `quick_actions.rs` (handler)
- Session-scoped `HashSet<String>` in `GatewayInner`
- `ChatRuntime::is_fast_mode()` trait method for downstream consumption
- Broadcasts state change via WebSocket

### /insights — Session analytics
- **File**: `quick_actions.rs`, gated on `#[cfg(feature = "metrics")]`
- Queries `SqliteMetricsStore::load_history()` for the requested time window
- Computes deltas between consecutive points (metrics are cumulative counters)
- Aggregates: completions, input/output tokens, errors, tool executions, per-provider breakdown, completions/hour rate

### /steer — Mid-flight agent steering
- **Files**: `crates/agents/src/runner/mod.rs` (SteerInbox type), `streaming.rs` (drain logic), `crates/chat/src/run_with_tools.rs` (polling task), `crates/chat/src/runtime.rs` (trait method), `crates/gateway/src/chat.rs` (impl), `crates/gateway/src/state.rs` (storage)
- `SteerInbox = Arc<tokio::sync::Mutex<Vec<String>>>`
- Background task polls `ChatRuntime::take_steer_text()` every 500ms
- Agent loop drains inbox between iterations, injects as user message
- Text format: `[User steering note — adjust your approach accordingly]: {text}`

### /queue — Queue message for next turn
- **File**: `quick_actions.rs`
- Delegates to `ChatService::send()` which auto-queues when a run is active
- Reports whether message was queued or sent immediately

### [auxiliary] config — Auxiliary model assignments
- **Files**: `crates/config/src/schema/chat.rs` (AuxiliaryModelsConfig), `schema.rs` (field), `validate/schema_map.rs`, `template.rs`
- Three fields: `compaction`, `title_generation`, `vision`
- All optional, fall back to session's primary provider when unset
- Provider resolution wiring tracked by `moltis-8me`

## Follow-up Work

1. **Wire auxiliary model provider resolution** — resolve `auxiliary.compaction` from `ProviderRegistry` and pass to `compact_session()` instead of the primary provider. Same for title generation and vision tasks. Tracked by `moltis-8me`.
2. **`/yolo` toggle** — skip dangerous-command approvals. Needs security design: hardline blocklist that `/yolo` cannot bypass, session-scoped only, audit logging.
3. **Web UI for /insights** — the metrics data is already available via `MetricsSnapshot`; could add a dashboard panel.
4. **Web UI for /fast** — expose the toggle in the session settings panel.
