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

                // Auto-restart: if a server was previously running and is now dead.
                // Skip restart if the server is in the middle of OAuth authentication.
                let awaiting_auth = s.auth_state == Some(moltis_mcp::McpAuthState::AwaitingBrowser);
                if prev == Some("running") && s.state == "dead" && s.enabled && !awaiting_auth {
                    let rs = restart_states
                        .entry(s.name.clone())
                        .or_insert(RestartState {
                            count: 0,
                            last_attempt: Instant::now() - MAX_BACKOFF,
                        });

                    if rs.count < MAX_RESTART_ATTEMPTS {
                        let backoff = std::cmp::min(
                            BASE_BACKOFF * 2u32.saturating_pow(rs.count),
                            MAX_BACKOFF,
                        );
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
            let backoff = std::cmp::min(BASE_BACKOFF * 2u32.saturating_pow(i), MAX_BACKOFF);
            assert!(backoff >= BASE_BACKOFF);
            assert!(backoff <= MAX_BACKOFF);
        }
    }

    #[test]
    fn test_max_backoff_cap() {
        let backoff = std::cmp::min(BASE_BACKOFF * 2u32.saturating_pow(10), MAX_BACKOFF);
        assert_eq!(backoff, MAX_BACKOFF);
    }
}
