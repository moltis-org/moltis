use sha2::{Digest, Sha256};

use crate::types::BridgeState;

/// Compaction scenario detected during delta sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionScenario {
    /// No compaction on either side — normal delta sync.
    None,
    /// Moltis compacted but CLI did not — CLI has more context.
    MoltisCompacted,
    /// CLI compacted but Moltis did not — need to re-send from Moltis.
    CliCompacted,
    /// Both sides compacted — use Moltis summary as authoritative.
    BothCompacted,
}

/// Result of computing the delta between Moltis session state and bridge state.
#[derive(Debug)]
pub struct DeltaSyncResult {
    pub scenario: CompactionScenario,
    pub start_index: u32,
    pub needs_context_snapshot: bool,
}

/// Compute the SHA-256 hex digest of a message's JSON representation.
#[must_use]
pub fn message_hash(message: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(message).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    hex::encode(digest)
}

/// Compute the delta sync parameters from the current bridge state and
/// the Moltis session's message array.
///
/// # Arguments
/// * `state` — persisted bridge state from the last sync
/// * `messages` — current Moltis session messages
/// * `cli_alive` — whether the external agent session is still running
#[must_use]
pub fn compute_delta(
    state: &BridgeState,
    messages: &[serde_json::Value],
    cli_alive: bool,
) -> DeltaSyncResult {
    let current_count = messages.len() as u32;

    // First turn: never synced before
    if !state.initialized {
        return DeltaSyncResult {
            scenario: CompactionScenario::None,
            start_index: 0,
            needs_context_snapshot: true,
        };
    }

    // CLI died — need full resync
    if !cli_alive {
        return DeltaSyncResult {
            scenario: CompactionScenario::CliCompacted,
            start_index: 0,
            needs_context_snapshot: true,
        };
    }

    // Check if the sync point is still valid
    let sync_idx = state.synced_message_count;

    // Moltis has fewer messages than at last sync — compaction happened
    if current_count < sync_idx {
        return DeltaSyncResult {
            scenario: CompactionScenario::MoltisCompacted,
            start_index: 0,
            needs_context_snapshot: true,
        };
    }

    // Verify the message at the sync point still matches
    if sync_idx > 0
        && let Some(msg) = messages.get((sync_idx - 1) as usize)
    {
        let hash = message_hash(msg);
        if state.last_synced_tail_hash.as_deref() != Some(&hash) {
            // Hash mismatch: Moltis compacted and rewrote history
            return DeltaSyncResult {
                scenario: CompactionScenario::MoltisCompacted,
                start_index: 0,
                needs_context_snapshot: true,
            };
        }
    }

    // Normal delta: send only new messages since last sync
    DeltaSyncResult {
        scenario: CompactionScenario::None,
        start_index: sync_idx,
        needs_context_snapshot: false,
    }
}

/// Update bridge state after a successful sync.
pub fn advance_bridge_state(
    state: &mut BridgeState,
    messages: &[serde_json::Value],
    external_session_id: Option<String>,
) {
    let count = messages.len() as u32;
    state.synced_message_count = count;
    state.last_synced_tail_hash = messages.last().map(message_hash);
    state.initialized = true;
    if external_session_id.is_some() {
        state.external_session_id = external_session_id;
    }
}

// We need hex encoding for the hash — use a minimal inline implementation
// to avoid adding a dependency just for this.
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().fold(String::new(), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::types::AgentTransportKind};

    fn make_messages(n: usize) -> Vec<serde_json::Value> {
        (0..n)
            .map(|i| serde_json::json!({ "role": "user", "content": format!("msg {i}") }))
            .collect()
    }

    #[test]
    fn first_turn_needs_full_context() {
        let state = BridgeState::new(AgentTransportKind::ClaudeCode);
        let messages = make_messages(0);
        let result = compute_delta(&state, &messages, true);
        assert!(result.needs_context_snapshot);
        assert_eq!(result.start_index, 0);
        assert_eq!(result.scenario, CompactionScenario::None);
    }

    #[test]
    fn delta_sync_after_initialization() {
        let messages = make_messages(5);
        let mut state = BridgeState::new(AgentTransportKind::ClaudeCode);
        advance_bridge_state(&mut state, &messages, None);

        // Add 2 more messages
        let mut extended = messages;
        extended.push(serde_json::json!({ "role": "assistant", "content": "reply 5" }));
        extended.push(serde_json::json!({ "role": "user", "content": "msg 6" }));

        let result = compute_delta(&state, &extended, true);
        assert!(!result.needs_context_snapshot);
        assert_eq!(result.start_index, 5);
        assert_eq!(result.scenario, CompactionScenario::None);
    }

    #[test]
    fn moltis_compaction_detected_by_count() {
        let messages = make_messages(10);
        let mut state = BridgeState::new(AgentTransportKind::ClaudeCode);
        advance_bridge_state(&mut state, &messages, None);

        // After compaction, fewer messages remain
        let compacted = make_messages(3);
        let result = compute_delta(&state, &compacted, true);
        assert!(result.needs_context_snapshot);
        assert_eq!(result.scenario, CompactionScenario::MoltisCompacted);
    }

    #[test]
    fn moltis_compaction_detected_by_hash() {
        let messages = make_messages(5);
        let mut state = BridgeState::new(AgentTransportKind::ClaudeCode);
        advance_bridge_state(&mut state, &messages, None);

        // Same count but different content at sync point (rewritten summary)
        let mut modified = make_messages(5);
        modified[4] = serde_json::json!({ "role": "system", "content": "compacted summary" });
        modified.push(serde_json::json!({ "role": "user", "content": "new" }));

        let result = compute_delta(&state, &modified, true);
        assert!(result.needs_context_snapshot);
        assert_eq!(result.scenario, CompactionScenario::MoltisCompacted);
    }

    #[test]
    fn cli_dead_triggers_resync() {
        let messages = make_messages(5);
        let mut state = BridgeState::new(AgentTransportKind::ClaudeCode);
        advance_bridge_state(&mut state, &messages, None);

        let result = compute_delta(&state, &messages, false);
        assert!(result.needs_context_snapshot);
        assert_eq!(result.scenario, CompactionScenario::CliCompacted);
    }

    #[test]
    fn advance_updates_state() {
        let messages = make_messages(3);
        let mut state = BridgeState::new(AgentTransportKind::Codex);
        assert!(!state.initialized);
        assert_eq!(state.synced_message_count, 0);

        advance_bridge_state(&mut state, &messages, Some("ext-123".into()));

        assert!(state.initialized);
        assert_eq!(state.synced_message_count, 3);
        assert!(state.last_synced_tail_hash.is_some());
        assert_eq!(state.external_session_id.as_deref(), Some("ext-123"));
    }

    #[test]
    fn message_hash_is_deterministic() {
        let msg = serde_json::json!({ "role": "user", "content": "hello" });
        let h1 = message_hash(&msg);
        let h2 = message_hash(&msg);
        assert_eq!(h1, h2);
        assert!(!h1.is_empty());
    }
}
