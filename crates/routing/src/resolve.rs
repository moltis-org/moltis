use {crate::Result, moltis_common::types::MsgContext};

/// Resolved route: which agent handles this message and the session key.
#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    pub agent_id: String,
    pub session_key: moltis_sessions::SessionKey,
}

/// Resolve which agent should handle a message, following the binding cascade.
pub fn resolve_agent_route(
    _msg: &MsgContext,
    _config: &serde_json::Value,
) -> Result<ResolvedRoute> {
    todo!("walk binding cascade: peer → guild → team → account → channel → default")
}
