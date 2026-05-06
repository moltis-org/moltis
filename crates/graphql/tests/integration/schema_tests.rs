use {async_graphql::Request, serde_json::json};

use crate::common::{MockDispatch, build_test_schema};

#[tokio::test]
async fn introspection_returns_types() {
    let mock = MockDispatch::new();
    let (schema, _) = build_test_schema(mock);

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
    let mock = MockDispatch::new();
    let (schema, _) = build_test_schema(mock);

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

    for expected in [
        "health", "status", "sessions", "cron", "chat", "config", "mcp",
    ] {
        assert!(
            fields.contains(&expected.to_string()),
            "missing query field: {expected}, got: {fields:?}"
        );
    }
}

#[tokio::test]
async fn service_error_becomes_graphql_error() {
    let mock = MockDispatch::new();
    let (schema, _) = build_test_schema(mock);

    let res = schema.execute(Request::new("{ health { ok } }")).await;

    assert!(!res.errors.is_empty(), "expected an error");
    assert!(
        res.errors[0].message.contains("no mock response"),
        "error: {}",
        res.errors[0].message
    );
}

#[tokio::test]
async fn subscription_types_exist_in_schema() {
    let mock = MockDispatch::new();
    let (schema, _) = build_test_schema(mock);

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

#[tokio::test]
async fn multiple_root_queries() {
    let mock = MockDispatch::new();
    mock.set_response("health", json!({"ok": true}));
    mock.set_response("status", json!({"hostname": "h"}));
    let (schema, _) = build_test_schema(mock);

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
    let mock = MockDispatch::new();
    mock.set_response("health", json!({"ok": "yes"}));
    let (schema, _) = build_test_schema(mock);

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
