//! Tool implementations and policy enforcement.
//!
//! Tools: bash/exec, browser, canvas, message, nodes, cron, sessions,
//! web fetch/search, memory, image gen, plus channel and plugin tools.
//!
//! Policy: multi-layered allow/deny (global, per-agent, per-provider,
//! per-group, per-sender, sandbox).

pub mod approval;
pub mod auto_checkpoint;
pub mod branch_session;
pub mod browser;
pub mod calc;
pub mod checkpoints;
mod client;
#[cfg(test)]
pub mod contract;
pub mod cron_tool;
#[cfg(feature = "wasm")]
pub mod embedded_wasm;
pub mod error;
pub mod exec;
pub mod file_io;
#[cfg(feature = "firecrawl")]
pub mod firecrawl;
#[cfg(feature = "fs-tools")]
pub mod fs;
pub mod image_cache;
#[cfg(feature = "provider-openai-codex")]
pub mod image_generation;
pub mod location;
pub mod map;
pub mod nodes;
pub mod params;
pub mod policy;
pub mod process;
pub mod sandbox;
pub mod sandbox_packages;
pub mod send_document;
pub mod send_image;
pub mod session_state;
pub mod sessions_communicate;
pub mod sessions_manage;
pub mod skill_tools;
pub mod spawn_agent;
pub mod ssrf;
pub mod task_list;
#[cfg(feature = "wasm")]
pub mod wasm_component;
#[cfg(feature = "wasm")]
pub mod wasm_engine;
pub mod wasm_limits;
#[cfg(feature = "wasm")]
pub mod wasm_tool_runner;
pub mod web_fetch;
pub mod web_search;
pub mod webhook_tool;

pub use {
    client::{build_http_client, init_shared_http_client, shared_http_client},
    error::{Error, Result},
};
