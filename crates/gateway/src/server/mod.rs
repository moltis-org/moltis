// Server module: split from server.rs for maintainability.
//
// Domain modules:
// - location:      GatewayLocationRequester trait impl
// - helpers:        utility fns, env helpers, mem probe, diagnostics
// - startup:        OpenClaw, warmup, WebAuthn sync, Tailscale, feature stubs
// - prepared:       PreparedGatewayCore struct definition
// - prepare_core:   prepare_gateway_core entry point
// - hooks:          hook discovery, DCG guard, seeding
// - seed_content:   large const strings for seed files
// - workspace:      workspace file seeding, persona sync
// - init_channels:  channel store/registry/plugin setup
// - init_memory:    memory system / embedding provider setup

mod helpers;
mod hooks;
mod init_channels;
mod init_memory;
mod location;
mod prepare_core;
mod prepared;
mod seed_content;
mod startup;
mod workspace;

#[cfg(test)]
#[allow(dead_code, clippy::all)]
mod tests_legacy;

// ── Re-exports ───────────────────────────────────────────────────────────────
// Preserves the original public API surface of `crate::server::*`.

pub(crate) use hooks::discover_and_build_hooks;
pub use {
    helpers::approval_manager_from_config,
    prepare_core::prepare_gateway_core,
    prepared::PreparedGatewayCore,
    startup::{
        openclaw_detected_for_ui, start_browser_warmup_after_listener,
        start_openclaw_background_tasks, sync_runtime_webauthn_host_and_notice,
    },
    workspace::sync_persona_into_preset,
};

#[cfg(feature = "local-llm")]
pub use startup::local_llama_cpp_bytes_for_ui;

#[cfg(not(feature = "local-llm"))]
pub use startup::local_llama_cpp_bytes_for_ui;
