# Plan: Runtime Host + Sandbox Prompt Context (Moltis)

## Goal
Give the model reliable runtime awareness so it can make correct execution decisions:
- what runs on host vs sandbox
- whether host sudo is available non-interactively
- when to ask for host installs or sandbox mode changes

## Design Principles
- Use typed Rust structs, not ad-hoc JSON, for prompt runtime context.
- Keep prompt behavior consistent between tool mode and stream-only mode.
- Minimize token overhead, especially for native tool-calling providers.
- Keep routing semantics explicit in the system prompt.

## Implementation Plan

1. Add typed runtime context in `moltis-agents` prompt layer.
- `PromptHostRuntimeContext`
- `PromptSandboxRuntimeContext`
- `PromptRuntimeContext`

2. Add runtime-aware prompt builders.
- `build_system_prompt_with_session_runtime(...)`
- `build_system_prompt_minimal_runtime(...)`
- Keep legacy builders as wrappers for backward compatibility.

3. Inject a `## Runtime` block into system prompts.
- Host facts: hostname, OS, arch, shell, provider, model, session.
- Host privilege facts: `sudo_non_interactive`, `sudo_status`.
- Sandbox facts: enabled state, mode, backend, scope, image, workspace mount, network policy, per-session override.

4. Add explicit routing instructions in tool-mode prompts.
- `exec` uses sandbox when enabled.
- host execution may require approval.
- if sandbox lacks packages/tools, ask before host install or sandbox-mode change.
- use `sudo_non_interactive` signal when deciding if host install can be self-executed.

5. Build runtime context in gateway chat flow.
- Detect shell from env.
- Detect sudo non-interactive capability via `sudo -n true`.
- Read sandbox router/session override to build effective sandbox runtime context.

6. Thread runtime context through all chat execution paths.
- Tool loop (`run_with_tools`).
- Stream-only path (`run_streaming`).
- Sync chat (`send_sync`).

7. Keep stream-only prompt parity with tool mode.
- Reuse identity/user/soul/workspace/runtime injection in minimal prompt builder.

8. Reduce prompt token usage for native tool providers.
- Keep compact `## Available Tools` list (name + short description).
- Do not duplicate full JSON tool parameter schemas in prompt when native tool-calling is already active.

9. Add targeted tests and run validation.
- Runtime context rendering tests.
- Minimal prompt behavior test.
- Compact native-tool list test.
- `cargo fmt`, targeted `cargo test`, targeted `cargo clippy`.

## Validation Commands Used
- `cargo fmt`
- `cargo test -p moltis-agents test_runtime_context_injected_when_provided`
- `cargo test -p moltis-agents test_minimal_prompt_runtime_does_not_add_exec_routing_block`
- `cargo test -p moltis-agents test_native_prompt_uses_compact_tool_list`
- `cargo clippy -p moltis-agents --tests`
- `cargo clippy -p moltis-gateway --lib`

## Known Environment Caveat
- Full workspace `cargo clippy --all --benches --tests --examples --all-features` is blocked in this environment by `llama-cpp-sys` CUDA/OpenMP toolchain requirements.
