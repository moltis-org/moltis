# Moltis as an ACP Agent

Moltis has long been an ACP *client*: it spawns and drives external agents such
as `codex-acp`, `claude-agent-acp`, and Cursor's `agent acp`. See
[External Agents](external-agents.md) for that direction.

`moltis acp` is the inverse. It lets any ACP *client* — [Zed](https://zed.dev),
Block's `buzz-acp`, or a bespoke harness — drive Moltis the same way it would
drive any other agent, while Moltis's own stack (sessions, memory, tool policy,
sandboxing, providers) stays behind the protocol.

## Usage

```bash
moltis acp
```

The command speaks JSON-RPC over stdin/stdout and serves exactly one client per
process, matching how every ACP harness spawns agents.

A prompt runs a real Moltis turn — the same path the Web UI takes, with your
configured providers, tools, memory, and session history — and streams the reply
back as `session/update` notifications while it runs.

### No server required

`moltis acp` boots the Moltis stack inside the spawned process and binds no
socket, so you do not start `moltis serve` first and it will not take a port.

Startup uses a headless profile. Providers, tools, memory, sessions, and
configured MCP servers are ready before the protocol starts, while channel
accounts and server-only background workers are not started.

Headless startup does not create or inject a gateway API key. There is no
gateway listener for sandboxed tools to call in this mode.

If sandbox networking is set to `trusted`, HTTP tools fail closed in ACP mode.
The trusted-domain approval proxy is not started because ACP has no domain
approval UI and the headless process does not bind listener ports.

It does open the databases under the data directory, which is what lets an ACP
session show up in the Web UI's session list. A gateway running at the same time
shares that state rather than being talked to over a socket.

### Checking a client without a model

```bash
moltis acp --echo
```

`--echo` serves a built-in agent that echoes the prompt back, without loading
providers, databases, or tools. Use it to verify a client's handshake in
isolation — if `--echo` works and a real run does not, the problem is your
Moltis configuration rather than the client.

## Why this is a surface, not a channel

ACP sits beside the Web UI and GraphQL rather than alongside Telegram or Nostr.
A channel exists to manage *external correspondents* — allowlists, sender
identities, OTP flows, per-account settings. An ACP client is the trusted local
parent process that spawned Moltis: there is no sender to gate and no account to
configure, so none of the channel contract applies. Do not expose the stdio
stream to an untrusted process. The controller can ask Moltis to use configured
tools and can provide stdio MCP servers, which is equivalent to granting that
controller the current user's privileges.

### stdout is the wire

Anything written to stdout that is not protocol framing corrupts the stream and
the client disconnects with a parse error. `moltis acp` therefore redirects all
logging to **stderr** for the lifetime of the process, including at
`--log-level trace`. If you add output to any code path this command reaches,
send it to stderr.

### Plugging into a client

Any harness that spawns an ACP agent binary can point at Moltis. For example,
`buzz-acp` spawns `$BUZZ_ACP_AGENT_COMMAND`:

```bash
BUZZ_ACP_AGENT_COMMAND=moltis BUZZ_ACP_AGENT_ARGS="acp" buzz-acp
```

Buzz is one client among several here, not a dependency — Moltis's Nostr channel
integration is a separate, unrelated feature.

## Sessions

ACP sessions map onto Moltis session keys in a dedicated `acp:` namespace, so a
`moltis acp` run cannot collide with a Web UI or channel session. The ACP
`SessionId` *is* the Moltis session key, which means a client can hand an id
straight back to `session/load` to resume a conversation.

The namespace is enforced, not merely conventional: `session/load` rejects any
id outside `acp:` with `invalid_params` before the backend is consulted, so a
client cannot name a Web UI or channel session and drive it. `session/new`
likewise refuses to return a key a backend minted outside the namespace.

The absolute `cwd` supplied by the ACP client becomes the session's Moltis
project and the working directory used by filesystem and execution tools. The
directory must exist. Loading an existing session rebinds it to the `cwd` in
that load request.

### Client-provided MCP servers

`session/new` and `session/load` accept stdio MCP servers. They are scoped to
that ACP session: their tools are added only to that session and they are shut
down when the client disconnects or replaces the session setup. HTTP and SSE
MCP transports are not advertised or accepted.

Each MCP command must be an absolute path. This makes executable selection
deterministic; it is not a sandbox. The command runs in the session `cwd` with
only the environment variables explicitly supplied in the ACP request and does
not inherit the Moltis process environment. Server names and environment
variable names must be unique within the request. Internally, client MCP tools
receive a session-specific namespace so they cannot override configured tools
or impersonate an MCP server allowed by an agent preset.

## Resource limits

The stdio surface rejects JSON-RPC frames over 4 MiB, prompts over 1 MiB or 256
blocks, and MCP setup data over 1 MiB. One connection may retain up to 64
sessions and run up to four session operations concurrently. History replay and
turn updates each have an 8 MiB output budget; persisted history reads stop at
that limit before parsing or allocating the remainder of the session file.
These limits bound accidental or malicious memory use and prevent one
controller from starting an unlimited number of simultaneous provider calls.

Protocol payload bodies are never written to Moltis logs, including at trace
level. MCP response bodies and child-process stderr are likewise omitted because
they may contain prompts, tool results, or credentials. In ACP mode, payload-
processing tracing targets are hard-filtered from both stderr and the in-memory
gateway log buffer, so a more-specific `RUST_LOG` directive cannot re-enable
them.

## Protocol support

| Method | Status |
|---|---|
| `initialize` | Supported; negotiates the version and advertises capabilities |
| `authenticate` | No-op — the client is a trusted local parent process |
| `session/new` | Supported; returns a namespaced Moltis session key |
| `session/prompt` | Supported; streams `session/update` notifications, then returns a stop reason |
| `session/cancel` | Supported; aborts the in-flight turn, resolving `prompt` with `cancelled` |
| `session/load` | Supported when the backend can resume; rejects ids outside the `acp:` namespace, then replays history before responding |
| `fs/*`, `terminal/*` | Not requested from the client |
| `session/request_permission` | Not yet routed through Moltis's tool gate |

Unknown session ids are rejected with `invalid_params`.

Only text and resource-link prompt blocks are accepted. Image, audio, and
embedded-resource blocks are rejected because those capabilities are not
advertised.

### What a turn streams

Moltis broadcasts a run's progress as the same events the Web UI renders. They
map onto `session/update` like this:

| Moltis | ACP |
|---|---|
| reply tokens | `agent_message_chunk` |
| reasoning | `agent_thought_chunk` (sent incrementally) |
| tool call started | `tool_call` with status `in_progress` |
| tool call finished | `tool_call_update` with status `completed` or `failed` |

Tool updates include capped raw arguments and output or error content. When a
turn finishes, Moltis reconciles streamed reply chunks with the authoritative
final message and emits any missing suffix before returning `end_turn`.

Web-UI affordances without an ACP equivalent — queueing, iteration counters,
voice-pending markers — are dropped rather than shown, so the client's
transcript is what the agent actually said.

> **Tool calls are reported, not gated.** A client sees tools as they run but
> cannot veto them: `session/request_permission` is not yet wired to Moltis's
> tool policy, so tool approval remains governed by your Moltis configuration,
> not by the ACP client. If you need a client to approve individual tool use,
> that is not available yet.

## Architecture notes

The protocol crate declares its traits with `#[async_trait(?Send)]`, so the
handler is pinned to the thread running a `tokio::task::LocalSet`, while
Moltis's services are `Send + Sync`. The `AcpBackend` trait in `crates/acp` is
where the two meet: implementations are `Send + Sync` and never learn a
`LocalSet` exists, and streaming flows back through a plain channel that the
protocol layer drains while the turn runs.
