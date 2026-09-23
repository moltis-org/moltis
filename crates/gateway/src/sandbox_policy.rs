//! Where "must this session run sandboxed?" is answered outside the router.
//!
//! The answer comes from the agent presets **as they are on disk right now**,
//! not from the `MoltisConfig` snapshot `GatewayState` takes at construction.
//! That snapshot has no writer anywhere: nothing in the process ever replaces
//! it, and on a container deployment `ctrl update` recreates the container and
//! then provisions, so the preset files are rewritten a second after the
//! gateway starts. A snapshot reader therefore answers for a set of presets
//! that stopped being true before the first request arrived, and stays wrong
//! until the next restart.
//!
//! The run path already reloads per turn and `agents.preset.get` already reads
//! the same way, so this is the same answer those give rather than a fourth
//! one.

use moltis_config::AgentsConfig;

/// The agent presets as they are on disk right now.
///
/// Load this once per request and ask it about every entry: it reads the
/// config file and scans the agent-definition directory, which is cheap next
/// to a page render but not free once per session row.
#[must_use]
pub fn live_agents_config() -> AgentsConfig {
    moltis_config::discover_and_load_readonly().agents
}

/// Does this agent's preset force its sandbox on, per the presets on disk?
///
/// For a single question. Anything looping over sessions loads
/// [`live_agents_config`] once instead.
#[must_use]
pub fn agent_sandbox_forced(agent_id: Option<&str>) -> bool {
    live_agents_config().sandbox_forced(agent_id)
}

/// Stamp `sandbox_forced` onto a session entry object in place.
///
/// The sandbox toggle is not a control for a session whose agent sets
/// `sandbox.force`, and the UI has no other way to know: the router only
/// learns the policy on the agent's first turn, and the toggle is on screen
/// before that.
pub fn stamp_sandbox_forced(agents: &AgentsConfig, entry: &mut serde_json::Value) {
    let agent_id = entry
        .get("agent_id")
        .and_then(|value| value.as_str())
        .map(String::from);
    let forced = agents.sandbox_forced(agent_id.as_deref());
    if let Some(object) = entry.as_object_mut() {
        object.insert(
            "sandbox_forced".to_string(),
            serde_json::Value::Bool(forced),
        );
    }
}

/// Stamp every entry of a session list, loading the presets once.
pub fn stamp_sandbox_forced_list(entries: &mut serde_json::Value) {
    let Some(array) = entries.as_array_mut() else {
        return;
    };
    let agents = live_agents_config();
    for entry in array {
        stamp_sandbox_forced(&agents, entry);
    }
}
