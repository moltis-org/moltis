#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The native client crate (`moltis-client`) against a real gateway: the
//! handshake, subscriptions and typed event payloads must stay compatible.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use {
    moltis_client::{
        ClientIdentity, ClientUpdate, ConnectOptions, Connection, Endpoint, ManagedClient,
        TlsTrust,
        api::Rpc,
        protocol::{
            chat_event::{ChatEvent, ChatEventKind},
            client_api::{events, methods},
        },
    },
    moltis_gateway::{
        auth,
        broadcast::{BroadcastOpts, broadcast},
        methods::MethodRegistry,
        services::GatewayServices,
        state::GatewayState,
    },
    moltis_httpd::server::{build_gateway_base, finalize_gateway_app},
    serde_json::json,
    tokio::net::TcpListener,
};

struct TestServer {
    addr: SocketAddr,
    state: Arc<GatewayState>,
    _config_dir: tempfile::TempDir,
    _data_dir: tempfile::TempDir,
}

async fn start_test_server() -> TestServer {
    let config_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    moltis_config::set_config_dir(config_dir.path().to_path_buf());
    moltis_config::set_data_dir(data_dir.path().to_path_buf());

    let state = GatewayState::new(auth::resolve_auth(None, None), GatewayServices::noop());
    let methods = Arc::new(MethodRegistry::new());
    #[cfg(feature = "push-notifications")]
    let (router, app_state) = build_gateway_base(Arc::clone(&state), methods, None, None);
    #[cfg(not(feature = "push-notifications"))]
    let (router, app_state) = build_gateway_base(Arc::clone(&state), methods, None);
    let app = finalize_gateway_app(router, app_state, false);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    TestServer {
        addr,
        state,
        _config_dir: config_dir,
        _data_dir: data_dir,
    }
}

fn options(server: &TestServer) -> ConnectOptions {
    let endpoint = Endpoint::parse(&format!("http://{}", server.addr), TlsTrust::System).unwrap();
    ConnectOptions::new(
        endpoint,
        ClientIdentity::new("moltis-desktop-test", "0.0.0"),
    )
}

#[tokio::test]
async fn native_client_handshake_and_typed_calls() {
    let server = start_test_server().await;
    let conn = Connection::connect(&options(&server)).await.unwrap();
    assert_eq!(conn.hello().protocol, 4);
    assert!(
        conn.hello()
            .features
            .methods
            .iter()
            .any(|m| m == methods::CHAT_SEND)
    );
    assert!(conn.sessions_list().await.unwrap().is_empty());
}

#[tokio::test]
async fn native_client_receives_typed_chat_events() {
    let server = start_test_server().await;
    let conn = Connection::connect(&options(&server)).await.unwrap();
    let mut received = conn.events();
    conn.call(methods::SUBSCRIBE, json!({ "events": [events::CHAT] }))
        .await
        .unwrap();

    broadcast(
        &server.state,
        events::CHAT,
        json!({"runId": "r1", "sessionKey": "main", "state": "delta", "text": "Hel", "seq": 1}),
        BroadcastOpts::default(),
    )
    .await;

    let frame = tokio::time::timeout(Duration::from_secs(5), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.event, events::CHAT);
    let event: ChatEvent = serde_json::from_value(frame.payload.unwrap()).unwrap();
    assert_eq!(event.session_key, "main");
    assert_eq!(event.kind, ChatEventKind::Delta { text: "Hel".into() });
}

#[tokio::test]
async fn managed_client_connects_and_subscribes() {
    let server = start_test_server().await;
    let (client, mut updates) =
        ManagedClient::spawn(options(&server), vec![events::CHAT.to_string()]);
    loop {
        let update = tokio::time::timeout(Duration::from_secs(5), updates.recv())
            .await
            .unwrap()
            .unwrap();
        match update {
            ClientUpdate::Connected { .. } => break,
            ClientUpdate::Connecting { .. } => {},
            other => panic!("unexpected {other:?}"),
        }
    }
    broadcast(
        &server.state,
        events::CHAT,
        json!({"sessionKey": "main", "state": "thinking"}),
        BroadcastOpts::default(),
    )
    .await;
    let update = tokio::time::timeout(Duration::from_secs(5), updates.recv())
        .await
        .unwrap()
        .unwrap();
    let ClientUpdate::Event(frame) = update else {
        panic!("expected event, got {update:?}");
    };
    let event: ChatEvent = serde_json::from_value(frame.payload.unwrap()).unwrap();
    assert_eq!(event.kind, ChatEventKind::Thinking);
    client.shutdown();
}
