# Running Moltis in Terminal-Bench / Harbor

`moltis-ctl chat` drives a task through the same authenticated WebSocket RPC and
agent loop as the web UI. The wrapper in this directory gives Harbor a simple
installed-agent entry point.

## Build

```sh
cargo build --release -p moltis-ctl
```

Make `target/release/moltis-ctl` available in the task container (or set
`MOLTIS_CTL_BIN` to its mounted path), then configure the installed-agent
command as:

```sh
tools/terminal_bench/run_moltis_agent.sh
```

The wrapper accepts the task instruction as arguments, from stdin, or via
`HARBOR_TASK_INSTRUCTION`.

## Required environment

- `MOLTIS_API_KEY`: gateway API key (required; do not commit it)
- `MOLTIS_GATEWAY_URL`: gateway URL, default
  `http://host.docker.internal:13131`
- `MOLTIS_CTL_BIN`: `moltis-ctl` path, default `moltis-ctl`
- `MOLTIS_SESSION_KEY`: optional stable per-task session key; defaults to
  `terminal-bench:${HARBOR_TASK_ID:-task}`

Each task must use a distinct session key so model/tool history cannot leak
between benchmark tasks.

## Smoke test

A representative end-to-end task can be run without Harbor orchestration:

```sh
MOLTIS_API_KEY='...' \
MOLTIS_GATEWAY_URL='http://127.0.0.1:13131' \
tools/terminal_bench/run_moltis_agent.sh \
  'Create /tmp/terminal-bench-smoke.txt containing exactly moltis-ok'

test "$(cat /tmp/terminal-bench-smoke.txt)" = moltis-ok
```

The `chat` command does not return until the Moltis agent loop finishes. Its
JSON output includes the assistant text and token/round usage. Use
`moltis-ctl chat-history --session-key <key>` when debugging a failed run.
