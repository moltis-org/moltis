//! Integration tests for the moltis-graphql crate.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use {
    async_graphql::Request,
    serde_json::{Value, json},
    tokio::sync::broadcast,
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
