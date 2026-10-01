//! MCP health polling and auto-restart background task.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tracing::{info, warn};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    mcp_service::LiveMcpService,
    state::GatewayState,
};

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const MAX_RESTART_ATTEMPTS: u32 = 5;
const BASE_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(300);

struct RestartState {
    count: u32,
    last_attempt: Instant,
}

fn restartable(state: &str, enabled: bool, awaiting_auth: bool) -> bool {
    enabled && state == "dead" && !awaiting_auth
}

fn restart_budget_available(count: u32) -> bool {
    count < MAX_RESTART_ATTEMPTS
}

fn restart_backoff(count: u32) -> Duration {
    std::cmp::min(BASE_BACKOFF * 2u32.saturating_pow(count), MAX_BACKOFF)
}

/// Run the health monitor loop. Checks all MCP servers periodically,
/// broadcasts status changes, and auto-restarts dead servers with backoff.
pub async fn run_health_monitor(state: Arc<GatewayState>, mcp: Arc<LiveMcpService>) {
    let mut prev_states: HashMap<String, String> = HashMap::new();
    let mut restart_states: HashMap<String, RestartState> = HashMap::new();

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        let statuses = mcp.manager().status_all().await;

        let mut changed = false;
        for s in &statuses {
            let prev = prev_states.get(&s.name).map(String::as_str);
            if prev != Some(&s.state) {
                changed = true;
            }

            // Auto-restart dead servers, including servers whose first start
            // failed. Skip interactive OAuth while a browser flow is pending.
            let awaiting_auth = s.auth_state == Some(moltis_mcp::McpAuthState::AwaitingBrowser);
            if restartable(&s.state, s.enabled, awaiting_auth) {
                let rs = restart_states
                    .entry(s.name.clone())
                    .or_insert(RestartState {
                        count: 0,
                        last_attempt: Instant::now() - MAX_BACKOFF,
                    });

                if restart_budget_available(rs.count) {
                    let backoff = restart_backoff(rs.count);
                    if rs.last_attempt.elapsed() >= backoff {
                        info!(
                            server = %s.name,
                            attempt = rs.count + 1,
                            "auto-restarting dead MCP server"
                        );
                        rs.count += 1;
                        rs.last_attempt = Instant::now();

                        match mcp.manager().restart_server(&s.name).await {
                            Ok(()) => {
                                mcp.sync_tools_if_ready().await;
                                info!(server = %s.name, "MCP server auto-restarted");
                            },
                            Err(e) => {
                                warn!(
                                    server = %s.name,
                                    error = %e,
                                    "MCP auto-restart failed"
                                );
                            },
                        }
                    }
                } else if rs.count == MAX_RESTART_ATTEMPTS {
                    warn!(
                        server = %s.name,
                        "MCP server exceeded max restart attempts, giving up"
                    );
                    rs.count += 1; // prevent repeating this warning
                }
            }

            // Reset restart counter when a server comes back to running
            if s.state == "running" {
                restart_states.remove(&s.name);
            }
            prev_states.insert(s.name.clone(), s.state.clone());
        }

        // Remove entries for servers no longer in the registry
        prev_states.retain(|name, _| statuses.iter().any(|s| &s.name == name));
        restart_states.retain(|name, _| statuses.iter().any(|s| &s.name == name));

        if changed {
            let payload = serde_json::to_value(&statuses).unwrap_or_default();
            broadcast(&state, "mcp.status", payload, BroadcastOpts::default()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_calculation() {
        // Verify backoff growth: 5, 10, 20, 40, 80 (capped at 300)
        for i in 0..MAX_RESTART_ATTEMPTS {
            let backoff = restart_backoff(i);
            assert!(backoff >= BASE_BACKOFF);
            assert!(backoff <= MAX_BACKOFF);
        }
    }

    #[test]
    fn test_max_backoff_cap() {
        let backoff = restart_backoff(10);
        assert_eq!(backoff, MAX_BACKOFF);
    }

    #[test]
    fn test_startup_failure_is_restartable() {
        assert!(restartable("dead", true, false));
    }

    #[test]
    fn test_stopped_server_is_not_restartable() {
        assert!(!restartable("stopped", true, false));
    }

    #[test]
    fn test_restart_limit() {
        assert!(restart_budget_available(MAX_RESTART_ATTEMPTS - 1));
        assert!(!restart_budget_available(MAX_RESTART_ATTEMPTS));
    }

    #[test]
    fn test_restart_backoff_schedule() {
        assert_eq!(restart_backoff(0), Duration::from_secs(5));
        assert_eq!(restart_backoff(1), Duration::from_secs(10));
        assert_eq!(restart_backoff(2), Duration::from_secs(20));
        assert_eq!(restart_backoff(3), Duration::from_secs(40));
        assert_eq!(restart_backoff(4), Duration::from_secs(80));
    }
}
