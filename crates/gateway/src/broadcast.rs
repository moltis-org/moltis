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
    /// Stream group ID for chunked delivery (v4).
    pub stream: Option<String>,
    /// End-of-stream marker (v4).
    pub done: bool,
    /// Logical channel for multiplexing (v4).
    pub channel: Option<String>,
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
    let stream = opts.stream.clone();
    let done = opts.done.then_some(true);
    let channel = opts.channel.clone();
    let frame = EventFrame {
        r#type: "event".into(),
        event: event.into(),
        payload: Some(payload),
        seq: Some(seq),
        state_version: opts.state_version,
        stream,
        done,
        channel,
    };
    let json = match serde_json::to_string(&frame) {
        Ok(j) => j,
        Err(e) => {
            warn!("failed to serialize broadcast event: {e}");
            return;
        },
    };

    // Forward to GraphQL subscription broadcast channel.
    #[cfg(feature = "graphql")]
    if let Some(ref payload) = frame.payload {
        let _ = state
            .graphql_broadcast
            .send((event.to_string(), payload.clone()));
    }

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

        // Subscription filter (v4): skip clients not subscribed to this event.
        if !client.is_subscribed_to(event) {
            continue;
        }

        // Channel filter (v4): if event is scoped to a channel, skip clients
        // that haven't joined it.
        if let Some(ref ch) = opts.channel
            && !client.is_in_channel(ch)
        {
            continue;
        }

        if !client.send(&json) && opts.drop_if_slow {
            // Channel full or closed — skip silently when drop_if_slow.
            continue;
        }
    }
}

/// Broadcast a tick event with the current timestamp and memory stats.
fn tick_mem_payload(
    process_memory_bytes: u64,
    local_llama_cpp_bytes: u64,
    system_available_bytes: u64,
    system_total_bytes: u64,
) -> serde_json::Value {
    let mut mem = serde_json::Map::new();
    mem.insert(
        "process".to_string(),
        serde_json::json!(process_memory_bytes),
    );
    mem.insert(
        "available".to_string(),
        serde_json::json!(system_available_bytes),
    );
    mem.insert("total".to_string(), serde_json::json!(system_total_bytes));
    if local_llama_cpp_bytes > 0 {
        mem.insert(
            "localLlamaCpp".to_string(),
            serde_json::json!(local_llama_cpp_bytes),
        );
    }
    serde_json::Value::Object(mem)
}

pub async fn broadcast_tick(
    state: &Arc<GatewayState>,
    process_memory_bytes: u64,
    local_llama_cpp_bytes: u64,
    system_available_bytes: u64,
    system_total_bytes: u64,
) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mem = tick_mem_payload(
        process_memory_bytes,
        local_llama_cpp_bytes,
        system_available_bytes,
        system_total_bytes,
    );

    broadcast(
        state,
        "tick",
        serde_json::json!({
            "ts": ts,
            "mem": mem
        }),
        BroadcastOpts {
            drop_if_slow: true,
            ..Default::default()
        },
    )
    .await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::tick_mem_payload;

    #[test]
    fn tick_mem_payload_omits_llama_cpp_when_zero() {
        let payload = tick_mem_payload(1, 0, 2, 3);
        assert_eq!(payload.get("process").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(payload.get("available").and_then(|v| v.as_u64()), Some(2));
        assert_eq!(payload.get("total").and_then(|v| v.as_u64()), Some(3));
        assert!(payload.get("localLlamaCpp").is_none());
    }

    #[test]
    fn tick_mem_payload_includes_llama_cpp_when_positive() {
        let payload = tick_mem_payload(1, 4, 2, 3);
        assert_eq!(
            payload.get("localLlamaCpp").and_then(|v| v.as_u64()),
            Some(4)
        );
    }
}
