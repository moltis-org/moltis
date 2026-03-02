use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use tracing::{debug, warn};

use moltis_protocol::{ErrorShape, ResponseFrame, error_codes};

use crate::state::GatewayState;

mod channel_mux;
mod gateway;
mod node;
mod pairing;
mod services;
mod subscribe;
mod voice;

// ── Types ────────────────────────────────────────────────────────────────────

/// Context passed to every method handler.
pub struct MethodContext {
    pub request_id: String,
    pub method: String,
    pub params: serde_json::Value,
    pub client_conn_id: String,
    pub client_role: String,
    pub client_scopes: Vec<String>,
    pub state: Arc<GatewayState>,
    /// Optional channel context from the request frame (v4).
    pub channel: Option<String>,
}

/// The result a method handler produces.
pub type MethodResult = Result<serde_json::Value, ErrorShape>;

/// A boxed async method handler.
pub type HandlerFn =
    Box<dyn Fn(MethodContext) -> Pin<Box<dyn Future<Output = MethodResult> + Send>> + Send + Sync>;

// ── Scope authorization ──────────────────────────────────────────────────────

const NODE_METHODS: &[&str] = &["node.invoke.result", "node.event", "skills.bins"];

const READ_METHODS: &[&str] = &[
    "health",
    "logs.tail",
    "logs.list",
    "logs.status",
    "channels.status",
    "channels.list",
    "channels.senders.list",
    "status",
    "usage.status",
    "usage.cost",
    "tts.status",
    "tts.providers",
    "stt.status",
    "stt.providers",
    "models.list",
    "models.list_all",
    #[cfg(feature = "agent")]
    "agents.list",
    #[cfg(feature = "agent")]
    "agents.get",
    #[cfg(feature = "agent")]
    "agents.identity.get",
    #[cfg(feature = "agent")]
    "agents.files.list",
    #[cfg(feature = "agent")]
    "agents.files.get",
    "agent.identity.get",
    "skills.list",
    "skills.status",
    "skills.security.status",
    "skills.security.scan",
    "skills.repos.list",
    "voicewake.get",
    "sessions.list",
    "sessions.preview",
    "sessions.search",
    "sessions.branches",
    "sessions.run_detail",
    "sessions.share.list",
    "projects.list",
    "projects.get",
    "projects.context",
    "projects.complete_path",
    "cron.list",
    "cron.status",
    "cron.runs",
    "heartbeat.status",
    "heartbeat.runs",
    "system-presence",
    "last-heartbeat",
    "node.list",
    "node.describe",
    "chat.history",
    "chat.context",
    "chat.raw_prompt",
    "providers.available",
    "providers.oauth.status",
    "providers.local.system_info",
    "providers.local.models",
    "providers.local.status",
    "providers.local.search_hf",
    "mcp.list",
    "mcp.status",
    "mcp.tools",
    "tts.generate_phrase",
    "voice.config.get",
    "voice.config.voxtral_requirements",
    "voice.providers.all",
    "voice.elevenlabs.catalog",
    #[cfg(feature = "graphql")]
    "graphql.config.get",
    "memory.status",
    "memory.config.get",
    "memory.qmd.status",
    "hooks.list",
    "network.audit.list",
    "network.audit.tail",
    "network.audit.stats",
    "openclaw.detect",
    "openclaw.scan",
    "system.describe",
];

const WRITE_METHODS: &[&str] = &[
    "send",
    "agent",
    "agent.wait",
    "agent.identity.update",
    "agent.identity.update_soul",
    #[cfg(feature = "agent")]
    "agents.create",
    #[cfg(feature = "agent")]
    "agents.update",
    #[cfg(feature = "agent")]
    "agents.delete",
    #[cfg(feature = "agent")]
    "agents.set_default",
    #[cfg(feature = "agent")]
    "agents.set_session",
    #[cfg(feature = "agent")]
    "agents.identity.update",
    #[cfg(feature = "agent")]
    "agents.identity.update_soul",
    #[cfg(feature = "agent")]
    "agents.files.set",
    "wake",
    "talk.mode",
    "tts.enable",
    "tts.disable",
    "tts.convert",
    "tts.setProvider",
    "stt.transcribe",
    "stt.setProvider",
    "voicewake.set",
    "node.invoke",
    "chat.send",
    "chat.abort",
    "chat.cancel_queued",
    "chat.clear",
    "chat.compact",
    "browser.request",
    "logs.ack",
    "models.detect_supported",
    "models.test",
    "providers.save_key",
    "providers.save_model",
    "providers.save_models",
    "providers.validate_key",
    "providers.remove_key",
    "providers.add_custom",
    "providers.oauth.start",
    "providers.oauth.complete",
    "providers.local.configure",
    "providers.local.configure_custom",
    "channels.add",
    "channels.remove",
    "channels.update",
    "channels.senders.approve",
    "channels.senders.deny",
    "sessions.switch",
    "sessions.fork",
    "sessions.voice.generate",
    "sessions.clear_all",
    "sessions.share.create",
    "sessions.share.revoke",
    "projects.upsert",
    "projects.delete",
    "projects.detect",
    "skills.install",
    "skills.remove",
    "skills.repos.remove",
    "skills.emergency_disable",
    "skills.skill.trust",
    "skills.skill.enable",
    "skills.skill.disable",
    "skills.install_dep",
    "mcp.add",
    "mcp.remove",
    "mcp.enable",
    "mcp.disable",
    "mcp.restart",
    "mcp.reauth",
    "mcp.update",
    "mcp.oauth.start",
    "mcp.oauth.complete",
    "cron.add",
    "cron.update",
    "cron.remove",
    "cron.run",
    "heartbeat.update",
    "heartbeat.run",
    "voice.config.save_key",
    "voice.config.save_settings",
    "voice.config.remove_key",
    "voice.provider.toggle",
    "voice.override.session.set",
    "voice.override.session.clear",
    "voice.override.channel.set",
    "voice.override.channel.clear",
    #[cfg(feature = "graphql")]
    "graphql.config.set",
    "memory.config.update",
    "hooks.enable",
    "hooks.disable",
    "hooks.save",
    "hooks.reload",
    "location.result",
    "openclaw.import",
    "subscribe",
    "unsubscribe",
    "channel.join",
    "channel.leave",
];

const APPROVAL_METHODS: &[&str] = &["exec.approval.request", "exec.approval.resolve"];

const PAIRING_METHODS: &[&str] = &[
    "node.pair.request",
    "node.pair.list",
    "node.pair.approve",
    "node.pair.reject",
    "node.pair.verify",
    "device.pair.list",
    "device.pair.approve",
    "device.pair.reject",
    "device.token.rotate",
    "device.token.revoke",
    "node.rename",
];

fn is_in(method: &str, list: &[&str]) -> bool {
    list.contains(&method)
}

/// Check role + scopes for a method. Returns None if authorized, Some(error) if not.
pub fn authorize_method(method: &str, role: &str, scopes: &[String]) -> Option<ErrorShape> {
    use moltis_protocol::scopes as s;

    if is_in(method, NODE_METHODS) {
        if role == "node" {
            return None;
        }
        return Some(ErrorShape::new(
            error_codes::FORBIDDEN,
            format!("unauthorized role: {role}"),
        ));
    }
    if role == "node" || role != "operator" {
        return Some(ErrorShape::new(
            error_codes::FORBIDDEN,
            format!("unauthorized role: {role}"),
        ));
    }

    let has = |scope: &str| scopes.iter().any(|s| s == scope);
    if has(s::ADMIN) {
        return None;
    }

    if is_in(method, APPROVAL_METHODS) && !has(s::APPROVALS) {
        return Some(ErrorShape::new(
            error_codes::UNAUTHORIZED,
            "missing scope: operator.approvals",
        ));
    }
    if is_in(method, PAIRING_METHODS) && !has(s::PAIRING) {
        return Some(ErrorShape::new(
            error_codes::UNAUTHORIZED,
            "missing scope: operator.pairing",
        ));
    }
    if is_in(method, READ_METHODS) && !(has(s::READ) || has(s::WRITE)) {
        return Some(ErrorShape::new(
            error_codes::UNAUTHORIZED,
            "missing scope: operator.read",
        ));
    }
    if is_in(method, WRITE_METHODS) && !has(s::WRITE) {
        return Some(ErrorShape::new(
            error_codes::UNAUTHORIZED,
            "missing scope: operator.write",
        ));
    }

    if is_in(method, APPROVAL_METHODS)
        || is_in(method, PAIRING_METHODS)
        || is_in(method, READ_METHODS)
        || is_in(method, WRITE_METHODS)
    {
        return None;
    }

    Some(ErrorShape::new(
        error_codes::UNAUTHORIZED,
        "missing scope: operator.admin",
    ))
}

// ── Method registry ──────────────────────────────────────────────────────────

pub struct MethodRegistry {
    handlers: HashMap<String, HandlerFn>,
}

impl Default for MethodRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MethodRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            handlers: HashMap::new(),
        };
        reg.register_defaults();
        reg
    }

    pub fn register(&mut self, method: impl Into<String>, handler: HandlerFn) {
        self.handlers.insert(method.into(), handler);
    }

    pub async fn dispatch(&self, ctx: MethodContext) -> ResponseFrame {
        let method = ctx.method.clone();
        let request_id = ctx.request_id.clone();
        let conn_id = ctx.client_conn_id.clone();

        if let Some(err) = authorize_method(&method, &ctx.client_role, &ctx.client_scopes) {
            warn!(method, conn_id = %conn_id, code = %err.code, "method auth denied");
            return ResponseFrame::err(&request_id, err);
        }

        let Some(handler) = self.handlers.get(&method) else {
            warn!(method, conn_id = %conn_id, "unknown method");
            return ResponseFrame::err(
                &request_id,
                ErrorShape::new(
                    error_codes::UNKNOWN_METHOD,
                    format!("unknown method: {method}"),
                ),
            );
        };

        debug!(method, request_id = %request_id, conn_id = %conn_id, "dispatching method");
        match handler(ctx).await {
            Ok(payload) => {
                debug!(method, request_id = %request_id, "method ok");
                ResponseFrame::ok(&request_id, payload)
            },
            Err(err) => {
                if err.code == error_codes::UNAVAILABLE {
                    debug!(method, request_id = %request_id, code = %err.code, msg = %err.message, "method unavailable");
                } else {
                    warn!(method, request_id = %request_id, code = %err.code, msg = %err.message, "method error");
                }
                ResponseFrame::err(&request_id, err)
            },
        }
    }

    pub fn method_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.handlers.keys().cloned().collect();
        names.sort();
        names
    }

    fn register_defaults(&mut self) {
        gateway::register(self);
        node::register(self);
        pairing::register(self);
        services::register(self);
        subscribe::register(self);
        channel_mux::register(self);
    }
}

/// Load the disabled hooks set from `data_dir/disabled_hooks.json`.
pub(crate) fn load_disabled_hooks() -> std::collections::HashSet<String> {
    let path = moltis_config::data_dir().join("disabled_hooks.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn scopes(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn assert_error_code(result: Option<ErrorShape>, expected_code: &str) {
        let err = result.expect("expected an error");
        assert_eq!(err.code, expected_code, "wrong error code: {}", err.message);
    }

    #[test]
    fn senders_list_requires_read() {
        assert!(
            authorize_method(
                "channels.senders.list",
                "operator",
                &scopes(&["operator.read"])
            )
            .is_none()
        );
        assert_error_code(
            authorize_method("channels.senders.list", "operator", &scopes(&[])),
            "UNAUTHORIZED",
        );
    }

    #[test]
    fn senders_approve_requires_write() {
        assert!(
            authorize_method(
                "channels.senders.approve",
                "operator",
                &scopes(&["operator.write"])
            )
            .is_none()
        );
        assert_error_code(
            authorize_method(
                "channels.senders.approve",
                "operator",
                &scopes(&["operator.read"]),
            ),
            "UNAUTHORIZED",
        );
    }

    #[test]
    fn senders_deny_requires_write() {
        assert!(
            authorize_method(
                "channels.senders.deny",
                "operator",
                &scopes(&["operator.write"])
            )
            .is_none()
        );
        assert_error_code(
            authorize_method(
                "channels.senders.deny",
                "operator",
                &scopes(&["operator.read"]),
            ),
            "UNAUTHORIZED",
        );
    }

    #[test]
    fn admin_scope_allows_all_sender_methods() {
        for method in &[
            "channels.senders.list",
            "channels.senders.approve",
            "channels.senders.deny",
        ] {
            assert!(
                authorize_method(method, "operator", &scopes(&["operator.admin"])).is_none(),
                "admin should authorize {method}"
            );
        }
    }

    #[test]
    fn node_role_denied_sender_methods() {
        for method in &[
            "channels.senders.list",
            "channels.senders.approve",
            "channels.senders.deny",
        ] {
            assert_error_code(
                authorize_method(method, "node", &scopes(&["operator.admin"])),
                "FORBIDDEN",
            );
        }
    }

    #[cfg(feature = "graphql")]
    #[test]
    fn graphql_config_get_requires_read() {
        assert!(
            authorize_method(
                "graphql.config.get",
                "operator",
                &scopes(&["operator.read"])
            )
            .is_none()
        );
        assert_error_code(
            authorize_method("graphql.config.get", "operator", &scopes(&[])),
            "UNAUTHORIZED",
        );
    }

    #[cfg(feature = "graphql")]
    #[test]
    fn graphql_config_set_requires_write() {
        assert!(
            authorize_method(
                "graphql.config.set",
                "operator",
                &scopes(&["operator.write"])
            )
            .is_none()
        );
        assert_error_code(
            authorize_method(
                "graphql.config.set",
                "operator",
                &scopes(&["operator.read"]),
            ),
            "UNAUTHORIZED",
        );
    }

    #[test]
    fn identity_get_requires_read() {
        assert!(
            authorize_method(
                "agent.identity.get",
                "operator",
                &scopes(&["operator.read"])
            )
            .is_none()
        );
        assert_error_code(
            authorize_method("agent.identity.get", "operator", &scopes(&[])),
            "UNAUTHORIZED",
        );
    }

    #[test]
    fn identity_update_requires_write() {
        assert!(
            authorize_method(
                "agent.identity.update",
                "operator",
                &scopes(&["operator.write"])
            )
            .is_none()
        );
        assert_error_code(
            authorize_method(
                "agent.identity.update",
                "operator",
                &scopes(&["operator.read"]),
            ),
            "UNAUTHORIZED",
        );
    }

    #[test]
    fn identity_update_soul_requires_write() {
        assert!(
            authorize_method(
                "agent.identity.update_soul",
                "operator",
                &scopes(&["operator.write"])
            )
            .is_none()
        );
        assert_error_code(
            authorize_method(
                "agent.identity.update_soul",
                "operator",
                &scopes(&["operator.read"]),
            ),
            "UNAUTHORIZED",
        );
    }

    #[test]
    fn cron_read_methods_require_read() {
        for method in &["cron.list", "cron.status", "cron.runs"] {
            assert!(
                authorize_method(method, "operator", &scopes(&["operator.read"])).is_none(),
                "read scope should authorize {method}"
            );
            assert_error_code(
                authorize_method(method, "operator", &scopes(&[])),
                "UNAUTHORIZED",
            );
        }
    }

    #[test]
    fn cron_write_methods_require_write() {
        for method in &["cron.add", "cron.update", "cron.remove", "cron.run"] {
            assert!(
                authorize_method(method, "operator", &scopes(&["operator.write"])).is_none(),
                "write scope should authorize {method}"
            );
            assert_error_code(
                authorize_method(method, "operator", &scopes(&["operator.read"])),
                "UNAUTHORIZED",
            );
        }
    }

    #[test]
    fn hooks_list_requires_read() {
        assert!(authorize_method("hooks.list", "operator", &scopes(&["operator.read"])).is_none());
        assert_error_code(
            authorize_method("hooks.list", "operator", &scopes(&[])),
            "UNAUTHORIZED",
        );
    }

    #[test]
    fn hooks_write_methods_require_write() {
        for method in &[
            "hooks.enable",
            "hooks.disable",
            "hooks.save",
            "hooks.reload",
        ] {
            assert!(
                authorize_method(method, "operator", &scopes(&["operator.write"])).is_none(),
                "write scope should authorize {method}"
            );
            assert_error_code(
                authorize_method(method, "operator", &scopes(&["operator.read"])),
                "UNAUTHORIZED",
            );
        }
    }

    #[test]
    fn unknown_method_returns_unknown_code() {
        use crate::{
            auth::{AuthMode, ResolvedAuth},
            services::GatewayServices,
            state::GatewayState,
        };

        let reg = MethodRegistry::new();
        let ctx = MethodContext {
            request_id: "test".into(),
            method: "nonexistent.method".into(),
            params: serde_json::Value::Null,
            client_conn_id: "conn-1".into(),
            client_role: "operator".into(),
            client_scopes: scopes(&["operator.admin"]),
            state: GatewayState::new(
                ResolvedAuth {
                    mode: AuthMode::Token,
                    token: None,
                    password: None,
                },
                GatewayServices::noop(),
            ),
            channel: None,
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let resp = rt.block_on(reg.dispatch(ctx));
        assert!(!resp.ok);
        assert_eq!(
            resp.error.as_ref().map(|e| e.code.as_str()),
            Some("UNKNOWN_METHOD")
        );
    }

    #[test]
    fn subscribe_method_authorized_with_write() {
        assert!(authorize_method("subscribe", "operator", &scopes(&["operator.write"])).is_none());
    }

    #[test]
    fn channel_join_authorized_with_write() {
        assert!(
            authorize_method("channel.join", "operator", &scopes(&["operator.write"])).is_none()
        );
    }

    #[test]
    fn system_describe_authorized_with_read() {
        assert!(
            authorize_method("system.describe", "operator", &scopes(&["operator.read"])).is_none()
        );
    }

    #[test]
    fn model_probe_params_include_provider_when_present() {
        let params = services::model_probe_params(Some("github-copilot"));
        assert_eq!(params["background"], serde_json::json!(true));
        assert_eq!(params["reason"], serde_json::json!("provider_connected"));
        assert_eq!(params["provider"], serde_json::json!("github-copilot"));
    }

    #[test]
    fn model_probe_params_omit_provider_when_missing() {
        let params = services::model_probe_params(None);
        assert_eq!(params["background"], serde_json::json!(true));
        assert_eq!(params["reason"], serde_json::json!("provider_connected"));
        assert!(params.get("provider").is_none());
    }

    #[test]
    fn model_probe_params_omit_provider_when_blank() {
        let params = services::model_probe_params(Some("   "));
        assert!(params.get("provider").is_none());
    }
}
