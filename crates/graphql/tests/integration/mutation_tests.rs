use {async_graphql::Request, serde_json::json};

use crate::common::{MockDispatch, build_test_schema};

#[tokio::test]
async fn config_set_mutation() {
    let mock = MockDispatch::new();
    mock.set_response("config.set", json!({"ok": true}));
    let (schema, _) = build_test_schema(mock.clone());

    let res = schema
        .execute(Request::new(
            r#"mutation { config { set(path: "theme", value: "dark") { ok } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let (method, params) = mock.last_call().expect("should have called");
    assert_eq!(method, "config.set");
    assert_eq!(params["path"], "theme");
    assert_eq!(params["value"], "dark");
}

#[tokio::test]
async fn chat_send_mutation() {
    let mock = MockDispatch::new();
    mock.set_response("chat.send", json!({"ok": true, "sessionKey": "sess1"}));
    let (schema, _) = build_test_schema(mock.clone());

    let res = schema
        .execute(Request::new(
            r#"mutation { chat { send(message: "Hello", sessionKey: "sess1") { ok } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let (method, params) = mock.last_call().expect("should have called");
    assert_eq!(method, "chat.send");
    assert_eq!(params["message"], "Hello");
    assert_eq!(params["sessionKey"], "sess1");
}

async fn assert_requires_session_key(query: &str, label: &str) {
    let mock = MockDispatch::new();
    let (schema, _) = build_test_schema(mock);
    let res = schema.execute(Request::new(query)).await;
    assert!(
        !res.errors.is_empty(),
        "{label} without sessionKey should fail"
    );
}

#[tokio::test]
async fn chat_send_requires_session_key() {
    assert_requires_session_key(
        r#"mutation { chat { send(message: "Hello") { ok } } }"#,
        "send",
    )
    .await;
}

#[tokio::test]
async fn chat_abort_requires_session_key() {
    assert_requires_session_key(r#"mutation { chat { abort { ok } } }"#, "abort").await;
}

#[tokio::test]
async fn chat_cancel_queued_requires_session_key() {
    assert_requires_session_key(
        r#"mutation { chat { cancelQueued { ok } } }"#,
        "cancelQueued",
    )
    .await;
}

#[tokio::test]
async fn chat_clear_requires_session_key() {
    assert_requires_session_key(r#"mutation { chat { clear { ok } } }"#, "clear").await;
}

#[tokio::test]
async fn chat_compact_requires_session_key() {
    assert_requires_session_key(r#"mutation { chat { compact { ok } } }"#, "compact").await;
}

#[tokio::test]
async fn chat_history_requires_session_key() {
    assert_requires_session_key(r#"query { chat { history } }"#, "history").await;
}

#[tokio::test]
async fn chat_context_requires_session_key() {
    assert_requires_session_key(r#"query { chat { context } }"#, "context").await;
}

#[tokio::test]
async fn chat_raw_prompt_requires_session_key() {
    assert_requires_session_key(r#"query { chat { rawPrompt { prompt } } }"#, "rawPrompt").await;
}

#[tokio::test]
async fn chat_full_context_requires_session_key() {
    assert_requires_session_key(r#"query { chat { fullContext } }"#, "fullContext").await;
}

#[tokio::test]
async fn agents_update_identity_mutation_returns_ok_on_success() {
    let mock = MockDispatch::new();
    mock.set_response(
        "agent.identity.update",
        json!({
            "name": "Rex",
            "user_name": "Alice",
        }),
    );
    let (schema, _) = build_test_schema(mock.clone());

    let res = schema
        .execute(Request::new(
            r#"mutation { agents { updateIdentity(input: { user_location: { latitude: 37.7749, longitude: -122.4194 } }) { ok } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert_eq!(data["agents"]["updateIdentity"]["ok"], true);

    let (method, params) = mock.last_call().expect("should have called");
    assert_eq!(method, "agent.identity.update");
    assert_eq!(params["user_location"]["latitude"], 37.7749);
    assert_eq!(params["user_location"]["longitude"], -122.4194);
}

#[tokio::test]
async fn agents_update_identity_accepts_json_string_payload() {
    let mock = MockDispatch::new();
    mock.set_response("agent.identity.update", json!({ "name": "Rex" }));
    let (schema, _) = build_test_schema(mock.clone());

    let res = schema
        .execute(Request::new(
            "mutation { agents { updateIdentity(input: \"{\\\"user_location\\\":{\\\"latitude\\\":37.0,\\\"longitude\\\":-122.0}}\") { ok } } }",
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let (method, params) = mock.last_call().expect("should have called");
    assert_eq!(method, "agent.identity.update");
    assert_eq!(params["user_location"]["latitude"], 37.0);
    assert_eq!(params["user_location"]["longitude"], -122.0);
}

#[tokio::test]
async fn providers_oauth_start_mutation_returns_typed_shape() {
    let mock = MockDispatch::new();
    mock.set_response(
        "providers.oauth.start",
        json!({
            "authUrl": "https://auth.example/start",
            "deviceFlow": false
        }),
    );
    let (schema, _) = build_test_schema(mock);

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
    let mock = MockDispatch::new();
    mock.set_response("cron.add", json!({"ok": true}));
    let (schema, _) = build_test_schema(mock.clone());

    let res = schema
        .execute(Request::new(
            r#"mutation { cron { add(input: { name: "backup" }) { ok } } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let (method, params) = mock.last_call().expect("should have called");
    assert_eq!(method, "cron.add");
    assert_eq!(params["name"], "backup");
}
