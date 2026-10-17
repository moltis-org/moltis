#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use {serde_json::json, tokio::net::TcpListener};

use {
    super::*,
    crate::test_support::{MockConfig, MockServer},
};

fn fast() -> Backoff {
    Backoff::new(Duration::from_millis(20), Duration::from_millis(50))
}

async fn next(updates: &mut mpsc::UnboundedReceiver<ClientUpdate>) -> ClientUpdate {
    tokio::time::timeout(Duration::from_secs(5), updates.recv())
        .await
        .expect("update in time")
        .expect("channel open")
}

async fn until_connected(updates: &mut mpsc::UnboundedReceiver<ClientUpdate>) {
    loop {
        if let ClientUpdate::Connected { .. } = next(updates).await {
            return;
        }
    }
}

#[tokio::test]
async fn connects_subscribes_and_forwards_events() {
    let server = MockServer::start(MockConfig::default()).await;
    let (client, mut updates) =
        ManagedClient::spawn_with_backoff(server.options(), vec!["chat".into()], fast());
    assert!(matches!(
        next(&mut updates).await,
        ClientUpdate::Connecting { attempt: 1 }
    ));
    until_connected(&mut updates).await;
    let subscribe = server.log.methods.lock().unwrap()[1].clone();
    assert_eq!(subscribe, ("subscribe".into(), json!({"events": ["chat"]})));

    client
        .call("emit", json!({"event": "chat", "payload": {"n": 1}}))
        .await
        .unwrap();
    let ClientUpdate::Event(event) = next(&mut updates).await else {
        panic!("expected event");
    };
    assert_eq!(event.payload.unwrap()["n"], 1);
    client.shutdown();
}

#[tokio::test]
async fn reconnects_after_drop() {
    let server = MockServer::start(MockConfig::default()).await;
    let (client, mut updates) = ManagedClient::spawn_with_backoff(server.options(), vec![], fast());
    until_connected(&mut updates).await;
    let _ = client.call("drop", json!(null)).await;
    loop {
        match next(&mut updates).await {
            ClientUpdate::Disconnected { .. } => break,
            ClientUpdate::Event(_) | ClientUpdate::Connecting { .. } => {},
            other => panic!("unexpected {other:?}"),
        }
    }
    until_connected(&mut updates).await;
    assert_eq!(
        server
            .log
            .connections
            .load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert!(client.connection().is_some());
    client.shutdown();
}

#[tokio::test]
async fn retries_until_daemon_starts() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let endpoint = crate::endpoint::Endpoint::parse(&addr.to_string(), Default::default()).unwrap();
    let options = ConnectOptions::new(endpoint, crate::connection::ClientIdentity::new("t", "0"));
    let (client, mut updates) = ManagedClient::spawn_with_backoff(options, vec![], fast());
    let mut failures = 0;
    while failures < 2 {
        if let ClientUpdate::Disconnected { kind, .. } = next(&mut updates).await {
            assert_eq!(kind, FailureKind::Unreachable);
            failures += 1;
        }
    }
    assert!(matches!(
        client.call("echo", json!(1)).await.unwrap_err(),
        Error::Disconnected
    ));
    let _server = MockServer::serve(
        TcpListener::bind(addr).await.unwrap(),
        MockConfig::default(),
    );
    client.reconnect_now();
    until_connected(&mut updates).await;
    assert_eq!(client.call("echo", json!(1)).await.unwrap(), json!(1));
    client.shutdown();
}

#[tokio::test]
async fn auth_failure_stops_retrying() {
    let server = MockServer::start(MockConfig {
        required_key: Some("mk_x".into()),
        ..MockConfig::default()
    })
    .await;
    let (_client, mut updates) =
        ManagedClient::spawn_with_backoff(server.options(), vec![], fast());
    loop {
        match next(&mut updates).await {
            ClientUpdate::AuthFailed { reason } => {
                assert!(reason.contains("bad key"), "{reason}");
                break;
            },
            ClientUpdate::Connecting { .. } => {},
            other => panic!("unexpected {other:?}"),
        }
    }
    // The supervisor exited, so the channel closes.
    assert!(updates.recv().await.is_none());
}

#[tokio::test]
async fn shutdown_closes_connection() {
    let server = MockServer::start(MockConfig::default()).await;
    let (client, mut updates) = ManagedClient::spawn_with_backoff(server.options(), vec![], fast());
    until_connected(&mut updates).await;
    let connection = client.connection().unwrap();
    client.shutdown();
    connection.closed().await;
    assert!(updates.recv().await.is_none());
    assert!(client.connection().is_none());
}
