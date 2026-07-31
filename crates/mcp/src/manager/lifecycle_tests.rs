use std::sync::{Arc, Barrier};

use super::*;

enum Mutation {
    Remove,
    Update,
}

async fn assert_start_is_invalidated(mutation: Mutation) {
    let mut server = mockito::Server::new_async().await;
    let initialize_started = Arc::new(Barrier::new(2));
    let release_initialize = Arc::new(Barrier::new(2));
    let started = Arc::clone(&initialize_started);
    let release = Arc::clone(&release_initialize);
    let _initialize = server
        .mock("POST", "/mcp")
        .match_body(mockito::Matcher::Regex("initialize".into()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |writer| {
            started.wait();
            release.wait();
            writer.write_all(br#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}"#)
        })
        .create_async()
        .await;
    let config = McpServerConfig {
        transport: TransportType::StreamableHttp,
        url: Some(secrecy::Secret::new(format!("{}/mcp", server.url()))),
        ..Default::default()
    };
    let storage = tempfile::tempdir().unwrap();
    let mut registry = McpRegistry::load(&storage.path().join("mcp.json")).unwrap();
    registry.servers.insert("race".into(), config.clone());
    let manager = Arc::new(McpManager::new(registry));
    let starting_manager = Arc::clone(&manager);
    let start = tokio::spawn(async move { starting_manager.start_server("race", &config).await });

    tokio::task::spawn_blocking(move || initialize_started.wait())
        .await
        .unwrap();
    match mutation {
        Mutation::Remove => assert!(manager.remove_server("race").await.unwrap()),
        Mutation::Update => {
            manager
                .update_server("race", McpServerConfig {
                    command: "replacement".into(),
                    ..Default::default()
                })
                .await
                .unwrap();
        },
    }
    tokio::task::spawn_blocking(move || release_initialize.wait())
        .await
        .unwrap();

    assert!(start.await.unwrap().is_err());
    let inner = manager.inner.read().await;
    assert!(!inner.clients.contains_key("race"));
    assert!(!inner.tools.contains_key("race"));
}

#[tokio::test]
async fn remove_invalidates_in_progress_start() {
    assert_start_is_invalidated(Mutation::Remove).await;
}

#[tokio::test]
async fn update_invalidates_in_progress_start() {
    assert_start_is_invalidated(Mutation::Update).await;
}
