//! Hook system — re-exports core types from `moltis-common` and adds config.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// Re-export all core hook types so downstream code can use `moltis_plugins::hooks::*`.
pub use moltis_common::hooks::{
    HookAction, HookEvent, HookHandler, HookPayload, HookRegistry, HookStats,
};

// ── Hook configuration ──────────────────────────────────────────────────────

/// Configuration for a single shell hook, loaded from TOML/config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellHookConfig {
    pub name: String,
    pub command: String,
    pub events: Vec<HookEvent>,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_timeout() -> u64 {
    10
}

/// Top-level hooks configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    #[serde(default)]
    pub hooks: Vec<ShellHookConfig>,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use {anyhow::Result, async_trait::async_trait, serde_json::Value};

    use super::*;

    struct PassthroughHandler {
        subscribed: Vec<HookEvent>,
    }

    #[async_trait]
    impl HookHandler for PassthroughHandler {
        fn name(&self) -> &str {
            "passthrough"
        }

        fn events(&self) -> &[HookEvent] {
            &self.subscribed
        }

        async fn handle(&self, _event: HookEvent, _payload: &HookPayload) -> Result<HookAction> {
            Ok(HookAction::Continue)
        }
    }

    struct BlockingHandler {
        reason: String,
        subscribed: Vec<HookEvent>,
    }

    #[async_trait]
    impl HookHandler for BlockingHandler {
        fn name(&self) -> &str {
            "blocker"
        }

        fn events(&self) -> &[HookEvent] {
            &self.subscribed
        }

        async fn handle(&self, _event: HookEvent, _payload: &HookPayload) -> Result<HookAction> {
            Ok(HookAction::Block(self.reason.clone()))
        }
    }

    struct ModifyHandler {
        data: Value,
        subscribed: Vec<HookEvent>,
    }

    #[async_trait]
    impl HookHandler for ModifyHandler {
        fn name(&self) -> &str {
            "modifier"
        }

        fn events(&self) -> &[HookEvent] {
            &self.subscribed
        }

        async fn handle(&self, _event: HookEvent, _payload: &HookPayload) -> Result<HookAction> {
            Ok(HookAction::ModifyPayload(self.data.clone()))
        }
    }

    struct FailingHandler {
        subscribed: Vec<HookEvent>,
    }

    #[async_trait]
    impl HookHandler for FailingHandler {
        fn name(&self) -> &str {
            "failer"
        }

        fn events(&self) -> &[HookEvent] {
            &self.subscribed
        }

        async fn handle(&self, _event: HookEvent, _payload: &HookPayload) -> Result<HookAction> {
            anyhow::bail!("handler failed")
        }
    }

    fn test_payload() -> HookPayload {
        HookPayload::BeforeToolCall {
            session_key: "test-session".into(),
            tool_name: "exec".into(),
            arguments: serde_json::json!({"command": "ls"}),
        }
    }

    #[tokio::test]
    async fn dispatch_with_no_handlers_returns_continue() {
        let registry = HookRegistry::new();
        let result = registry.dispatch(&test_payload()).await.unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[tokio::test]
    async fn dispatch_passthrough_returns_continue() {
        let mut registry = HookRegistry::new();
        registry.register(Arc::new(PassthroughHandler {
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        let result = registry.dispatch(&test_payload()).await.unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[tokio::test]
    async fn dispatch_block_short_circuits() {
        let mut registry = HookRegistry::new();
        registry.register(Arc::new(BlockingHandler {
            reason: "dangerous".into(),
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        registry.register(Arc::new(PassthroughHandler {
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        let result = registry.dispatch(&test_payload()).await.unwrap();
        match result {
            HookAction::Block(reason) => assert_eq!(reason, "dangerous"),
            _ => panic!("expected Block"),
        }
    }

    #[tokio::test]
    async fn dispatch_modify_returns_last_modification() {
        let mut registry = HookRegistry::new();
        registry.register(Arc::new(ModifyHandler {
            data: serde_json::json!({"first": true}),
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        registry.register(Arc::new(ModifyHandler {
            data: serde_json::json!({"second": true}),
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        let result = registry.dispatch(&test_payload()).await.unwrap();
        match result {
            HookAction::ModifyPayload(v) => assert_eq!(v, serde_json::json!({"second": true})),
            _ => panic!("expected ModifyPayload"),
        }
    }

    #[tokio::test]
    async fn dispatch_error_is_non_fatal() {
        let mut registry = HookRegistry::new();
        registry.register(Arc::new(FailingHandler {
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        registry.register(Arc::new(PassthroughHandler {
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        let result = registry.dispatch(&test_payload()).await.unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[tokio::test]
    async fn has_handlers_returns_correct_values() {
        let mut registry = HookRegistry::new();
        assert!(!registry.has_handlers(HookEvent::BeforeToolCall));
        registry.register(Arc::new(PassthroughHandler {
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        assert!(registry.has_handlers(HookEvent::BeforeToolCall));
        assert!(!registry.has_handlers(HookEvent::SessionEnd));
    }

    #[tokio::test]
    async fn unrelated_events_dont_trigger() {
        let mut registry = HookRegistry::new();
        registry.register(Arc::new(BlockingHandler {
            reason: "should not fire".into(),
            subscribed: vec![HookEvent::SessionEnd],
        }));
        let result = registry.dispatch(&test_payload()).await.unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[test]
    fn payload_event_matches() {
        let payload = test_payload();
        assert_eq!(payload.event(), HookEvent::BeforeToolCall);

        let payload = HookPayload::GatewayStop;
        assert_eq!(payload.event(), HookEvent::GatewayStop);
    }

    #[test]
    fn hook_payload_serializes_roundtrip() {
        let payload = test_payload();
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: HookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event(), HookEvent::BeforeToolCall);
    }

    #[test]
    fn hook_config_deserializes() {
        let toml_str = r#"
[[hooks]]
name = "audit"
command = "/usr/local/bin/audit.sh"
events = ["BeforeToolCall", "AfterToolCall"]
timeout = 5

[[hooks]]
name = "notify"
command = "./notify.sh"
events = ["SessionEnd"]
"#;
        let config: HooksConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hooks.len(), 2);
        assert_eq!(config.hooks[0].name, "audit");
        assert_eq!(config.hooks[0].timeout, 5);
        assert_eq!(config.hooks[1].timeout, 10);
        assert_eq!(config.hooks[1].events, vec![HookEvent::SessionEnd]);
    }
}
