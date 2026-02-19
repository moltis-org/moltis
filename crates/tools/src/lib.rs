//! Tool implementations and policy enforcement.
//!
//! Tools: bash/exec, browser, canvas, message, nodes, cron, sessions,
//! web fetch/search, memory, image gen, plus channel and plugin tools.
//!
//! Policy: multi-layered allow/deny (global, per-agent, per-provider,
//! per-group, per-sender, sandbox).

pub mod approval;
pub mod branch_session;

/// Shared HTTP client for tools that don't need custom configuration.
///
/// Reusing a single `reqwest::Client` avoids per-request connection pool,
/// DNS resolver, and TLS session cache overhead — significant on
/// memory-constrained devices.
pub fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::LazyLock<reqwest::Client> =
        std::sync::LazyLock::new(reqwest::Client::new);
    &CLIENT
}
pub mod browser;
pub mod calc;
pub mod cron_tool;
pub mod exec;
pub mod image_cache;
pub mod location;
pub mod map;
pub mod policy;
pub mod process;
pub mod sandbox;
pub mod sandbox_packages;
pub mod session_state;
pub mod skill_tools;
pub mod spawn_agent;
pub mod web_fetch;
pub mod web_search;
