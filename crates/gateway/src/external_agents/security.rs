use serde_json::Value;

pub(super) fn is_explicit_shell_request(params: &Value) -> bool {
    params
        .get("text")
        .or_else(|| params.get("message"))
        .and_then(Value::as_str)
        .is_some_and(|text| moltis_agents::runner::explicit_shell_command(text).is_some())
}

pub(super) fn allows_external_agent_request(params: &Value, channel_bound: bool) -> bool {
    let private_context = match params.get("_private_context") {
        None => true,
        Some(Value::Bool(value)) => *value,
        Some(_) => false,
    };
    let native_channel = params
        .get("_native_channel_request")
        .and_then(Value::as_bool)
        == Some(true);

    private_context
        && params.get("_tool_policy").is_none()
        && params.get("_tool_audience").is_none()
        && (!channel_bound || native_channel)
}

#[cfg(test)]
mod tests {
    use super::{allows_external_agent_request, is_explicit_shell_request};

    #[test]
    fn explicit_shell_requests_use_the_deterministic_runner() {
        assert!(is_explicit_shell_request(
            &serde_json::json!({"text": "/sh echo safe"})
        ));
        assert!(!is_explicit_shell_request(
            &serde_json::json!({"text": "explain shell scripting"})
        ));
    }

    #[test]
    fn rejects_public_and_tool_restricted_requests() {
        assert!(!allows_external_agent_request(
            &serde_json::json!({"_private_context": false}),
            false
        ));
        assert!(!allows_external_agent_request(
            &serde_json::json!({"_private_context": "false"}),
            false
        ));
        assert!(!allows_external_agent_request(
            &serde_json::json!({"_tool_policy": {"allow": ["calc"]}}),
            false
        ));
        assert!(!allows_external_agent_request(
            &serde_json::json!({"_tool_audience": "public"}),
            false
        ));
    }

    #[test]
    fn only_native_requests_may_use_channel_bound_external_agents() {
        assert!(!allows_external_agent_request(&serde_json::json!({}), true));
        assert!(allows_external_agent_request(
            &serde_json::json!({"_native_channel_request": true}),
            true
        ));
        assert!(allows_external_agent_request(&serde_json::json!({}), false));
    }
}
