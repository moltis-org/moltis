//! Integration tests for the moltis-graphql crate.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use {
    async_graphql::Request,
    serde_json::{Value, json},
    tokio::{
        sync::broadcast,
        time::{Duration, timeout},
    },
    tokio_stream::StreamExt,
};

use moltis_graphql::context::ServiceCaller;

/// Mock service caller that records calls and returns preset responses.
struct MockCaller {
    responses: Mutex<HashMap<String, Value>>,
    calls: Mutex<Vec<(String, Value)>>,
}

impl MockCaller {
    fn new() -> Self {
        Self {
            responses: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn set_response(&self, method: &str, response: Value) {
        self.responses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(method.to_string(), response);
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    fn last_call(&self) -> Option<(String, Value)> {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last()
            .cloned()
    }
}

#[async_trait::async_trait]
impl ServiceCaller for MockCaller {
    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((method.to_string(), params));
        let responses = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        match responses.get(method) {
            Some(v) => Ok(v.clone()),
            None => Err(format!("no mock response for {method}")),
        }
    }
}

fn build_test_schema(
    caller: Arc<MockCaller>,
) -> (
    moltis_graphql::MoltisSchema,
    broadcast::Sender<(String, Value)>,
) {
    let (tx, _) = broadcast::channel(16);
    let schema = moltis_graphql::build_schema(caller, tx.clone());
    (schema, tx)
}

fn set_responses(caller: &MockCaller, methods: &[&str], response: Value) {
    for method in methods {
        caller.set_response(method, response.clone());
    }
}

// ── Schema introspection ────────────────────────────────────────────────────

#[tokio::test]
async fn introspection_returns_types() {
    let caller = Arc::new(MockCaller::new());
    let (schema, _) = build_test_schema(caller);

    let res = schema
        .execute(Request::new(
            r#"{ __schema { queryType { name } mutationType { name } subscriptionType { name } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert_eq!(data["__schema"]["queryType"]["name"], "QueryRoot");
    assert_eq!(data["__schema"]["mutationType"]["name"], "MutationRoot");
    assert_eq!(
        data["__schema"]["subscriptionType"]["name"],
        "SubscriptionRoot"
    );
}

#[tokio::test]
async fn introspection_lists_query_fields() {
    let caller = Arc::new(MockCaller::new());
    let (schema, _) = build_test_schema(caller);

    let res = schema
        .execute(Request::new(
            r#"{ __type(name: "QueryRoot") { fields { name } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    let fields: Vec<String> = data["__type"]["fields"]
        .as_array()
        .expect("fields array")
        .iter()
        .map(|f| f["name"].as_str().expect("field name").to_string())
        .collect();

    // Verify key top-level query fields exist.
    for expected in [
        "health", "status", "sessions", "cron", "chat", "config", "mcp",
    ] {
        assert!(
            fields.contains(&expected.to_string()),
            "missing query field: {expected}, got: {fields:?}"
        );
    }
}

// ── Query resolvers ─────────────────────────────────────────────────────────

#[tokio::test]
async fn health_query_returns_data() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response("health", json!({"ok": true, "connections": 3}));
    let (schema, _) = build_test_schema(caller.clone());

    let res = schema
        .execute(Request::new("{ health { ok connections } }"))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert_eq!(data["health"]["ok"], true);
    assert_eq!(data["health"]["connections"], 3);
    assert_eq!(caller.call_count(), 1);
}

#[tokio::test]
async fn status_query_returns_data() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response(
        "status",
        json!({"hostname": "test-host", "version": "1.0.0", "connections": 5}),
    );
    let (schema, _) = build_test_schema(caller.clone());

    let res = schema
        .execute(Request::new("{ status { hostname version connections } }"))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert_eq!(data["status"]["hostname"], "test-host");
    assert_eq!(data["status"]["version"], "1.0.0");
    assert_eq!(data["status"]["connections"], 5);
}

#[tokio::test]
async fn cron_list_query() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response(
        "cron.list",
        json!([{"id": "job1", "name": "test-job", "enabled": true}]),
    );
    let (schema, _) = build_test_schema(caller.clone());

    let res = schema
        .execute(Request::new("{ cron { list { id name enabled } } }"))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    let list = &data["cron"]["list"];
    assert!(list.is_array());
    assert_eq!(list[0]["name"], "test-job");
}

#[tokio::test]
async fn sessions_list_query() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response(
        "sessions.list",
        json!([{"key": "sess1", "label": "test session"}]),
    );
    let (schema, _) = build_test_schema(caller.clone());

    let res = schema
        .execute(Request::new("{ sessions { list { key label } } }"))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert!(data["sessions"]["list"].is_array());
    assert_eq!(data["sessions"]["list"][0]["key"], "sess1");
}

#[tokio::test]
async fn system_presence_query_returns_typed_shape() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response(
        "system-presence",
        json!({
            "clients": [{"connId": "c1", "role": "operator", "connectedAt": 42}],
            "nodes": [{"nodeId": "n1", "displayName": "Node One"}]
        }),
    );
    let (schema, _) = build_test_schema(caller);

    let res = schema
        .execute(Request::new(
            r#"{ system { presence { clients { connId role connectedAt } nodes { nodeId displayName } } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert_eq!(data["system"]["presence"]["clients"][0]["connId"], "c1");
    assert_eq!(
        data["system"]["presence"]["nodes"][0]["displayName"],
        "Node One"
    );
}

#[tokio::test]
async fn logs_status_query_returns_typed_shape() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response(
        "logs.status",
        json!({
            "unseen_warns": 2,
            "unseen_errors": 1,
            "enabled_levels": {"debug": true, "trace": false}
        }),
    );
    let (schema, _) = build_test_schema(caller);

    let res = schema
        .execute(Request::new(
            r#"{ logs { status { unseenWarns unseenErrors enabledLevels { debug trace } } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert_eq!(data["logs"]["status"]["unseenWarns"], 2);
    assert_eq!(data["logs"]["status"]["enabledLevels"]["debug"], true);
}

// ── Mutation resolvers ──────────────────────────────────────────────────────

#[tokio::test]
async fn config_set_mutation() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response("config.set", json!({"ok": true}));
    let (schema, _) = build_test_schema(caller.clone());

    let res = schema
        .execute(Request::new(
            r#"mutation { config { set(path: "theme", value: "dark") { ok } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let (method, params) = caller.last_call().expect("should have called");
    assert_eq!(method, "config.set");
    assert_eq!(params["path"], "theme");
    assert_eq!(params["value"], "dark");
}

#[tokio::test]
async fn chat_send_mutation() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response("chat.send", json!({"ok": true, "sessionKey": "sess1"}));
    let (schema, _) = build_test_schema(caller.clone());

    let res = schema
        .execute(Request::new(
            r#"mutation { chat { send(message: "Hello") { ok } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let (method, params) = caller.last_call().expect("should have called");
    assert_eq!(method, "chat.send");
    assert_eq!(params["message"], "Hello");
}

#[tokio::test]
async fn providers_oauth_start_mutation_returns_typed_shape() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response(
        "providers.oauth.start",
        json!({
            "authUrl": "https://auth.example/start",
            "deviceFlow": false
        }),
    );
    let (schema, _) = build_test_schema(caller);

    let res = schema
        .execute(Request::new(
            r#"mutation { providers { oauthStart(provider: "openai") { authUrl deviceFlow } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert_eq!(
        data["providers"]["oauthStart"]["authUrl"],
        "https://auth.example/start"
    );
}

#[tokio::test]
async fn cron_add_mutation() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response("cron.add", json!({"ok": true}));
    let (schema, _) = build_test_schema(caller.clone());

    let res = schema
        .execute(Request::new(
            r#"mutation { cron { add(input: { name: "backup" }) { ok } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let (method, params) = caller.last_call().expect("should have called");
    assert_eq!(method, "cron.add");
    assert_eq!(params["name"], "backup");
}

// ── Error propagation ───────────────────────────────────────────────────────

#[tokio::test]
async fn service_error_becomes_graphql_error() {
    let caller = Arc::new(MockCaller::new());
    // Don't set any response — the mock will return Err("no mock response for health")
    let (schema, _) = build_test_schema(caller);

    let res = schema.execute(Request::new("{ health { ok } }")).await;

    assert!(!res.errors.is_empty(), "expected an error");
    assert!(
        res.errors[0].message.contains("no mock response"),
        "error: {}",
        res.errors[0].message
    );
}

// ── Namespace nesting ───────────────────────────────────────────────────────

#[tokio::test]
async fn nested_query_namespaces() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response("tts.status", json!({"enabled": true, "provider": "openai"}));
    caller.set_response("mcp.list", json!([]));
    let (schema, _) = build_test_schema(caller.clone());

    let res = schema
        .execute(Request::new(
            "{ tts { status { enabled provider } } mcp { list { name enabled } } }",
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert!(data["tts"]["status"].is_object());
    assert_eq!(data["tts"]["status"]["provider"], "openai");
    assert!(data["mcp"]["list"].is_array());
}

// ── Subscription types exist ────────────────────────────────────────────────

#[tokio::test]
async fn subscription_types_exist_in_schema() {
    let caller = Arc::new(MockCaller::new());
    let (schema, _) = build_test_schema(caller);

    let res = schema
        .execute(Request::new(
            r#"{ __type(name: "SubscriptionRoot") { fields { name } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    let fields: Vec<String> = data["__type"]["fields"]
        .as_array()
        .expect("fields array")
        .iter()
        .map(|f| f["name"].as_str().expect("field name").to_string())
        .collect();

    for expected in [
        "chatEvent",
        "sessionChanged",
        "cronNotification",
        "tick",
        "logEntry",
        "allEvents",
    ] {
        assert!(
            fields.contains(&expected.to_string()),
            "missing subscription: {expected}, got: {fields:?}"
        );
    }
}

// ── Multiple queries in one request ─────────────────────────────────────────

#[tokio::test]
async fn multiple_root_queries() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response("health", json!({"ok": true}));
    caller.set_response("status", json!({"hostname": "h"}));
    let (schema, _) = build_test_schema(caller.clone());

    let res = schema
        .execute(Request::new("{ health { ok } status { hostname } }"))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert_eq!(data["health"]["ok"], true);
    assert_eq!(data["status"]["hostname"], "h");
}

#[tokio::test]
async fn parse_error_becomes_graphql_error() {
    let caller = Arc::new(MockCaller::new());
    caller.set_response("health", json!({"ok": "yes"}));
    let (schema, _) = build_test_schema(caller);

    let res = schema.execute(Request::new("{ health { ok } }")).await;
    assert!(!res.errors.is_empty(), "expected parse error");
    assert!(
        res.errors[0].message.contains("failed to parse response"),
        "error: {}",
        res.errors[0].message
    );
}

#[test]
fn json_wrapper_traits_and_generic_event_conversion() {
    let parsed: moltis_graphql::scalars::Json =
        serde_json::from_value(json!({"k": ["v", 2]})).expect("json deserialization");
    let cloned = parsed.clone();
    assert_eq!(cloned.0["k"][0], "v");
    assert!(format!("{cloned:?}").contains("Json("));

    let event = moltis_graphql::types::GenericEvent::from(json!({"event": "x"}));
    assert_eq!(event.data.0["event"], "x");
}

#[tokio::test]
async fn query_resolvers_smoke_cover_all_namespaces() {
    let caller = Arc::new(MockCaller::new());

    set_responses(
        &caller,
        &[
            "status",
            "chat.context",
            "chat.raw_prompt",
            "chat.full_context",
            "sessions.preview",
            "sessions.resolve",
            "config.get",
            "config.schema",
            "cron.status",
            "heartbeat.status",
            "logs.status",
            "tts.status",
            "stt.status",
            "voice.config.get",
            "voice.elevenlabs.catalog",
            "voice.config.voxtral_requirements",
            "skills.bins",
            "skills.skill.detail",
            "skills.security.status",
            "skills.security.scan",
            "providers.local.system_info",
            "usage.status",
            "usage.cost",
            "exec.approvals.get",
            "exec.approvals.node.get",
            "projects.get",
            "projects.context",
            "memory.status",
            "memory.config.get",
            "agent.identity.get",
            "voicewake.get",
        ],
        json!({}),
    );

    set_responses(
        &caller,
        &[
            "system-presence",
            "node.list",
            "chat.history",
            "sessions.list",
            "sessions.search",
            "sessions.branches",
            "sessions.share.list",
            "channels.list",
            "cron.list",
            "cron.runs",
            "heartbeat.runs",
            "tts.providers",
            "stt.providers",
            "voice.providers.all",
            "skills.list",
            "skills.repos.list",
            "models.list",
            "models.list_all",
            "providers.available",
            "providers.local.models",
            "providers.local.search_hf",
            "mcp.list",
            "mcp.tools",
            "projects.list",
            "projects.complete_path",
            "hooks.list",
            "agents.list",
        ],
        json!([]),
    );

    set_responses(
        &caller,
        &[
            "health",
            "last-heartbeat",
            "channels.status",
            "skills.status",
            "providers.oauth.status",
            "providers.local.status",
            "mcp.status",
            "memory.qmd.status",
        ],
        json!({"ok": true}),
    );

    caller.set_response("node.describe", json!({ "ok": true }));
    caller.set_response("node.pair.list", json!([]));
    caller.set_response("channels.senders.list", json!({ "senders": [] }));
    caller.set_response("logs.tail", json!({ "entries": [], "subscribed": true }));
    caller.set_response("logs.list", json!({ "entries": [] }));
    caller.set_response("tts.generate_phrase", json!("hello"));
    caller.set_response("device.pair.list", json!([]));

    let (schema, _) = build_test_schema(caller);

    let res = schema
        .execute(Request::new(
            r#"
            {
              health { ok }
              status { hostname }
              system { presence { clients { connId } nodes { nodeId } } lastHeartbeat { ok } }
              node { list { nodeId } describe(nodeId: "n1") { nodeId displayName } pairRequests }
              chat {
                history(sessionKey: "s1")
                context(sessionKey: "s1")
                rawPrompt(sessionKey: "s1") { prompt }
                fullContext(sessionKey: "s1")
              }
              sessions {
                list { key }
                preview(key: "s1") { key }
                search(query: "q") { key }
                resolve(key: "s1") { key }
                branches(key: "s1") { key }
                shares(key: "s1") { id }
              }
              channels { status { ok } list { name } senders { senders { peerId } } }
              config { get(path: "chat.model") schema }
              cron { list { id } status { running } runs(jobId: "job") { jobId } }
              heartbeat { status { hasPrompt } runs(limit: 1) { jobId } }
              logs { tail(lines: 5) { subscribed entries { ts } } list { entries { ts } } status { unseenWarns } }
              tts { status { enabled } providers { name } generatePhrase }
              stt { status { enabled } providers { name } }
              voice { config { tts { enabled } stt { enabled } } providers { name } elevenlabsCatalog voxtralRequirements { os compatible } }
              skills {
                list { name }
                status { ok }
                bins
                repos { source }
                detail(name: "skill") { name }
                securityStatus { supported }
                securityScan { ok }
              }
              models { list { id } listAll { id } }
              providers {
                available { name }
                oauthStatus { ok }
                local {
                  systemInfo { totalRamGb }
                  models { id }
                  status { ok }
                  searchHf(query: "qwen")
                }
              }
              mcp { list { name } status { ok } tools(name: "srv") { name } }
              usage { status { sessionCount } cost { cost } }
              execApprovals { get { mode } nodeConfig { mode } }
              projects {
                list { id }
                get(id: "p1") { id }
                context(id: "p1") { project { id } }
                completePath(prefix: "./")
              }
              memory { status { enabled } config { backend } qmdStatus { ok } }
              hooks { list { name } }
              agents { list identity { name } }
              voicewake { get { enabled } }
              device { pairRequests }
            }
            "#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
}

#[tokio::test]
async fn mutation_resolvers_smoke_cover_all_namespaces() {
    let caller = Arc::new(MockCaller::new());

    set_responses(
        &caller,
        &[
            "system-event",
            "set-heartbeats",
            "wake",
            "talk.mode",
            "update.run",
            "node.rename",
            "node.pair.request",
            "node.pair.approve",
            "node.pair.reject",
            "node.pair.verify",
            "device.pair.approve",
            "device.pair.reject",
            "device.token.rotate",
            "device.token.revoke",
            "chat.send",
            "chat.abort",
            "chat.cancel_queued",
            "chat.clear",
            "chat.compact",
            "chat.inject",
            "sessions.switch",
            "sessions.fork",
            "sessions.patch",
            "sessions.reset",
            "sessions.delete",
            "sessions.clear_all",
            "sessions.compact",
            "sessions.share.revoke",
            "channels.add",
            "channels.remove",
            "channels.update",
            "channels.logout",
            "channels.senders.approve",
            "channels.senders.deny",
            "config.set",
            "config.apply",
            "config.patch",
            "cron.add",
            "cron.update",
            "cron.remove",
            "cron.run",
            "heartbeat.update",
            "heartbeat.run",
            "tts.enable",
            "tts.disable",
            "tts.setProvider",
            "stt.setProvider",
            "voice.config.save_key",
            "voice.config.save_settings",
            "voice.config.remove_key",
            "voice.provider.toggle",
            "voice.override.session.set",
            "voice.override.session.clear",
            "voice.override.channel.set",
            "voice.override.channel.clear",
            "skills.install",
            "skills.remove",
            "skills.update",
            "skills.repos.remove",
            "skills.emergency_disable",
            "skills.skill.trust",
            "skills.skill.enable",
            "skills.skill.disable",
            "skills.install_dep",
            "models.enable",
            "models.disable",
            "models.detect_supported",
            "providers.save_key",
            "providers.validate_key",
            "providers.save_model",
            "providers.save_models",
            "providers.remove_key",
            "providers.add_custom",
            "providers.oauth.complete",
            "providers.local.configure",
            "providers.local.configure_custom",
            "providers.local.remove_model",
            "mcp.add",
            "mcp.remove",
            "mcp.enable",
            "mcp.disable",
            "mcp.restart",
            "mcp.reauth",
            "mcp.update",
            "mcp.oauth.complete",
            "projects.upsert",
            "projects.delete",
            "projects.detect",
            "exec.approvals.set",
            "exec.approvals.node.set",
            "exec.approval.request",
            "exec.approval.resolve",
            "logs.ack",
            "memory.config.update",
            "hooks.enable",
            "hooks.disable",
            "hooks.save",
            "hooks.reload",
            "agent.identity.update",
            "agent.identity.update_soul",
            "voicewake.set",
        ],
        json!({"ok": true}),
    );

    caller.set_response("node.invoke", json!({"result": "ok"}));
    caller.set_response("sessions.share.create", json!({"id": "share-1"}));
    caller.set_response("tts.convert", json!({"audio": "AAAA"}));
    caller.set_response("stt.transcribe", json!({"text": "hello"}));
    caller.set_response("models.test", json!({"ok": true}));
    caller.set_response(
        "providers.oauth.start",
        json!({"authUrl": "https://auth.example/start", "deviceFlow": false}),
    );
    caller.set_response("mcp.oauth.start", json!({"ok": true}));
    caller.set_response("agent", json!({"status": "queued"}));
    caller.set_response("agent.wait", json!({"status": "done"}));
    caller.set_response("browser.request", json!({"ok": true}));

    let (schema, _) = build_test_schema(caller);

    let res = schema
        .execute(Request::new(
            r#"
            mutation {
              system {
                event(event: "test", payload: { k: "v", n: 1 }) { ok }
                setHeartbeats { ok }
                wake { ok }
                talkMode(mode: "brief") { ok }
                updateRun { ok }
              }
              node {
                invoke(input: { op: "ping" })
                rename(nodeId: "n1", displayName: "Node") { ok }
                pairRequest(input: { requestId: "r1" }) { ok }
                pairApprove(requestId: "r1") { ok }
                pairReject(requestId: "r1") { ok }
                pairVerify(input: { requestId: "r1" }) { ok }
              }
              device {
                pairApprove(deviceId: "d1") { ok }
                pairReject(deviceId: "d1") { ok }
                tokenRotate(deviceId: "d1") { ok }
                tokenRevoke(deviceId: "d1") { ok }
              }
              chat {
                send(message: "hi", sessionKey: "s1", model: "m1") { ok }
                abort(sessionKey: "s1") { ok }
                cancelQueued(sessionKey: "s1") { ok }
                clear(sessionKey: "s1") { ok }
                compact(sessionKey: "s1") { ok }
                inject(input: { role: "user", content: "x", sessionKey: "s1" }) { ok }
              }
              sessions {
                switch(key: "s1") { ok }
                fork(input: { key: "s1" }) { ok }
                patch(input: { key: "s1" }) { ok }
                reset(key: "s1") { ok }
                delete(key: "s1") { ok }
                clearAll { ok }
                compact(key: "s1") { ok }
                shareCreate(input: { key: "s1" }) { id }
                shareRevoke(shareId: "share-1") { ok }
              }
              channels {
                add(input: { name: "telegram" }) { ok }
                remove(name: "telegram") { ok }
                update(input: { name: "telegram" }) { ok }
                logout(name: "telegram") { ok }
                approveSender(input: { peerId: "p1" }) { ok }
                denySender(input: { peerId: "p1" }) { ok }
              }
              config {
                set(path: "chat.model", value: "gpt-test") { ok }
                apply(config: { chat: { model: "gpt-test" } }) { ok }
                patch(patch: { chat: { model: "gpt-2" } }) { ok }
              }
              cron {
                add(input: { name: "job" }) { ok }
                update(input: { id: "job" }) { ok }
                remove(id: "job") { ok }
                run(id: "job") { ok }
              }
              heartbeat {
                update(input: { enabled: true }) { ok }
                run { ok }
              }
              tts {
                enable(input: { provider: "mock" }) { ok }
                disable { ok }
                convert(audio: "AAAA") { audio }
                setProvider(provider: "mock") { ok }
              }
              stt {
                transcribe(input: { audio: "AAAA" }) { text }
                setProvider(provider: "mock") { ok }
              }
              voice {
                saveKey(input: { provider: "elevenlabs", key: "k" }) { ok }
                saveSettings(settings: { enabled: true }) { ok }
                removeKey(provider: "elevenlabs") { ok }
                toggleProvider(input: { provider: "elevenlabs", enabled: true }) { ok }
                sessionOverrideSet(input: { sessionKey: "s1" }) { ok }
                sessionOverrideClear(sessionKey: "s1") { ok }
                channelOverrideSet(input: { channelKey: "c1" }) { ok }
                channelOverrideClear(channelKey: "c1") { ok }
              }
              skills {
                install(input: { source: "repo" }) { ok }
                remove(source: "repo") { ok }
                update(name: "skill") { ok }
                reposRemove(source: "repo") { ok }
                emergencyDisable { ok }
                trust(name: "skill") { ok }
                enable(name: "skill") { ok }
                disable(name: "skill") { ok }
                installDep(input: { name: "jq" }) { ok }
              }
              models {
                enable(input: { id: "m1" }) { ok }
                disable(input: { id: "m1" }) { ok }
                detectSupported { ok }
                test(input: { id: "m1" }) { ok }
              }
              providers {
                saveKey(input: { provider: "openai", key: "k" }) { ok }
                validateKey(input: { provider: "openai", key: "k" }) { ok }
                saveModel(input: { provider: "openai", model: "m" }) { ok }
                saveModels(input: { provider: "openai", models: ["m"] }) { ok }
                removeKey(provider: "openai") { ok }
                addCustom(input: { name: "custom" }) { ok }
                oauthStart(provider: "openai") { authUrl }
                oauthComplete(input: { provider: "openai", code: "c" }) { ok }
                local {
                  configure(input: { backend: "llama" }) { ok }
                  configureCustom(input: { name: "custom" }) { ok }
                  removeModel(input: { id: "m1" }) { ok }
                }
              }
              mcp {
                add(input: { name: "srv" }) { ok }
                remove(name: "srv") { ok }
                enable(name: "srv") { ok }
                disable(name: "srv") { ok }
                restart(name: "srv") { ok }
                reauth(name: "srv") { ok }
                update(input: { name: "srv" }) { ok }
                oauthStart(name: "srv") { ok }
                oauthComplete(input: { name: "srv", code: "c" }) { ok }
              }
              projects {
                upsert(input: { id: "p1" }) { ok }
                delete(id: "p1") { ok }
                detect { ok }
              }
              execApprovals {
                set(input: { mode: "auto" }) { ok }
                setNodeConfig(input: { mode: "auto" }) { ok }
                request(input: { id: "req-1" }) { ok }
                resolve(input: { id: "req-1" }) { ok }
              }
              logs { ack { ok } }
              memory { updateConfig(input: { enabled: true }) { ok } }
              hooks {
                enable(name: "h1") { ok }
                disable(name: "h1") { ok }
                save(input: { name: "h1" }) { ok }
                reload { ok }
              }
              agents {
                run(input: { prompt: "hello" })
                runWait(input: { prompt: "hello" })
                updateIdentity(input: { name: "Bot" }) { ok }
                updateSoul(soul: "concise") { ok }
              }
              voicewake { set(input: { enabled: true }) { ok } }
              browser { request(input: { url: "https://example.com" }) }
            }
            "#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
}

#[tokio::test]
async fn subscription_event_stream_variants_emit_payloads() {
    let caller = Arc::new(MockCaller::new());
    let (schema, tx) = build_test_schema(caller);

    let cases = [
        ("sessionChanged", "session"),
        ("cronNotification", "cron"),
        ("channelEvent", "channel"),
        ("nodeEvent", "node"),
        ("logEntry", "logs"),
        ("mcpStatusChanged", "mcp.status"),
        ("configChanged", "config"),
        ("presenceChanged", "presence"),
        ("metricsUpdate", "metrics.update"),
        ("updateAvailable", "update.available"),
        ("voiceConfigChanged", "voice.config.changed"),
        ("skillsInstallProgress", "skills.install.progress"),
    ];

    for (field, event_name) in cases {
        let query = format!("subscription {{ {field} {{ data }} }}");
        let mut stream = schema.execute_stream(Request::new(query));
        let _ = timeout(Duration::from_millis(20), stream.next()).await;
        tx.send((event_name.to_string(), json!({ "kind": event_name })))
            .expect("broadcast");
        let resp = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("timeout")
            .expect("subscription response");
        assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
        let payload = resp.data.into_json().expect("json");
        assert_eq!(payload[field]["data"]["kind"], event_name);
    }
}

#[tokio::test]
async fn chat_event_subscription_filters_by_session_key() {
    let caller = Arc::new(MockCaller::new());
    let (schema, tx) = build_test_schema(caller);
    let mut stream = schema.execute_stream(Request::new(
        r#"subscription { chatEvent(sessionKey: "s1") { data } }"#,
    ));
    let _ = timeout(Duration::from_millis(20), stream.next()).await;

    tx.send((
        "chat".to_string(),
        json!({ "sessionKey": "other", "text": "skip" }),
    ))
    .expect("broadcast other");
    tx.send((
        "chat".to_string(),
        json!({ "sessionKey": "s1", "text": "deliver" }),
    ))
    .expect("broadcast matching");

    let resp = timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("timeout")
        .expect("subscription response");
    assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
    let payload = resp.data.into_json().expect("json");
    assert_eq!(payload["chatEvent"]["data"]["text"], "deliver");
}

#[tokio::test]
async fn tick_approval_and_all_events_subscriptions_emit() {
    let caller = Arc::new(MockCaller::new());
    let (schema, tx) = build_test_schema(caller);

    let mut tick = schema.execute_stream(Request::new(
        "subscription { tick { ts mem { process available total } } }",
    ));
    let _ = timeout(Duration::from_millis(20), tick.next()).await;
    tx.send((
        "tick".to_string(),
        json!({ "ts": 1, "mem": { "process": 2, "available": 3, "total": 4 } }),
    ))
    .expect("broadcast tick");
    let tick_resp = timeout(Duration::from_secs(1), tick.next())
        .await
        .expect("timeout")
        .expect("subscription response");
    assert!(
        tick_resp.errors.is_empty(),
        "errors: {:?}",
        tick_resp.errors
    );
    let tick_json = tick_resp.data.into_json().expect("json");
    assert_eq!(tick_json["tick"]["mem"]["total"], 4);

    let mut approval =
        schema.execute_stream(Request::new("subscription { approvalEvent { data } }"));
    let _ = timeout(Duration::from_millis(20), approval.next()).await;
    tx.send((
        "exec.approval.requested".to_string(),
        json!({ "requestId": "a1" }),
    ))
    .expect("broadcast approval");
    let approval_resp = timeout(Duration::from_secs(1), approval.next())
        .await
        .expect("timeout")
        .expect("subscription response");
    assert!(
        approval_resp.errors.is_empty(),
        "errors: {:?}",
        approval_resp.errors
    );
    let approval_json = approval_resp.data.into_json().expect("json");
    assert_eq!(approval_json["approvalEvent"]["data"]["requestId"], "a1");

    let mut all = schema.execute_stream(Request::new("subscription { allEvents { data } }"));
    let _ = timeout(Duration::from_millis(20), all.next()).await;
    tx.send(("custom.event".to_string(), json!({ "x": 1 })))
        .expect("broadcast all");
    let all_resp = timeout(Duration::from_secs(1), all.next())
        .await
        .expect("timeout")
        .expect("subscription response");
    assert!(all_resp.errors.is_empty(), "errors: {:?}", all_resp.errors);
    let all_json = all_resp.data.into_json().expect("json");
    assert_eq!(all_json["allEvents"]["data"]["x"], 1);
}
