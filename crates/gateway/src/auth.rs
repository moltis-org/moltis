pub use moltis_auth::*;

/// Generate a random 8-character alphanumeric setup code (~48 bits of entropy).
///
/// Uses uppercase + digits only (no ambiguous chars like 0/O, 1/I/L) for easy
/// reading from a terminal.
pub fn generate_setup_code() -> String {
    use rand::RngExt;
    const CHARSET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut rng = rand::rng();
    (0..8)
        .map(|_| CHARSET[rng.random_range(0..CHARSET.len())] as char)
        .collect()
}

/// Scopes minted for the sandbox's `sandbox-ctl` API key.
///
/// Read-only, and deliberately so. This key is injected as `MOLTIS_API_KEY`
/// into every sandboxed exec, next to a `MOLTIS_GATEWAY_URL` that reaches the
/// gateway back through `host.docker.internal`, so whatever the key can do a
/// prompt-injected agent inside that sandbox can do. With `operator.write` on
/// it the agent could call `agents.preset.update` against its own preset, set
/// `sandbox.mounts` to `/:/host:rw` or the docker socket, and collect that
/// mount on its next turn, when the fingerprint change recreates the
/// container. The sandbox would be writing its own escape hatch.
///
/// Nothing needs the write half. `moltis-ctl` is the only consumer of this
/// key, and the in-process `mcp_*` agent tools exist precisely so an agent
/// does not have to shell out to `moltis-ctl` at all.
pub const SANDBOX_API_KEY_SCOPES: &[&str] = &[moltis_protocol::scopes::READ];
