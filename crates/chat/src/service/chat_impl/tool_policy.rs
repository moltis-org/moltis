//! Request-scoped tool restriction for `chat.send` / `chat.send_sync`.
//!
//! Callers that cannot trust the requester — the channel gateway and webhooks
//! — pass `_tool_audience = "public"`. The registry filters by host-owned tool
//! metadata before applying an optional `_tool_policy` name filter. Both are
//! restrictions only and compose with configured policy layers applied later
//! in `apply_runtime_tool_filters`, which can remove more but never add back.
//!
//! Both send paths must apply it. `send()` (async, used by every channel turn)
//! previously ignored the parameter while `send_sync()` honoured it, which
//! silently disabled the restriction on exactly the untrusted path it exists
//! for.

use std::sync::Arc;

use {serde_json::Value, tokio::sync::RwLock};

use moltis_agents::tool_registry::{ToolAudience, ToolRegistry};

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

/// Parse the host-owned audience ceiling for this request.
///
/// Trusted requests omit the marker. Callers may only opt into the more
/// restrictive public audience; any other value is rejected rather than
/// risking a fail-open interpretation.
pub(crate) fn parse_request_tool_audience(params: &Value) -> Result<ToolAudience, String> {
    match params.get("_tool_audience") {
        None => Ok(ToolAudience::Trusted),
        Some(Value::String(value)) if value == "public" => Ok(ToolAudience::Public),
        Some(_) => Err("invalid '_tool_audience' parameter: expected 'public'".to_string()),
    }
}

/// Apply the fail-closed context used for delayed or channel-bound requests
/// that cannot retain caller authorization.
pub(crate) fn apply_untrusted_request_context(params: &mut Value) {
    params["_tool_audience"] = Value::String("public".to_string());
    params["_tool_policy"] = serde_json::json!({ "deny": ["*"] });
    params["_private_context"] = Value::Bool(false);
}

/// Downgrade a privileged channel turn before delayed replay, when its sender's
/// authorization may no longer be valid.
#[must_use]
pub(crate) fn downgrade_queued_channel_request(params: &mut Value, private_context: bool) -> bool {
    if !private_context || !params.get("channel").is_some_and(Value::is_object) {
        return false;
    }
    apply_untrusted_request_context(params);
    true
}

/// Resolve the tool registry for one run, applying its audience ceiling and
/// optional name policy.
///
/// Without a policy this hands back the shared registry unchanged (no clone of
/// the tool set, no behaviour change for trusted callers such as the web UI).
pub(crate) async fn resolve_request_tool_registry(
    base: &Arc<RwLock<ToolRegistry>>,
    policy: Option<&ToolPolicy>,
    audience: ToolAudience,
) -> Arc<RwLock<ToolRegistry>> {
    if policy.is_none() && audience == ToolAudience::Trusted {
        return Arc::clone(base);
    }
    let registry = base.read().await;
    let audience_registry = registry.clone_for_audience(audience);
    let filtered = match policy {
        Some(policy) => audience_registry.clone_allowed_by(|name| policy.is_allowed(name)),
        None => audience_registry,
    };
    Arc::new(RwLock::new(filtered))
}

/// Whether this request may receive owner-private prompt and memory context.
pub(crate) fn allows_private_context(params: &Value) -> bool {
    if params.get("_tool_audience").and_then(Value::as_str) == Some("public") {
        return false;
    }
    match params.get("_private_context") {
        None => true,
        Some(Value::Bool(value)) => *value,
        Some(_) => false,
    }
}

fn request_security_context(
    params: &Value,
) -> (
    Option<Value>,
    Option<Value>,
    bool,
    Option<Value>,
    Option<Value>,
) {
    let reply_scope = params.get("_channel_reply_target").map(|target| {
        serde_json::json!({
            "channel_type": target.get("channel_type"),
            "account_id": target.get("account_id"),
            "chat_id": target.get("chat_id"),
            "thread_id": target.get("thread_id"),
            "message_id": target.get("message_id"),
        })
    });
    (
        params.get("_tool_audience").cloned(),
        params.get("_tool_policy").cloned(),
        allows_private_context(params),
        params
            .get("channel")
            .and_then(|channel| channel.get("sender_id"))
            .cloned(),
        reply_scope,
    )
}

/// Split queued messages into the leading group that shares one authorization
/// context, plus the remainder.
///
/// `MessageQueueMode::Collect` joins the text of every message in a group into
/// a single turn and runs it under one set of params. Messages from senders of
/// different principals or privilege must therefore never share a group: a
/// shared channel session queues multiple senders side by side, and merging
/// them would attribute all text to the final sender and its policy.
///
/// The caller replays the returned group and puts the remainder back on the
/// queue, where the next drain replays it under its own policy. Splitting
/// rather than merging policies keeps the rule simple: a turn never runs with
/// more privilege than the sender of any line in it.
pub(crate) fn split_by_request_security_context(
    mut queued: Vec<QueuedMessage>,
) -> (Vec<QueuedMessage>, Vec<QueuedMessage>) {
    let Some(head_context) = queued
        .first()
        .map(|message| request_security_context(&message.params))
    else {
        return (queued, Vec::new());
    };
    let split = queued
        .iter()
        .position(|message| request_security_context(&message.params) != head_context)
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

    #[test]
    fn audience_defaults_to_trusted_and_only_accepts_public_restriction() {
        assert_eq!(
            parse_request_tool_audience(&serde_json::json!({})),
            Ok(ToolAudience::Trusted)
        );
        assert_eq!(
            parse_request_tool_audience(&serde_json::json!({"_tool_audience": "public"})),
            Ok(ToolAudience::Public)
        );
        assert!(
            parse_request_tool_audience(&serde_json::json!({"_tool_audience": "trusted"})).is_err()
        );
        assert!(parse_request_tool_audience(&serde_json::json!({"_tool_audience": true})).is_err());
    }

    #[test]
    fn private_context_defaults_on_and_can_be_disabled() {
        assert!(allows_private_context(&serde_json::json!({})));
        assert!(!allows_private_context(
            &serde_json::json!({"_private_context": false})
        ));
        assert!(!allows_private_context(
            &serde_json::json!({"_private_context": "false"})
        ));
        assert!(!allows_private_context(
            &serde_json::json!({"_private_context": null})
        ));
        assert!(!allows_private_context(
            &serde_json::json!({"_tool_audience": "public", "_private_context": true})
        ));
    }

    #[test]
    fn untrusted_request_context_denies_all_tools() {
        let mut params = serde_json::json!({
            "_tool_policy": {"allow": ["*"]},
            "_private_context": true,
        });

        apply_untrusted_request_context(&mut params);

        assert_eq!(params["_tool_audience"], "public");
        assert_eq!(params["_tool_policy"]["deny"], serde_json::json!(["*"]));
        assert_eq!(params["_private_context"], false);
    }

    #[test]
    fn queued_channel_downgrade_removes_tools_and_private_context() {
        let mut params = serde_json::json!({
            "channel": {"sender_id": "owner"},
            "_private_context": true,
        });

        assert!(downgrade_queued_channel_request(&mut params, true));
        assert_eq!(params["_tool_audience"], "public");
        assert_eq!(params["_tool_policy"]["deny"], serde_json::json!(["*"]));
        assert_eq!(params["_private_context"], false);

        assert!(!downgrade_queued_channel_request(
            &mut serde_json::json!({}),
            true
        ));
    }

    #[tokio::test]
    async fn no_policy_returns_the_shared_registry() {
        let base = registry_with(&["exec", "web_search"]);
        let resolved = resolve_request_tool_registry(&base, None, ToolAudience::Trusted).await;
        assert!(
            Arc::ptr_eq(&base, &resolved),
            "trusted callers must keep the shared registry"
        );
    }

    /// Regression guard for the bug this module was extracted to fix.
    ///
    /// `send_sync` once honoured `_tool_policy` while `send` ignored it and ran
    /// the agent with the shared registry, so the guest restriction was a no-op
    /// on exactly the untrusted path it exists for. Both trait methods now route
    /// through `send_impl`; keep request security in that shared path.
    ///
    /// If you are refactoring and this fails, keep the invariant rather than
    /// deleting the test — every run started from a send path must receive the
    /// registry returned by `resolve_request_tool_registry`.
    #[test]
    fn both_send_paths_apply_request_tool_security() {
        const SEND_IMPL: &str = include_str!("send.rs");
        const CHAT_IMPL: &str = include_str!("../chat_impl.rs");

        assert!(
            CHAT_IMPL.matches("self.send_impl(").count() >= 2,
            "both ChatService send methods must delegate to send_impl"
        );
        assert!(
            SEND_IMPL.contains("resolve_request_tool_registry"),
            "send_impl must resolve the request-scoped tool registry"
        );
        assert!(
            SEND_IMPL.contains("parse_request_tool_audience"),
            "send_impl must parse the request tool audience"
        );
        assert!(
            !SEND_IMPL.contains("Arc::clone(&self.tool_registry)"),
            "send_impl passes the unfiltered shared registry to a run; use the registry \
             from resolve_request_tool_registry instead"
        );
    }

    /// The gateway skips the untrusted tool ceiling for an operator direct-chat
    /// turn it recognizes as an explicit `/sh`, on the promise that the chat
    /// layer will then run it through the deterministic shell path instead of
    /// the agent loop. If the two disagree, forms only the gateway recognizes —
    /// `/sh@bot ls`, `/SH ls` — arrive here as ordinary prose *and* keep the
    /// unrestricted registry and private context, which is the opposite of what
    /// the ceiling exists for.
    ///
    /// Keep one predicate. Do not reintroduce a local variant.
    #[test]
    fn send_path_detects_shell_commands_exactly_as_the_gateway_does() {
        const SEND_ASYNC: &str = include_str!("send.rs");

        assert!(
            SEND_ASYNC.contains("moltis_agents::runner::explicit_shell_command"),
            "send.rs must detect `/sh` with the same predicate the gateway \
             authorizes on and the runner executes on"
        );

        // Forms the gateway treats as `/sh`. Each must be recognized here too,
        // or it bypasses the ceiling without taking the shell path.
        for input in ["/sh ls", "  /sh ls", "/SH ls", "/sh@mybot ls"] {
            assert!(
                moltis_agents::runner::explicit_shell_command(input).is_some(),
                "{input} must be recognized as an explicit shell command"
            );
        }
        for input in ["explain what /sh does", "/shell ls", "/sh", "/sh   "] {
            assert!(
                moltis_agents::runner::explicit_shell_command(input).is_none(),
                "{input} must not be treated as an explicit shell command"
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

    fn public_queued(text: &str, policy: Option<Value>) -> QueuedMessage {
        let mut message = queued(text, policy);
        message.params["_tool_audience"] = serde_json::json!("public");
        message
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
        let (group, rest) = split_by_request_security_context(vec![
            queued("a", Some(guest_policy())),
            queued("b", Some(guest_policy())),
        ]);
        assert_eq!(texts(&group), ["a", "b"]);
        assert!(rest.is_empty());
    }

    #[test]
    fn unrestricted_messages_stay_in_one_group() {
        let (group, rest) =
            split_by_request_security_context(vec![queued("a", None), queued("b", None)]);
        assert_eq!(texts(&group), ["a", "b"]);
        assert!(rest.is_empty());
    }

    /// The escalation this split exists to stop: Collect replay runs a merged
    /// turn under the *last* message's params, so a guest message followed by
    /// an operator message would execute the guest's text with no
    /// `_tool_policy` at all.
    #[test]
    fn guest_text_is_never_merged_into_an_operator_turn() {
        let (group, rest) = split_by_request_security_context(vec![
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
        let (group, rest) = split_by_request_security_context(vec![
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
        let (group, rest) = split_by_request_security_context(vec![
            queued("a", Some(guest_policy())),
            queued("b", Some(other)),
        ]);
        assert_eq!(texts(&group), ["a"]);
        assert_eq!(texts(&rest), ["b"]);
    }

    #[test]
    fn same_policy_from_different_senders_splits_apart() {
        let mut alice = queued("a", Some(guest_policy()));
        alice.params["channel"] = serde_json::json!({"sender_id": "alice"});
        let mut bob = queued("b", Some(guest_policy()));
        bob.params["channel"] = serde_json::json!({"sender_id": "bob"});

        let (group, rest) = split_by_request_security_context(vec![alice, bob]);
        assert_eq!(texts(&group), ["a"]);
        assert_eq!(texts(&rest), ["b"]);
    }

    #[test]
    fn private_and_public_contexts_split_apart() {
        let private = queued("a", Some(guest_policy()));
        let mut public = queued("b", Some(guest_policy()));
        public.params["_private_context"] = serde_json::json!(false);

        let (group, rest) = split_by_request_security_context(vec![private, public]);
        assert_eq!(texts(&group), ["a"]);
        assert_eq!(texts(&rest), ["b"]);
    }

    #[test]
    fn public_and_trusted_audiences_split_apart() {
        let (group, rest) = split_by_request_security_context(vec![
            public_queued("public", Some(guest_policy())),
            queued("trusted", Some(guest_policy())),
        ]);
        assert_eq!(texts(&group), ["public"]);
        assert_eq!(texts(&rest), ["trusted"]);
    }

    #[test]
    fn different_thread_roots_split_apart() {
        let mut first = queued("a", Some(guest_policy()));
        first.params["_channel_reply_target"] = serde_json::json!({
            "channel_type": "slack",
            "account_id": "bot",
            "chat_id": "C123",
            "message_id": "thread-a",
        });
        let mut second = queued("b", Some(guest_policy()));
        second.params["_channel_reply_target"] = serde_json::json!({
            "channel_type": "slack",
            "account_id": "bot",
            "chat_id": "C123",
            "message_id": "thread-b",
        });

        let (group, rest) = split_by_request_security_context(vec![first, second]);
        assert_eq!(texts(&group), ["a"]);
        assert_eq!(texts(&rest), ["b"]);
    }

    #[test]
    fn empty_queue_splits_into_nothing() {
        let (group, rest) = split_by_request_security_context(Vec::new());
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

        let resolved =
            resolve_request_tool_registry(&base, Some(&policy), ToolAudience::Trusted).await;
        let names = resolved.read().await.list_names();

        assert!(!names.iter().any(|n| n == "exec"));
        assert!(!names.iter().any(|n| n == "memory_save"));
        assert!(names.iter().any(|n| n == "web_search"));

        // The shared registry must be untouched — the filter is per-run.
        assert_eq!(base.read().await.list_names().len(), 3);
    }

    #[tokio::test]
    async fn public_audience_cannot_be_widened_by_name_policy() {
        let mut registry = ToolRegistry::new();
        registry.register_public(Box::new(StubTool("calc")));
        registry.register(Box::new(StubTool("future_tool")));
        registry.register_mcp(Box::new(StubTool("mcp__example__read")), "example".into());
        let base = Arc::new(RwLock::new(registry));
        let permissive = ToolPolicy {
            allow: vec!["*".into()],
            deny: Vec::new(),
        };

        let resolved =
            resolve_request_tool_registry(&base, Some(&permissive), ToolAudience::Public).await;
        assert_eq!(resolved.read().await.list_names(), vec!["calc"]);
    }

    #[tokio::test]
    async fn deny_all_public_request_has_no_tools() {
        let mut registry = ToolRegistry::new();
        registry.register_public(Box::new(StubTool("calc")));
        registry.register(Box::new(StubTool("exec")));
        let base = Arc::new(RwLock::new(registry));
        let deny_all = ToolPolicy {
            allow: Vec::new(),
            deny: vec!["*".into()],
        };

        let resolved =
            resolve_request_tool_registry(&base, Some(&deny_all), ToolAudience::Public).await;
        assert!(resolved.read().await.is_empty());
    }

    #[tokio::test]
    async fn narrow_public_request_keeps_only_named_public_tool() {
        let mut registry = ToolRegistry::new();
        registry.register_public(Box::new(StubTool("calc")));
        registry.register_public(Box::new(StubTool("web_search")));
        registry.register(Box::new(StubTool("exec")));
        let base = Arc::new(RwLock::new(registry));
        let calc_only = ToolPolicy {
            allow: vec!["calc".into()],
            deny: Vec::new(),
        };

        let resolved =
            resolve_request_tool_registry(&base, Some(&calc_only), ToolAudience::Public).await;
        assert_eq!(resolved.read().await.list_names(), vec!["calc"]);
    }
}
