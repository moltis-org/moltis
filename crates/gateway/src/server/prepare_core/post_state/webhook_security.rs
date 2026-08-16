use serde_json::Value;

/// Resolve the request policy for an untrusted webhook turn.
///
/// Webhooks receive no tools unless their stored configuration explicitly opts
/// into the host-owned public audience applied by the caller.
pub(super) fn request_tool_policy(
    policy: Option<&moltis_webhooks::types::ToolPolicy>,
) -> anyhow::Result<Value> {
    policy.map_or_else(
        || Ok(serde_json::json!({ "deny": ["*"] })),
        |policy| serde_json::to_value(policy).map_err(Into::into),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_policy_denies_every_tool() -> anyhow::Result<()> {
        let policy = request_tool_policy(None)?;
        assert_eq!(policy["deny"], serde_json::json!(["*"]));
        Ok(())
    }

    #[test]
    fn explicit_policy_is_preserved_for_public_audience_filtering() -> anyhow::Result<()> {
        let configured = moltis_webhooks::types::ToolPolicy {
            allow: vec!["web_search".to_string()],
            deny: Vec::new(),
        };
        let policy = request_tool_policy(Some(&configured))?;
        assert_eq!(policy["allow"], serde_json::json!(["web_search"]));
        assert!(policy.get("deny").is_none());
        Ok(())
    }
}
