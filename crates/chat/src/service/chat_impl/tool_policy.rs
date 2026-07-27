//! Request-scoped tool restriction for `chat.send` / `chat.send_sync`.
//!
//! Callers that cannot trust the requester — the channel gateway for
//! non-operator senders, webhooks — pass a `_tool_policy` parameter. It is a
//! restriction only: it filters the shared registry for that one run and
//! composes with the configured policy layers applied later in
//! `apply_runtime_tool_filters`, which can remove more but never add back.
//!
//! Both send paths must apply it. `send()` (async, used by every channel turn)
//! previously ignored the parameter while `send_sync()` honoured it, which
//! silently disabled the restriction on exactly the untrusted path it exists
//! for.

use std::sync::Arc;

use {serde_json::Value, tokio::sync::RwLock};

use moltis_agents::tool_registry::ToolRegistry;

use moltis_tools::policy::ToolPolicy;

use crate::service::types::QueuedMessage;

/// Parse the request's `_tool_policy`, if present.
///
/// Returns `Err` on a malformed policy rather than ignoring it — a caller that
/// meant to restrict tools must not silently get an unrestricted run.
pub(crate) fn parse_request_tool_policy(params: &Value) -> Result<Option<ToolPolicy>, String> {
    params
        .get("_tool_policy")
        .cloned()
        .map(serde_json::from_value::<ToolPolicy>)
        .transpose()
        .map_err(|e| format!("invalid '_tool_policy' parameter: {e}"))
}

/// Resolve the tool registry for one run, applying the request policy when the
/// caller supplied one.
///
/// Without a policy this hands back the shared registry unchanged (no clone of
/// the tool set, no behaviour change for trusted callers such as the web UI).
pub(crate) async fn resolve_request_tool_registry(
    base: &Arc<RwLock<ToolRegistry>>,
    policy: Option<&ToolPolicy>,
) -> Arc<RwLock<ToolRegistry>> {
    let Some(policy) = policy else {
        return Arc::clone(base);
    };
    let registry = base.read().await;
    Arc::new(RwLock::new(
        registry.clone_allowed_by(|name| policy.is_allowed(name)),
    ))
}

/// Split queued messages into the leading group that shares one authorization
/// context, plus the remainder.
///
/// `MessageQueueMode::Collect` joins the text of every message in a group into
/// a single turn and runs it under one set of params. Messages from senders of
/// different privilege must therefore never share a group: a shared channel
/// session queues guest and operator messages side by side, and merging them
/// would run the guest's text under the last sender's params — an operator's
/// params carry no `_tool_policy`, so the guest's instructions would reach
/// `exec`, the owner's memory, and every other tool the guest is denied.
///
/// The caller replays the returned group and puts the remainder back on the
/// queue, where the next drain replays it under its own policy. Splitting
/// rather than merging policies keeps the rule simple: a turn never runs with
/// more privilege than the sender of any line in it.
pub(crate) fn split_by_request_tool_policy(
    mut queued: Vec<QueuedMessage>,
) -> (Vec<QueuedMessage>, Vec<QueuedMessage>) {
    let Some(head_policy) = queued
        .first()
        .map(|m| m.params.get("_tool_policy").cloned())
    else {
        return (queued, Vec::new());
    };
    let split = queued
        .iter()
        .position(|m| m.params.get("_tool_policy").cloned() != head_policy)
        .unwrap_or(queued.len());
    let rest = queued.split_off(split);
    (queued, rest)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    use {async_trait::async_trait, moltis_agents::tool_registry::AgentTool};

    struct StubTool(&'static str);

    #[async_trait]
    impl AgentTool for StubTool {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "stub"
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _args: Value) -> anyhow::Result<Value> {
            Ok(Value::Null)
        }
    }

    fn registry_with(names: &[&'static str]) -> Arc<RwLock<ToolRegistry>> {
        let mut registry = ToolRegistry::new();
        for name in names {
            registry.register(Box::new(StubTool(name)));
        }
        Arc::new(RwLock::new(registry))
    }

    #[test]
    fn absent_policy_parses_to_none() {
        let params = serde_json::json!({ "text": "hi" });
        assert!(
            parse_request_tool_policy(&params)
                .expect("absent policy is not an error")
                .is_none()
        );
    }

    #[test]
    fn deny_list_parses() {
        let params = serde_json::json!({ "_tool_policy": { "deny": ["exec", "memory_*"] } });
        let policy = parse_request_tool_policy(&params)
            .expect("valid policy")
            .expect("policy present");
        assert!(!policy.is_allowed("exec"));
        assert!(!policy.is_allowed("memory_save"));
        assert!(policy.is_allowed("web_search"));
    }

    #[test]
    fn malformed_policy_is_rejected_not_ignored() {
        let params = serde_json::json!({ "_tool_policy": "not-an-object" });
        assert!(parse_request_tool_policy(&params).is_err());
    }

    #[tokio::test]
    async fn no_policy_returns_the_shared_registry() {
        let base = registry_with(&["exec", "web_search"]);
        let resolved = resolve_request_tool_registry(&base, None).await;
        assert!(
            Arc::ptr_eq(&base, &resolved),
            "trusted callers must keep the shared registry"
        );
    }

    /// Regression guard for the bug this module was extracted to fix.
    ///
    /// `send_sync` honoured `_tool_policy` while `send` (the async path every
    /// channel turn uses) ignored it and ran the agent with the shared
    /// registry, so the guest restriction was a no-op on exactly the untrusted
    /// path it exists for. A behavioural test would need a live provider and
    /// database, so assert the structural invariant instead: neither send path
    /// may hand a run the unfiltered `self.tool_registry`.
    ///
    /// If you are refactoring and this fails, keep the invariant rather than
    /// deleting the test — every run started from a send path must receive the
    /// registry returned by `resolve_request_tool_registry`.
    #[test]
    fn both_send_paths_apply_the_request_tool_policy() {
        const SEND_ASYNC: &str = include_str!("send.rs");
        const SEND_SYNC: &str = include_str!("../chat_impl.rs");

        for (path, src) in [
            ("chat_impl/send.rs", SEND_ASYNC),
            ("chat_impl.rs", SEND_SYNC),
        ] {
            assert!(
                src.contains("resolve_request_tool_registry"),
                "{path} must resolve the request-scoped tool registry"
            );
            assert!(
                !src.contains("Arc::clone(&self.tool_registry)"),
                "{path} passes the unfiltered shared registry to a run — a caller's \
                 `_tool_policy` restriction would be silently ignored. Use the registry \
                 from resolve_request_tool_registry instead."
            );
        }
    }

    fn queued(text: &str, policy: Option<Value>) -> QueuedMessage {
        let mut params = serde_json::json!({ "text": text });
        if let Some(policy) = policy {
            params["_tool_policy"] = policy;
        }
        QueuedMessage { params }
    }

    fn guest_policy() -> Value {
        serde_json::json!({ "deny": ["exec", "memory_*"] })
    }

    fn texts(messages: &[QueuedMessage]) -> Vec<&str> {
        messages
            .iter()
            .filter_map(|m| m.params.get("text").and_then(Value::as_str))
            .collect()
    }

    #[test]
    fn same_policy_messages_stay_in_one_group() {
        let (group, rest) = split_by_request_tool_policy(vec![
            queued("a", Some(guest_policy())),
            queued("b", Some(guest_policy())),
        ]);
        assert_eq!(texts(&group), ["a", "b"]);
        assert!(rest.is_empty());
    }

    #[test]
    fn unrestricted_messages_stay_in_one_group() {
        let (group, rest) =
            split_by_request_tool_policy(vec![queued("a", None), queued("b", None)]);
        assert_eq!(texts(&group), ["a", "b"]);
        assert!(rest.is_empty());
    }

    /// The escalation this split exists to stop: Collect replay runs a merged
    /// turn under the *last* message's params, so a guest message followed by
    /// an operator message would execute the guest's text with no
    /// `_tool_policy` at all.
    #[test]
    fn guest_text_is_never_merged_into_an_operator_turn() {
        let (group, rest) = split_by_request_tool_policy(vec![
            queued("rm -rf /", Some(guest_policy())),
            queued("hi", None),
        ]);
        assert_eq!(texts(&group), ["rm -rf /"], "guest message replays alone");
        assert_eq!(texts(&rest), ["hi"], "operator message is requeued");
        assert_eq!(
            group.last().map(|m| m.params.get("_tool_policy").cloned()),
            Some(Some(guest_policy())),
            "the replayed group keeps the guest restriction"
        );
    }

    #[test]
    fn operator_group_splits_before_a_guest_message() {
        let (group, rest) = split_by_request_tool_policy(vec![
            queued("a", None),
            queued("b", None),
            queued("c", Some(guest_policy())),
        ]);
        assert_eq!(texts(&group), ["a", "b"]);
        assert_eq!(texts(&rest), ["c"]);
    }

    #[test]
    fn differing_policies_split_apart() {
        let other = serde_json::json!({ "deny": ["exec"] });
        let (group, rest) = split_by_request_tool_policy(vec![
            queued("a", Some(guest_policy())),
            queued("b", Some(other)),
        ]);
        assert_eq!(texts(&group), ["a"]);
        assert_eq!(texts(&rest), ["b"]);
    }

    #[test]
    fn empty_queue_splits_into_nothing() {
        let (group, rest) = split_by_request_tool_policy(Vec::new());
        assert!(group.is_empty());
        assert!(rest.is_empty());
    }

    #[tokio::test]
    async fn policy_filters_denied_tools_out_of_the_registry() {
        let base = registry_with(&["exec", "memory_save", "web_search"]);
        let policy = ToolPolicy {
            allow: Vec::new(),
            deny: vec!["exec".into(), "memory_*".into()],
        };

        let resolved = resolve_request_tool_registry(&base, Some(&policy)).await;
        let names = resolved.read().await.list_names();

        assert!(!names.iter().any(|n| n == "exec"));
        assert!(!names.iter().any(|n| n == "memory_save"));
        assert!(names.iter().any(|n| n == "web_search"));

        // The shared registry must be untouched — the filter is per-run.
        assert_eq!(base.read().await.list_names().len(), 3);
    }
}
