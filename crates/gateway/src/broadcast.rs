use std::{collections::HashMap, sync::Arc};

use {
    moltis_protocol::{EventFrame, StateVersion, scopes},
    tracing::{debug, warn},
};

use crate::state::GatewayState;

// ── Scope guards ─────────────────────────────────────────────────────────────

/// Events that require specific scopes to receive.
fn event_scope_guards() -> HashMap<&'static str, &'static [&'static str]> {
    let mut m = HashMap::new();
    m.insert("exec.approval.requested", [scopes::APPROVALS].as_slice());
    m.insert("exec.approval.resolved", [scopes::APPROVALS].as_slice());
    m.insert("device.pair.requested", [scopes::PAIRING].as_slice());
    m.insert("device.pair.resolved", [scopes::PAIRING].as_slice());
    m.insert("node.pair.requested", [scopes::PAIRING].as_slice());
    m.insert("node.pair.resolved", [scopes::PAIRING].as_slice());
    m
}

// ── Broadcast options ────────────────────────────────────────────────────────

#[derive(Default)]
pub struct BroadcastOpts {
    pub drop_if_slow: bool,
    pub state_version: Option<StateVersion>,
}

// ── Broadcaster ──────────────────────────────────────────────────────────────

/// Broadcast events to all connected WebSocket clients, respecting scope
/// guards and dropping/closing slow consumers.
pub async fn broadcast(
    state: &Arc<GatewayState>,
    event: &str,
    payload: serde_json::Value,
    opts: BroadcastOpts,
) {
    let seq = state.next_seq();
    let frame = EventFrame {
        r#type: "event".into(),
        event: event.into(),
        payload: Some(payload),
        seq: Some(seq),
        state_version: opts.state_version,
    };
    let json = match serde_json::to_string(&frame) {
        Ok(j) => j,
        Err(e) => {
            warn!("failed to serialize broadcast event: {e}");
            return;
        },
    };

    let guards = event_scope_guards();
    let required_scopes = guards.get(event);

    let inner = state.inner.read().await;
    debug!(
        event,
        seq,
        clients = inner.clients.len(),
        "broadcasting event"
    );
    for client in inner.clients.values() {
        // Check scope guard: if the event requires a scope, verify the client has it.
        if let Some(required) = required_scopes {
            let client_scopes = client.scopes();
            let has = client_scopes.contains(&scopes::ADMIN)
                || required.iter().any(|s| client_scopes.contains(s));
            if !has {
                continue;
            }
        }

        if !client.send(&json) && opts.drop_if_slow {
            // Channel full or closed — skip silently when drop_if_slow.
            continue;
        }
    }
}

/// Broadcast a tick event with the current timestamp and memory stats.
pub async fn broadcast_tick(
    state: &Arc<GatewayState>,
    process_memory_bytes: u64,
    system_available_bytes: u64,
    system_total_bytes: u64,
) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    broadcast(
        state,
        "tick",
        serde_json::json!({
            "ts": ts,
            "mem": {
                "process": process_memory_bytes,
                "available": system_available_bytes,
                "total": system_total_bytes
            }
        }),
        BroadcastOpts {
            drop_if_slow: true,
            ..Default::default()
        },
    )
    .await;
}
