use {
    async_graphql::Request,
    serde_json::json,
    tokio::time::{Duration, timeout},
    tokio_stream::StreamExt,
};

use crate::common::{MockDispatch, build_test_schema};

#[tokio::test]
async fn chat_event_subscription_requires_session_key() {
    let mock = MockDispatch::new();
    let (schema, _) = build_test_schema(mock);

    let mut stream = schema.execute_stream(Request::new(r#"subscription { chatEvent { data } }"#));
    let resp = stream.next().await.expect("subscription response");

    assert!(
        !resp.errors.is_empty(),
        "chatEvent without sessionKey should fail"
    );
}

#[tokio::test]
async fn subscription_event_stream_variants_emit_payloads() {
    let mock = MockDispatch::new();
    let (schema, tx) = build_test_schema(mock);

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
    let mock = MockDispatch::new();
    let (schema, tx) = build_test_schema(mock);
    let mut stream = schema.execute_stream(Request::new(
        r#"subscription { chatEvent(sessionKey: "s1") { data } }"#,
    ));
    let _ = timeout(Duration::from_millis(20), stream.next()).await;

    tx.send((
        "chat".to_string(),
        json!({ "sessionKey": "other", "text": "skip" }),
    ))
    .expect("broadcast other");
    tx.send(("chat".to_string(), json!({ "text": "no-key" })))
        .expect("broadcast no-key");
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
    let mock = MockDispatch::new();
    let (schema, tx) = build_test_schema(mock);

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
