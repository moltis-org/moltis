# Instrumentation

Moltis can export what a completed agent run did — its LLM calls, tool
invocations and retrievals, with timings, token usage and errors — to an
external backend.

Three backends are supported, and they are configured together under one
`[instrumentation]` section because they are fed from a single instrumentation
pass in the agent runtime:

| Backend | What it is for |
| --- | --- |
| **Langfuse** | LLM observability: prompts, completions, sessions, token usage, inferred cost, and reaction feedback |
| **OTLP** | Any OpenTelemetry backend — Grafana Tempo/Alloy, Honeycomb, an OpenTelemetry Collector |
| **Datadog** | Datadog APM, through the Datadog Agent's OTLP intake |

> **Instrumentation is disabled by default.** Unlike every other Moltis feature
> flag, it stays off until you turn it on: enabling it sends data about your
> conversations to a third party. Read [What gets sent](#what-gets-sent) before
> switching it on.

## Backends receive different data, on purpose

This is the most important thing to understand about the design.

Langfuse is an LLM-native product. Moltis sends completed traces, conversation
content, token usage, session and user attribution, and reaction feedback so
Langfuse can provide LLM observability and infer costs.

Datadog and Grafana are infrastructure tools. Sending them prompt bodies is
actively harmful:

- span size becomes unbounded, and most vendors bill per ingested byte;
- prompt text as a span attribute is a cardinality problem in a trace index;
- it copies user conversation content into a system that was never scoped,
  reviewed, or access-controlled for it.

So Moltis applies a different **export profile** per backend:

| | Langfuse | OTLP / Datadog |
| --- | --- | --- |
| Prompts, completions, tool arguments and results | ✅ sent | ❌ **never sent** by default |
| Payload sizes (`moltis.input.bytes`) | — | ✅ sent |
| Observation type (`AGENT`, `TOOL`, `RETRIEVER`, …) | ✅ `langfuse.observation.type` | ✅ as `gen_ai.operation.name` |
| Model, latency, errors | ✅ | ✅ |
| Token counts | ✅ usage details with cache split; Langfuse infers cost | ✅ counts only |
| End-user id | ✅ | ❌ off by default (cardinality) |
| Session id | ✅ | ✅ |
| Tags | ✅ | ✅ OTLP · ❌ Datadog (billing) |

You can raise an APM's content level deliberately with
`content = "full"`, or lower it to `none`. You cannot accidentally end up
sending prompts to Datadog: the default for any newly configured OTLP-family
backend is `metadata_only`.

**For Grafana specifically:** most of what you want is already available
without this feature. Moltis exposes a Prometheus endpoint at `/metrics` with
`moltis_llm_*` counters for tokens, latency and time-to-first-token. Scrape
that for dashboards and alerts; use the OTLP exporter here when you want
per-run traces to correlate a latency spike with a specific agent run.

Moltis does not currently integrate Langfuse Prompt Management, datasets or
dataset runs, evaluators, or media uploads. User reaction scores are the only
evaluation signal sent directly to Langfuse.

## Quick start: Langfuse

Prefer the process environment for the secret key:

```sh
export MOLTIS_INSTRUMENTATION__LANGFUSE__SECRET_KEY='sk-lf-...'
```

```toml
[instrumentation]
enabled     = true
environment = "production"

[instrumentation.langfuse]
enabled    = true
host       = "https://cloud.langfuse.com"   # or your self-hosted URL
public_key = "pk-lf-..."
```

Keys come from your Langfuse project settings. For a self-hosted deployment,
point `host` at your own instance — no data leaves your network.

Once Moltis has restarted, use **Settings → Instrumentation → Test connection**
in the web UI to verify the host is reachable and the credentials are accepted
before relying on it.

## Quick start: Grafana / OpenTelemetry Collector

```toml
[instrumentation]
enabled = true

[instrumentation.otlp]
enabled  = true
endpoint = "http://localhost:4318/v1/traces"
content  = "metadata_only"   # default; "full" or "none" also accepted

[instrumentation.otlp.headers]
Authorization = "Basic ..."   # if your collector requires auth
```

## Quick start: Datadog

Run the Datadog Agent with OTLP ingest enabled and point Moltis at it. This is
the recommended setup — the Agent handles batching, retries and your API key,
so no Datadog credential needs to live in the Moltis config at all.

```toml
[instrumentation]
enabled = true

[instrumentation.datadog]
enabled  = true
endpoint = "http://localhost:4318/v1/traces"
service  = "moltis"
```

To post to Datadog's intake directly instead, set `endpoint` to the regional
OTLP intake URL and supply `api_key`.

## Where the settings live

Instrumentation is configured in `moltis.toml` under `[instrumentation]`, either
by editing the file directly or through **Settings → Configuration** in the web
UI, which edits the same TOML. Either way the change takes effect on restart.

**Settings → Instrumentation is a read-only view.** It reports what is actually
running — active backends, any that failed to start and why, delivery counters,
the capture policy in force, and whether credentials are present — and it can
test the Langfuse connection. It deliberately has no fields to edit: turning
instrumentation on sends conversation data to a third party, so that decision
lives in the config file next to the credential it needs, not behind a toggle in
a browser.

## Build features

Both exporter families are compiled in by default, so nothing here needs a
rebuild — only the config above switches them on. To leave one out of a build:

```bash
# Langfuse only (drops the generic OTLP and Datadog exporters).
cargo build --no-default-features --features "web-ui,tls,langfuse"

# No exporter at all.
cargo build --no-default-features --features "web-ui,tls"
```

`langfuse` implies `otlp`, because Langfuse ingests traces over the OTLP wire
format, and `otlp` also carries Datadog. A backend enabled in config that the
running binary was not built with is reported in **Settings → Instrumentation**
as skipped with the reason `not compiled into this build`, rather than failing
quietly.

## Running several backends at once

Backends are independent. Enabling all three sends full traces to Langfuse and
operational spans to Grafana and Datadog from the same run, each with its own
content policy. A backend that fails to start is reported in the settings UI
with a reason; the others keep running.

## What gets sent

With Langfuse enabled and default settings, each completed agent run produces:

- a trace named `agent-run` whose root observation has type **`AGENT`** and
  carries the user's message and final answer;
- a **`GENERATION`** observation for each completed LLM call, with the full
  message array sent to the provider, completion text, model, token usage split
  into fresh / cache-read / cache-write buckets, and time-to-first-token;
- a completed **`TOOL`** or **`RETRIEVER`** observation for each tool call,
  with arguments and results;
- the session key as the session id, so a Langfuse session matches a Moltis
  conversation;
- the channel sender as the user id, namespaced per channel (`telegram:42`).

Root and child observations are immutable and exported exactly once, after
completion. Moltis does not send in-progress observation updates. Cancelled or
timed-out work is closed as an error rather than exported as successful work.

You can narrow this without turning the backend off:

```toml
[instrumentation.langfuse]
capture_input    = false   # drop turn and LLM inputs
capture_output   = false   # drop turn and LLM outputs
capture_tool_io  = false   # drop tool arguments and results
```

`capture_tool_io` is separate from the others because tool arguments are the
likeliest place for a credential to appear — a `curl` command with a bearer
token, a connection string, an API key passed to a script.

### Redaction

Redaction runs before anything is serialized, in two passes:

1. **By key name** — any object key containing `password`, `secret`, `token`,
   `api_key`, `authorization`, `credential`, `private_key` and similar has its
   value replaced with `[REDACTED]`, at any nesting depth.
2. **By value shape** — string values that look like credentials are replaced
   even under an innocuous key, because tool output is unstructured. Covers
   `sk-`, `ghp_`, `github_pat_`, `xoxb-`, `AKIA`, `AIza`, `Bearer …`, PEM
   private key blocks and others.

Add your own key patterns; they extend the built-in list rather than replacing
it, so you cannot accidentally weaken the baseline:

```toml
[instrumentation]
redact = ["customer_ref", "internal_id"]
```

Redaction is a strong mitigation, not a guarantee. It cannot detect a secret
that looks like ordinary prose. If your agents routinely handle sensitive data,
prefer `capture_tool_io = false`, a self-hosted Langfuse, or both.

### Transport security

Plaintext `http://` endpoints are refused for non-loopback hosts. These
payloads carry conversation content and a credential in a header; shipping them
unencrypted across a network is not something you should be able to do by
mistyping a URL. `http://localhost` and `http://127.0.0.1` are allowed, for a
local Agent or collector.

## Sampling and volume

```toml
[instrumentation]
sample_rate       = 1.0        # fraction of turns traced, 0.0-1.0
queue_capacity    = 10000      # bounded export queue
flush_interval_ms = 5000
max_batch_bytes   = 3000000
```

Sampling is per-turn: a sampled turn is traced completely, so you never get a
trace with holes in it.

Telemetry never blocks an agent. Events go into a bounded queue and are
**dropped** if it fills, rather than slowing a user's turn. Drops are counted
and shown in the settings UI — if you see them, raise `queue_capacity` or lower
`sample_rate`.

## Configuration reference

### `[instrumentation]`

| Key | Default | Description |
| --- | --- | --- |
| `enabled` | `false` | Master switch. Off by default. |
| `environment` | `"production"` | Reported to every backend. |
| `release` | running version | Build identifier. |
| `sample_rate` | `1.0` | Fraction of turns traced. |
| `redact` | `[]` | Extra key patterns to redact. |
| `queue_capacity` | `10000` | Bounded export queue depth. |
| `flush_interval_ms` | `5000` | Maximum time an event waits before export. |
| `max_batch_bytes` | `3000000` | Forced flush threshold. |

### `[instrumentation.langfuse]`

| Key | Default | Description |
| --- | --- | --- |
| `enabled` | `false` | |
| `host` | `https://cloud.langfuse.com` | Set to your self-hosted URL to keep data on-premises. |
| `public_key` | `""` | |
| `secret_key` | unset | Prefer `MOLTIS_INSTRUMENTATION__LANGFUSE__SECRET_KEY`; a config value is also accepted and is never logged. |
| `capture_input` | `true` | |
| `capture_output` | `true` | |
| `capture_tool_io` | `true` | |
| `timeout_secs` | `10` | |

### `[instrumentation.otlp]`

| Key | Default | Description |
| --- | --- | --- |
| `enabled` | `false` | |
| `endpoint` | `""` | Full traces URL, e.g. `http://localhost:4318/v1/traces`. |
| `headers` | `{}` | Extra headers, typically auth. |
| `content` | `"metadata_only"` | `full`, `metadata_only`, or `none`. |
| `emit_user_id` | `false` | High-cardinality in an APM index. |
| `timeout_secs` | `10` | |

### `[instrumentation.datadog]`

| Key | Default | Description |
| --- | --- | --- |
| `enabled` | `false` | |
| `endpoint` | `http://localhost:4318/v1/traces` | The Datadog Agent's OTLP intake. |
| `api_key` | unset | Only needed when posting to the intake directly. |
| `service` | `"moltis"` | Datadog service name. |
| `content` | `"metadata_only"` | |
| `timeout_secs` | `10` | |

### `[instrumentation.feedback]`

| Key | Default | Description |
| --- | --- | --- |
| `enabled` | `true` | Collect reaction feedback. Gated by the master switch. |
| `positive` | `[]` | Reaction tokens counted as approval. Empty uses the built-in list. |
| `negative` | `[]` | Reaction tokens counted as disapproval. Empty uses the built-in list. |
| `link_retention_days` | `30` | How long a reply stays attributable to its turn. |

## User feedback

A thumbs up or down on a reply becomes a `user-feedback` score with Langfuse
data type `BOOLEAN`, attached to the trace that produced the reply. The Scores
API represents the value as `1.0` for positive and `0.0` for negative.

Feedback works in three places:

- **Telegram, Discord and Slack** — react to any message the bot sent.
- **The web UI** — thumbs appear in the action bar under each assistant
  message, but only while instrumentation is active and feedback is enabled.
  A control that goes nowhere is worse than no control.

Reactions accept raw emoji or shortcodes, and skin-tone and presentation
selectors are ignored when matching, so `👍`, `👍🏾`, `:+1:` and `thumbsup` are
one signal. The default vocabulary is deliberately narrow; a default that
swallowed every positive-looking emoji would turn release parties into quality
data. Override either list to change it:

```toml
[instrumentation.feedback]
positive = ["👍", "🎉", "ship-it"]
```

Setting one list leaves the other on its defaults.

**Changing your mind works.** Score ids are derived from the trace and the
reacting user. Switching from 👍 to 👎 submits the same id, so Langfuse replaces
the vote instead of recording both. Removing the reaction deletes the score
through the dedicated Scores API. Creates, replacements and deletions share one
ordered delivery queue, preventing an older queued create from recreating a
score after deletion. Two reacting users still count separately.

### Why a reaction sometimes does nothing

- **The reply is older than `link_retention_days`.** Attribution comes from a
  correlation table written when the reply is sent, and it is pruned on that
  schedule. The web UI says so explicitly rather than failing silently.
- **The reply predates instrumentation being switched on.** There was no trace
  to attribute it to.
- **The channel cannot report the ids of messages it sends.** Telegram, Discord
  and Slack can; other channels deliver the message but lose feedback
  attribution.

## Cost

Moltis exports the model id and token usage details, including separate
cache-read and cache-write buckets, but it does not compute or export a cost of
its own. Langfuse infers cost from its own versioned model definitions and
pricing tiers, which stay current without Moltis shipping a price table that
would go stale and then override them.

Unknown, private or custom models may show no cost until their pricing is
configured in Langfuse.

## How it works

Trace observations are sent to Langfuse using OTLP/HTTP JSON at
`/api/public/otel/v1/traces`. Every OTLP request includes
`x-langfuse-ingestion-version: 4`, selecting Langfuse's v4 ingestion contract.
The exporter sends only completed, immutable root and child observations.

OTLP cannot represent feedback scores. Moltis therefore creates, replaces and
deletes `BOOLEAN` feedback through Langfuse's dedicated Scores API at
`/api/public/scores`, using a separate ordered sink.

Spans carry two attribute vocabularies depending on the profile:

- `langfuse.*` — Langfuse's mapping for trace context, observation taxonomy,
  content, and detailed token usage.
- `gen_ai.*` — the OpenTelemetry GenAI semantic conventions, understood by
  Grafana, Datadog, Honeycomb and other OTel consumers.

## Troubleshooting

**No traces appear.** Check `enabled = true` at both the `[instrumentation]`
level and the backend level — the master switch gates everything. Then use
Test connection in the settings UI.

**Traces appear but token counts look wrong.** Moltis reports fresh input
tokens separately from cache reads and writes, because Langfuse prices them
differently. A run with heavy prompt caching will show a low `input` and a
large `input_cache_read`; that is correct, not a bug.

**Events are being dropped.** The export queue is full — the backend is slower
than the agent is producing events. Raise `queue_capacity`, lower
`sample_rate`, or check backend latency.

**A backend shows as skipped.** The settings UI gives the reason: missing
credentials, an empty endpoint, or a rejected plaintext URL.
