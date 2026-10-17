#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    secrecy::SecretString,
    serde_json::{Value, json},
};

use {
    super::*,
    crate::test_support::{MockConfig, MockServer},
};

#[tokio::test]
async fn handshake_and_call() {
    let server = MockServer::start(MockConfig::default()).await;
    let conn = Connection::connect(&server.options()).await.unwrap();
    assert_eq!(conn.hello().server.conn_id, "conn-1");

    let payload = conn.call("echo", json!({"a": 1})).await.unwrap();
    assert_eq!(payload, json!({"a": 1}));

    let methods = server.methods();
    assert_eq!(methods, vec!["connect", "echo"]);
    let connect = &server.log.methods.lock().unwrap()[0].1;
    assert_eq!(connect["protocol"]["min"], 4);
    assert_eq!(connect["client"]["id"], "test-client");
    assert_eq!(connect["role"], "operator");
    assert!(connect.get("auth").is_none());
}

#[tokio::test]
async fn typed_call_and_null_params() {
    #[derive(serde::Serialize)]
    struct Params {
        text: &'static str,
    }
    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Reply {
        text: String,
    }
    let server = MockServer::start(MockConfig::default()).await;
    let conn = Connection::connect(&server.options()).await.unwrap();
    let reply: Reply = conn
        .call_typed("echo", &Params { text: "hi" })
        .await
        .unwrap();
    assert_eq!(reply.text, "hi");
    assert_eq!(conn.call("echo", Value::Null).await.unwrap(), Value::Null);
}

#[tokio::test]
async fn rpc_errors_are_typed() {
    let server = MockServer::start(MockConfig::default()).await;
    let conn = Connection::connect(&server.options()).await.unwrap();
    let error = conn.call("fail", Value::Null).await.unwrap_err();
    assert!(matches!(error, Error::Rpc { ref code, .. } if code == "NOT_FOUND"));
}

#[tokio::test]
async fn concurrent_calls_are_routed_by_id() {
    let server = MockServer::start(MockConfig::default()).await;
    let conn = Connection::connect(&server.options()).await.unwrap();
    let calls = (0..20).map(|i| {
        let conn = conn.clone();
        async move { conn.call("echo", json!({ "i": i })).await.unwrap() }
    });
    let results = futures::future::join_all(calls).await;
    for (i, result) in results.into_iter().enumerate() {
        assert_eq!(result["i"], i);
    }
}

#[tokio::test]
async fn events_are_broadcast() {
    let server = MockServer::start(MockConfig::default()).await;
    let conn = Connection::connect(&server.options()).await.unwrap();
    let mut events = conn.events();
    conn.call(
        "emit",
        json!({"event": "chat", "payload": {"state": "delta", "text": "x"}}),
    )
    .await
    .unwrap();
    let event = events.recv().await.unwrap();
    assert_eq!(event.event, "chat");
    assert_eq!(event.payload.unwrap()["text"], "x");
}

#[tokio::test]
async fn api_key_is_sent_in_header_and_connect() {
    let server = MockServer::start(MockConfig {
        required_key: Some("mk_good".into()),
        ..MockConfig::default()
    })
    .await;
    let options = server
        .options()
        .with_api_key(Some(SecretString::new("mk_good".into())));
    Connection::connect(&options).await.unwrap();
    assert_eq!(
        server.log.authorization.lock().unwrap()[0].as_deref(),
        Some("Bearer mk_good")
    );
    assert!(format!("{options:?}").contains("[REDACTED]"));
    assert!(!format!("{options:?}").contains("mk_good"));
}

#[tokio::test]
async fn wrong_key_is_unauthorized() {
    let server = MockServer::start(MockConfig {
        required_key: Some("mk_good".into()),
        ..MockConfig::default()
    })
    .await;
    let options = server
        .options()
        .with_api_key(Some(SecretString::new("mk_bad".into())));
    let error = Connection::connect(&options).await.unwrap_err();
    assert!(error.is_auth(), "{error:?}");
}

#[tokio::test]
async fn rejected_upgrade_is_unauthorized() {
    let server = MockServer::start(MockConfig {
        reject_upgrade: true,
        ..MockConfig::default()
    })
    .await;
    let error = Connection::connect(&server.options()).await.unwrap_err();
    assert!(matches!(error, Error::Unauthorized(_)), "{error:?}");
}

#[tokio::test]
async fn refused_connection_is_connect_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let endpoint = Endpoint::parse(&addr.to_string(), crate::tls::TlsTrust::System).unwrap();
    let options = ConnectOptions::new(endpoint, ClientIdentity::new("t", "0"));
    let error = Connection::connect(&options).await.unwrap_err();
    assert!(matches!(error, Error::Connect(_)), "{error:?}");
}

#[tokio::test]
async fn disconnect_fails_pending_calls_and_reports_closed() {
    let server = MockServer::start(MockConfig::default()).await;
    let conn = Connection::connect(&server.options()).await.unwrap();
    let error = conn.call("drop", Value::Null).await.unwrap_err();
    assert!(matches!(error, Error::Disconnected), "{error:?}");
    let reason = conn.closed().await;
    assert!(!reason.is_empty());
    assert!(conn.is_closed());
    assert!(matches!(
        conn.call("echo", Value::Null).await.unwrap_err(),
        Error::Disconnected
    ));
}

#[tokio::test]
async fn slow_call_times_out() {
    let server = MockServer::start(MockConfig::default()).await;
    let mut options = server.options();
    options.request_timeout = Duration::from_millis(100);
    let conn = Connection::connect(&options).await.unwrap();
    let error = conn.call("slow", Value::Null).await.unwrap_err();
    assert!(matches!(error, Error::Timeout));
    // The connection stays usable.
    assert_eq!(conn.call("echo", json!(1)).await.unwrap(), json!(1));
}

#[tokio::test]
async fn server_requests_get_an_error_reply() {
    let server = MockServer::start(MockConfig::default()).await;
    let conn = Connection::connect(&server.options()).await.unwrap();
    conn.call("ask_client", Value::Null).await.unwrap();
    // A round trip after the server's request guarantees the reply was read.
    conn.call("echo", Value::Null).await.unwrap();
    let replies = server.log.client_replies.lock().unwrap().clone();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["id"], "srv-1");
    assert_eq!(replies[0]["ok"], false);
    assert_eq!(replies[0]["error"]["code"], "UNKNOWN_METHOD");
}

#[tokio::test]
async fn explicit_close() {
    let server = MockServer::start(MockConfig::default()).await;
    let conn = Connection::connect(&server.options()).await.unwrap();
    conn.close();
    conn.closed().await;
    assert!(conn.is_closed());
}
