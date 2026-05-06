use {async_graphql::Request, serde_json::json};

use crate::common::{MockDispatch, build_test_schema};

#[tokio::test]
async fn health_query_returns_data() {
    let mock = MockDispatch::new();
    mock.set_response("health", json!({"ok": true, "connections": 3}));
    let (schema, _) = build_test_schema(mock.clone());

    let res = schema
        .execute(Request::new("{ health { ok connections } }"))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let data = res.data.into_json().expect("json");
    assert_eq!(data["health"]["ok"], true);
    assert_eq!(data["health"]["connections"], 3);
    assert_eq!(mock.call_count(), 1);
}

#[tokio::test]
async fn status_query_returns_data() {
    let mock = MockDispatch::new();
    mock.set_response(
        "status",
        json!({"hostname": "test-host", "version": "1.0.0", "connections": 5}),
    );
    let (schema, _) = build_test_schema(mock);

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
    let mock = MockDispatch::new();
    mock.set_response(
        "cron.list",
        json!([{"id": "job1", "name": "test-job", "enabled": true}]),
    );
    let (schema, _) = build_test_schema(mock);

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
    let mock = MockDispatch::new();
    mock.set_response(
        "sessions.list",
        json!([{"key": "sess1", "label": "test session"}]),
    );
    let (schema, _) = build_test_schema(mock);

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
    let mock = MockDispatch::new();
    mock.set_response(
        "system-presence",
        json!({
            "clients": [{"connId": "c1", "role": "operator", "connectedAt": 42}],
            "nodes": [{"nodeId": "n1", "displayName": "Node One"}]
        }),
    );
    let (schema, _) = build_test_schema(mock);

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
    let mock = MockDispatch::new();
    mock.set_response(
        "logs.status",
        json!({
            "unseen_warns": 2,
            "unseen_errors": 1,
            "enabled_levels": {"debug": true, "trace": false}
        }),
    );
    let (schema, _) = build_test_schema(mock);

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

#[tokio::test]
async fn chat_history_query_forwards_session_key() {
    let mock = MockDispatch::new();
    mock.set_response("chat.history", json!([]));
    let (schema, _) = build_test_schema(mock.clone());

    let res = schema
        .execute(Request::new(
            r#"query { chat { history(sessionKey: "sess1") } }"#,
        ))
        .await;

    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let (method, params) = mock.last_call().expect("should have called");
    assert_eq!(method, "chat.history");
    assert_eq!(params["sessionKey"], "sess1");
}

#[tokio::test]
async fn nested_query_namespaces() {
    let mock = MockDispatch::new();
    mock.set_response("tts.status", json!({"enabled": true, "provider": "openai"}));
    mock.set_response("mcp.list", json!([]));
    let (schema, _) = build_test_schema(mock);

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
