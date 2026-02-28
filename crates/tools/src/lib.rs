//! Tool implementations and policy enforcement.
//!
//! Tools: bash/exec, browser, canvas, message, nodes, cron, sessions,
//! web fetch/search, memory, image gen, plus channel and plugin tools.
//!
//! Policy: multi-layered allow/deny (global, per-agent, per-provider,
//! per-group, per-sender, sandbox).

pub mod approval;
pub mod branch_session;

static SHARED_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

/// Initialize the shared HTTP client with optional proxy.
/// Call once at gateway startup; subsequent calls are no-ops.
pub fn init_shared_http_client(proxy_url: Option<&str>) {
    let _ = SHARED_CLIENT.set(build_http_client(proxy_url));
}

/// Shared HTTP client for tools that don't need custom configuration.
///
/// Reusing a single `reqwest::Client` avoids per-request connection pool,
/// DNS resolver, and TLS session cache overhead — significant on
/// memory-constrained devices.
///
/// Falls back to a plain client if [`init_shared_http_client`] was never
/// called (e.g. in tests).
pub fn shared_http_client() -> &'static reqwest::Client {
    SHARED_CLIENT.get_or_init(reqwest::Client::new)
}

/// Build a `reqwest::Client` with optional proxy configuration.
pub fn build_http_client(proxy_url: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder();
    if let Some(url) = proxy_url
        && let Ok(proxy) = reqwest::Proxy::all(url)
    {
        let proxy = proxy.no_proxy(reqwest::NoProxy::from_string("localhost,127.0.0.1,::1"));
        builder = builder.proxy(proxy);
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}
pub mod browser;
pub mod calc;
pub mod cron_tool;
#[cfg(feature = "wasm")]
pub mod embedded_wasm;
pub mod exec;
pub mod image_cache;
pub mod location;
pub mod map;
pub mod policy;
pub mod process;
pub mod sandbox;
pub mod sandbox_packages;
pub mod send_image;
pub mod session_state;
pub mod skill_tools;
pub mod spawn_agent;
pub mod ssrf;
#[cfg(feature = "wasm")]
pub mod wasm_component;
#[cfg(feature = "wasm")]
pub mod wasm_engine;
pub mod wasm_limits;
#[cfg(feature = "wasm")]
pub mod wasm_tool_runner;
pub mod web_fetch;
pub mod web_search;
