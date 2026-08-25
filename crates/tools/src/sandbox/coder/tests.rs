#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use {
    futures::{SinkExt, StreamExt},
    mockito::Matcher,
    secrecy::Secret,
    serde_json::Value,
    tokio_tungstenite::{accept_async, tungstenite::Message},
};

use {
    super::{
        AgentLifecycle, AgentStatus, BuildStatus, CoderAgent, CoderBuild, CoderJob, CoderResource,
        CoderSandbox, CoderSandboxConfig, CoderSession, CoderWorkspace, JobStatus,
        MAX_WORKSPACE_NAME_LEN, TrackedWorkspace, command_timeout_error, complete_before,
        failed_agent_state, ready_agent, validated_coder_url, workspace_is_stopped, workspace_name,
        workspace_session,
    },
    crate::sandbox::{Sandbox, SandboxConfig, SandboxId, SandboxScope},
};

use super::transport::{
    CLIENT_MESSAGE_CAP, PTY_COLS, PTY_ROWS, PtyMarkers, PtyOutputParser, STDIN_CHUNK_BYTES,
    bootstrap_command, ctrl_c_message, eof_stdin, framed_stdin_chunk, resize_message,
    stdin_message, wrapped_command,
};

fn agent(lifecycle: Option<AgentLifecycle>, status: Option<AgentStatus>) -> CoderAgent {
    CoderAgent {
        id: "agent-1".into(),
        status,
        lifecycle_state: lifecycle,
        directory: None,
        expanded_directory: Some("/workspaces/demo".into()),
    }
}

fn workspace_with(status: BuildStatus, agents: Vec<CoderAgent>) -> CoderWorkspace {
    CoderWorkspace {
        id: "ws-1".into(),
        name: "ws".into(),
        latest_build: Some(CoderBuild {
            status: Some(status),
            job: None,
            resources: vec![CoderResource { agents }],
        }),
    }
}

fn transcript(nonce: &str, stdout: &str, stderr: &str, exit_code: i32) -> String {
    format!(
        "login noise\r\n__MOLTIS_READY_{nonce}__\r\n{stdout}\r\n__MOLTIS_EXIT_{nonce}__{exit_code}\r\n__MOLTIS_STDERR_{nonce}_BEGIN__\r\n{stderr}\r\n__MOLTIS_STDERR_{nonce}_END__\r\n"
    )
}

fn binary_payload(message: Message) -> Vec<u8> {
    match message {
        Message::Binary(bytes) => bytes.to_vec(),
        other => panic!("expected binary WebSocket message, got {other:?}"),
    }
}

fn sandbox_config(url: String) -> CoderSandboxConfig {
    CoderSandboxConfig {
        url,
        token: Secret::new("token".into()),
        organization: None,
        user: "me".into(),
        template_id: Some("template-1".into()),
        template_name: None,
        workspace_prefix: "moltis".into(),
        ttl_ms: None,
        size: None,
        template_presets: HashMap::new(),
        parameter_values: HashMap::new(),
    }
}

fn sandbox_id() -> SandboxId {
    SandboxId {
        scope: SandboxScope::Session,
        key: "session/key".into(),
    }
}

fn workspace_lookup_path(id: &SandboxId) -> String {
    format!(
        "/api/v2/users/me/workspace/{}",
        workspace_name("moltis", &id.key)
    )
}

fn tracked_session(workspace_id: &str) -> TrackedWorkspace {
    TrackedWorkspace::Ready(CoderSession {
        workspace_id: workspace_id.into(),
        workspace_name: "tracked".into(),
        agent_id: "agent-old".into(),
        workspace_dir: "/workspaces/old".into(),
    })
}

fn ready_workspace_json(id: &str, name: &str) -> String {
    serde_json::json!({
        "id": id,
        "name": name,
        "latest_build": {
            "status": "running",
            "job": {"status": "succeeded"},
            "resources": [{
                "agents": [{
                    "id": "agent-new",
                    "status": "connected",
                    "lifecycle_state": "ready",
                    "expanded_directory": "/workspaces/new"
                }]
            }]
        }
    })
    .to_string()
}

#[test]
fn client_messages_are_binary_json_and_safely_sized() {
    let resize = binary_payload(resize_message().unwrap());
    let resize_json: Value = serde_json::from_slice(&resize).unwrap();
    assert_eq!(resize_json["height"], PTY_ROWS);
    assert_eq!(resize_json["width"], PTY_COLS);

    let chunk = framed_stdin_chunk(&"x".repeat(STDIN_CHUNK_BYTES));
    let stdin = binary_payload(stdin_message(&chunk).unwrap());
    let stdin_json: Value = serde_json::from_slice(&stdin).unwrap();
    assert_eq!(stdin_json["data"].as_str().unwrap(), chunk);
    assert!(stdin.len() < CLIENT_MESSAGE_CAP);

    let ctrl_c = binary_payload(ctrl_c_message().unwrap());
    let ctrl_c_json: Value = serde_json::from_slice(&ctrl_c).unwrap();
    assert_eq!(ctrl_c_json["data"].as_str().unwrap(), "\u{3}");
    assert!(ctrl_c.len() < CLIENT_MESSAGE_CAP);
}

#[test]
fn eof_message_is_binary_and_bounded() {
    let markers = PtyMarkers::with_nonce("abc");
    let eof = eof_stdin(&markers);
    assert_eq!(eof, "__MOLTIS_EOF_abc__\n");
    let payload = binary_payload(stdin_message(&eof).unwrap());
    assert!(payload.len() < CLIENT_MESSAGE_CAP);
}

#[test]
fn bootstrap_stays_small_and_remote_watchdog_fails_closed() {
    let markers = PtyMarkers::with_nonce("abc");
    let bootstrap = bootstrap_command(&markers);
    assert!(bootstrap.len() < 1024);
    assert!(bootstrap.contains("stty raw -echo"));

    let command = wrapped_command(
        "printf hi",
        "/home/coder/project",
        &[
            ("FOO".into(), "bar baz".into()),
            ("BAD-KEY".into(), "x".into()),
        ],
        &markers,
        Duration::from_millis(1500),
    );
    assert!(command.contains("command -v timeout"));
    assert!(command.contains("timeout --signal=TERM --kill-after=2s 2s"));
    assert!(command.contains("required timeout utility is unavailable"));
    assert!(command.contains("FOO='bar baz'") || command.contains("FOO=bar\\ baz"));
    assert!(!command.contains("BAD-KEY"));
}

#[tokio::test]
async fn command_deadline_is_shared_and_reports_configured_timeout() {
    let configured = Duration::from_millis(25);
    let deadline = tokio::time::Instant::now() + configured;
    assert_eq!(complete_before(deadline, async { 7 }).await, Some(7));
    assert!(
        complete_before(deadline, tokio::time::sleep(Duration::from_secs(1)))
            .await
            .is_none()
    );
    assert!(
        command_timeout_error(configured)
            .to_string()
            .contains("configured timeout 25ms")
    );
}

#[tokio::test]
async fn cancelling_pty_script_releases_websocket_ownership() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (reader_active_tx, reader_active_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = accept_async(stream).await.unwrap();
        websocket
            .send(Message::Binary(
                b"__MOLTIS_READY_cancel__\n".to_vec().into(),
            ))
            .await
            .unwrap();
        let mut reader_active_tx = Some(reader_active_tx);
        while let Some(message) = websocket.next().await {
            match message {
                Ok(Message::Binary(bytes)) => {
                    let value: Value = serde_json::from_slice(&bytes).unwrap();
                    if value.get("data").is_some()
                        && let Some(tx) = reader_active_tx.take()
                    {
                        tx.send(()).unwrap();
                    }
                },
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {},
            }
        }
    });

    let sandbox = Arc::new(CoderSandbox::new(
        SandboxConfig::default(),
        sandbox_config(format!("http://{address}")),
    ));
    let task = tokio::spawn(async move {
        let session = CoderSession {
            workspace_id: "workspace-1".into(),
            workspace_name: "workspace".into(),
            agent_id: "agent-1".into(),
            workspace_dir: "/workspaces/demo".into(),
        };
        let markers = PtyMarkers::with_nonce("cancel");
        sandbox
            .run_pty_script(
                &session,
                "sleep 60",
                &markers,
                Duration::from_secs(60),
                1024,
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(2), reader_active_rx)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("server retained the cancelled PTY connection")
        .unwrap();
}

#[test]
fn parser_handles_every_binary_fragment_and_utf8_boundary() {
    let markers = PtyMarkers::with_nonce("abc");
    let output = transcript("abc", "hello 世界", "oops ñ", 7);
    let mut parser = PtyOutputParser::new(markers, 1024);
    let mut saw_ready = false;
    for byte in output.as_bytes() {
        saw_ready |= parser
            .feed_message(&Message::Binary(vec![*byte].into()))
            .unwrap();
    }
    assert!(saw_ready);
    let parsed = parser.finish().unwrap();
    assert_eq!(parsed.stdout, "hello 世界");
    assert_eq!(parsed.stderr, "oops ñ");
    assert_eq!(parsed.exit_code, 7);
}

#[test]
fn parser_accepts_fragmented_text_and_binary_frames() {
    let markers = PtyMarkers::with_nonce("abc");
    let output = transcript("abc", "one", "two", 0);
    let split = output.find("__MOLTIS_EXIT").unwrap() + 8;
    let mut parser = PtyOutputParser::new(markers, 1024);
    parser
        .feed_message(&Message::Text(output[..split].to_string().into()))
        .unwrap();
    parser
        .feed_message(&Message::Binary(output.as_bytes()[split..].to_vec().into()))
        .unwrap();
    let parsed = parser.finish().unwrap();
    assert_eq!(parsed.stdout, "one");
    assert_eq!(parsed.stderr, "two");
}

#[test]
fn parser_bounds_output_but_consumes_through_markers() {
    let markers = PtyMarkers::with_nonce("abc");
    let output = transcript("abc", &"o".repeat(1_000_000), &"e".repeat(1_000_000), 23);
    let mut parser = PtyOutputParser::new(markers, 64);
    parser.feed(output.as_bytes()).unwrap();
    assert!(parser.is_done());
    assert!(parser.buffered_bytes() <= 256);
    let parsed = parser.finish().unwrap();
    assert_eq!(parsed.stdout, "o".repeat(64));
    assert_eq!(parsed.stderr, "e".repeat(64));
    assert_eq!(parsed.exit_code, 23);
}

#[test]
fn parser_ignores_large_trailing_payload_after_completion() {
    let markers = PtyMarkers::with_nonce("abc");
    let mut output = transcript("abc", "ok", "", 0).into_bytes();
    output.extend(std::iter::repeat_n(b'x', 1_000_000));
    let mut parser = PtyOutputParser::new(markers, 64);
    parser.feed(&output).unwrap();
    assert!(parser.buffered_bytes() <= 256);
    assert_eq!(parser.finish().unwrap().stdout, "ok");
}

#[test]
fn parser_strips_ansi_and_errors_without_completion() {
    let markers = PtyMarkers::with_nonce("abc");
    let output = transcript("abc", "\x1b[32mgreen\x1b[0m", "", 0);
    let mut parser = PtyOutputParser::new(markers.clone(), 1024);
    parser.feed(output.as_bytes()).unwrap();
    assert_eq!(parser.finish().unwrap().stdout, "green");

    let mut incomplete = PtyOutputParser::new(markers, 10);
    incomplete.feed(b"noise only").unwrap();
    assert!(incomplete.finish().is_err());
}

#[test]
fn workspace_names_always_have_sha_suffix_and_avoid_normalized_collisions() {
    let short = workspace_name("Moltis!", "abc");
    let normalized_a = workspace_name("moltis", "session_key");
    let normalized_b = workspace_name("moltis", "session-key");
    assert!(short.starts_with("moltis-abc-"), "{short}");
    assert_eq!(short.rsplit('-').next().unwrap().len(), 10);
    assert_ne!(normalized_a, normalized_b);
    for name in [short, normalized_a, normalized_b] {
        assert!(name.len() <= MAX_WORKSPACE_NAME_LEN, "{name}");
        assert!(name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        }));
    }
}

#[test]
fn workspace_names_are_deterministic_and_handle_empty_slugs() {
    let first = workspace_name("!!!", "///");
    assert!(first.starts_with("moltis-"));
    assert_eq!(first, workspace_name("!!!", "///"));
    assert_ne!(first, workspace_name("???", "///"));
}

#[test]
fn readiness_requires_connected_and_explicitly_ready() {
    for lifecycle in [
        None,
        Some(AgentLifecycle::Created),
        Some(AgentLifecycle::Starting),
    ] {
        let workspace = workspace_with(BuildStatus::Running, vec![agent(
            lifecycle,
            Some(AgentStatus::Connected),
        )]);
        assert!(ready_agent(&workspace).is_none());
        assert!(
            workspace_session("ws", "name", &workspace)
                .unwrap()
                .is_none()
        );
    }
    let disconnected = workspace_with(BuildStatus::Running, vec![agent(
        Some(AgentLifecycle::Ready),
        Some(AgentStatus::Disconnected),
    )]);
    assert!(ready_agent(&disconnected).is_none());
}

#[test]
fn terminal_lifecycle_and_build_states_fail_fast() {
    for lifecycle in [AgentLifecycle::StartTimeout, AgentLifecycle::StartError] {
        let workspace = workspace_with(BuildStatus::Running, vec![agent(
            Some(lifecycle),
            Some(AgentStatus::Connected),
        )]);
        assert_eq!(failed_agent_state(&workspace), Some(lifecycle));
        assert!(workspace_session("ws", "name", &workspace).is_err());
    }
    let mut failed = workspace_with(BuildStatus::Failed, Vec::new());
    failed.latest_build.as_mut().unwrap().job = Some(CoderJob {
        status: Some(JobStatus::Failed),
        error: Some("boom".into()),
    });
    assert!(workspace_session("ws", "name", &failed).is_err());
}

#[test]
fn stopped_workspace_is_not_usable() {
    let workspace = workspace_with(BuildStatus::Stopped, vec![agent(
        Some(AgentLifecycle::Ready),
        Some(AgentStatus::Connected),
    )]);
    assert!(workspace_is_stopped(&workspace));
}

#[test]
fn validates_coder_urls_and_preserves_base_path() {
    let https = validated_coder_url("https://coder.example.com/base").unwrap();
    assert_eq!(https.path(), "/base");
    assert!(validated_coder_url("http://localhost:3000").is_ok());
    assert!(validated_coder_url("http://127.0.0.1:3000").is_ok());
    assert!(validated_coder_url("http://[::1]:3000").is_ok());
    assert!(validated_coder_url("http://coder.example.com").is_err());
    assert!(validated_coder_url("ftp://coder.example.com").is_err());
    assert!(validated_coder_url("https://user@coder.example.com").is_err());
    assert!(validated_coder_url("https://@coder.example.com").is_err());
    assert!(validated_coder_url("https://coder.example.com?x=1").is_err());
    assert!(validated_coder_url("https://coder.example.com/#frag").is_err());
}

#[tokio::test]
async fn ensure_adopts_deterministic_workspace_before_create() {
    let mut server = mockito::Server::new_async().await;
    let id = sandbox_id();
    let name = workspace_name("moltis", &id.key);
    let lookup_path = workspace_lookup_path(&id);
    let lookup = server
        .mock("GET", lookup_path.as_str())
        .with_status(200)
        .with_body(ready_workspace_json("ws-adopted", &name))
        .create_async()
        .await;
    let ready = server
        .mock("GET", "/api/v2/workspaces/ws-adopted")
        .with_status(200)
        .with_body(ready_workspace_json("ws-adopted", &name))
        .create_async()
        .await;
    let sandbox = CoderSandbox::new(SandboxConfig::default(), sandbox_config(server.url()));

    sandbox.ensure_ready(&id, None).await.unwrap();

    lookup.assert_async().await;
    ready.assert_async().await;
    assert_eq!(
        sandbox
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id.key)
            .and_then(TrackedWorkspace::workspace_id),
        Some("ws-adopted")
    );
}

#[tokio::test]
async fn malformed_create_response_is_adopted_on_retry() {
    let mut server = mockito::Server::new_async().await;
    let id = sandbox_id();
    let name = workspace_name("moltis", &id.key);
    let lookup_path = workspace_lookup_path(&id);
    let lookup_missing = server
        .mock("GET", lookup_path.as_str())
        .with_status(404)
        .create_async()
        .await;
    let template = server
        .mock("GET", "/api/v2/templates/template-1")
        .with_status(200)
        .with_body(r#"{"id":"template-1","name":"dev","active_version_id":"v1"}"#)
        .create_async()
        .await;
    let create = server
        .mock("POST", "/api/v2/users/me/workspaces")
        .with_status(201)
        .with_body("not json")
        .create_async()
        .await;
    let sandbox = CoderSandbox::new(SandboxConfig::default(), sandbox_config(server.url()));

    assert!(sandbox.ensure_ready(&id, None).await.is_err());
    assert!(matches!(
        sandbox
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id.key),
        Some(TrackedWorkspace::Pending { workspace_name }) if workspace_name == &name
    ));
    lookup_missing.assert_async().await;
    template.assert_async().await;
    create.assert_async().await;
    drop(lookup_missing);
    drop(template);
    drop(create);

    let lookup_existing = server
        .mock("GET", lookup_path.as_str())
        .with_status(200)
        .with_body(ready_workspace_json("ws-recovered", &name))
        .create_async()
        .await;
    let ready = server
        .mock("GET", "/api/v2/workspaces/ws-recovered")
        .with_status(200)
        .with_body(ready_workspace_json("ws-recovered", &name))
        .create_async()
        .await;
    sandbox.ensure_ready(&id, None).await.unwrap();

    lookup_existing.assert_async().await;
    ready.assert_async().await;
    assert_eq!(
        sandbox
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id.key)
            .and_then(TrackedWorkspace::workspace_id),
        Some("ws-recovered")
    );
}

#[tokio::test]
async fn failed_startup_cleanup_retains_provisional_workspace() {
    let mut server = mockito::Server::new_async().await;
    let id = sandbox_id();
    let _lookup = server
        .mock("GET", workspace_lookup_path(&id).as_str())
        .with_status(404)
        .create_async()
        .await;
    let _template = server
        .mock("GET", "/api/v2/templates/template-1")
        .with_status(200)
        .with_body(r#"{"id":"template-1","name":"dev","active_version_id":"v1"}"#)
        .create_async()
        .await;
    let _create = server
        .mock("POST", "/api/v2/users/me/workspaces")
        .match_header("coder-session-token", "token")
        .with_status(200)
        .with_body(r#"{"id":"ws-1","name":"created","latest_build":null}"#)
        .create_async()
        .await;
    let _poll = server
        .mock("GET", "/api/v2/workspaces/ws-1")
        .with_status(503)
        .with_body("unavailable")
        .create_async()
        .await;
    let _delete = server
        .mock("POST", "/api/v2/workspaces/ws-1/builds")
        .match_body(Matcher::PartialJson(
            serde_json::json!({"transition": "delete"}),
        ))
        .with_status(500)
        .with_body("delete failed")
        .create_async()
        .await;
    let sandbox = CoderSandbox::new(SandboxConfig::default(), sandbox_config(server.url()));
    assert!(sandbox.ensure_ready(&id, None).await.is_err());
    let active = sandbox
        .active
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(matches!(
        active.get(&id.key),
        Some(TrackedWorkspace::Provisional { workspace_id, .. }) if workspace_id == "ws-1"
    ));
}

#[tokio::test]
async fn cancellation_after_create_keeps_workspace_tracked() {
    let mut server = mockito::Server::new_async().await;
    let id = sandbox_id();
    let _lookup = server
        .mock("GET", workspace_lookup_path(&id).as_str())
        .with_status(404)
        .create_async()
        .await;
    let _template = server
        .mock("GET", "/api/v2/templates/template-1")
        .with_status(200)
        .with_body(r#"{"id":"template-1","name":"dev","active_version_id":"v1"}"#)
        .create_async()
        .await;
    let _create = server
        .mock("POST", "/api/v2/users/me/workspaces")
        .with_status(200)
        .with_body(r#"{"id":"ws-cancel","name":"cancelled","latest_build":null}"#)
        .create_async()
        .await;
    let _poll = server
        .mock("GET", "/api/v2/workspaces/ws-cancel")
        .with_status(200)
        .with_body(
            serde_json::json!({
                "id": "ws-cancel",
                "name": "cancelled",
                "latest_build": {
                    "status": "starting",
                    "resources": [{
                        "agents": [{
                            "id": "agent-starting",
                            "status": "connected",
                            "lifecycle_state": "starting"
                        }]
                    }]
                }
            })
            .to_string(),
        )
        .create_async()
        .await;
    let sandbox = Arc::new(CoderSandbox::new(
        SandboxConfig::default(),
        sandbox_config(server.url()),
    ));
    let task_sandbox = Arc::clone(&sandbox);
    let task_id = id.clone();
    let task = tokio::spawn(async move { task_sandbox.ensure_ready(&task_id, None).await });

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                sandbox
                    .active
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&id.key),
                Some(TrackedWorkspace::Provisional { workspace_id, .. })
                    if workspace_id == "ws-cancel"
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;

    assert!(matches!(
        sandbox
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id.key),
        Some(TrackedWorkspace::Provisional { workspace_id, .. }) if workspace_id == "ws-cancel"
    ));
}

#[tokio::test]
async fn cleanup_removes_only_after_confirmed_delete() {
    let mut server = mockito::Server::new_async().await;
    let sandbox = CoderSandbox::new(SandboxConfig::default(), sandbox_config(server.url()));
    let id = sandbox_id();
    sandbox
        .active
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(id.key.clone(), tracked_session("ws-1"));

    let accepted_then_failed = server
        .mock("POST", "/api/v2/workspaces/ws-1/builds")
        .match_body(Matcher::PartialJson(
            serde_json::json!({"transition": "delete"}),
        ))
        .with_status(201)
        .with_body(r#"{"status":"deleting","job":{"status":"running"}}"#)
        .create_async()
        .await;
    let failed_confirmation = server
        .mock("GET", "/api/v2/workspaces/ws-1")
        .with_status(200)
        .with_body(
            r#"{"id":"ws-1","name":"tracked","latest_build":{"status":"failed","job":{"status":"failed","error":"delete failed"}}}"#,
        )
        .create_async()
        .await;
    assert!(sandbox.cleanup(&id).await.is_err());
    assert!(
        sandbox
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&id.key)
    );
    accepted_then_failed.assert_async().await;
    failed_confirmation.assert_async().await;
    drop(accepted_then_failed);
    drop(failed_confirmation);

    let accepted = server
        .mock("POST", "/api/v2/workspaces/ws-1/builds")
        .match_body(Matcher::PartialJson(
            serde_json::json!({"transition": "delete"}),
        ))
        .with_status(201)
        .with_body(r#"{"status":"deleting","job":{"status":"running"}}"#)
        .create_async()
        .await;
    let confirmed = server
        .mock("GET", "/api/v2/workspaces/ws-1")
        .with_status(404)
        .create_async()
        .await;
    sandbox.cleanup(&id).await.unwrap();
    accepted.assert_async().await;
    confirmed.assert_async().await;
    assert!(
        !sandbox
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&id.key)
    );
}

#[tokio::test]
async fn delete_confirmation_times_out_while_workspace_exists() {
    let mut server = mockito::Server::new_async().await;
    let pending = server
        .mock("GET", "/api/v2/workspaces/ws-1")
        .expect_at_least(1)
        .with_status(200)
        .with_body(
            r#"{"id":"ws-1","name":"tracked","latest_build":{"status":"deleting","job":{"status":"running"}}}"#,
        )
        .create_async()
        .await;
    let sandbox = CoderSandbox::new(SandboxConfig::default(), sandbox_config(server.url()));

    let error = sandbox
        .wait_for_deleted_workspace("ws-1", Duration::from_secs(2), Duration::from_millis(10))
        .await
        .unwrap_err();

    pending.assert_async().await;
    assert!(error.to_string().contains("within 2s"));
}

#[tokio::test]
async fn successful_cleanup_retires_creation_semaphore() {
    let sandbox = CoderSandbox::new(
        SandboxConfig::default(),
        sandbox_config("https://coder.example.com".into()),
    );
    let id = sandbox_id();
    drop(sandbox.creation_permit(&id).await);
    assert!(sandbox.creation_permits.read().await.contains_key(&id.key));

    sandbox.cleanup(&id).await.unwrap();

    assert!(!sandbox.creation_permits.read().await.contains_key(&id.key));
}

#[tokio::test]
async fn waiting_ensure_keeps_cleanup_semaphore_from_being_replaced() {
    let sandbox = Arc::new(CoderSandbox::new(
        SandboxConfig::default(),
        sandbox_config("https://coder.example.com".into()),
    ));
    let id = sandbox_id();
    let cleanup_permit = sandbox.creation_permit(&id).await;
    let cleanup_guard = cleanup_permit.acquire().await.unwrap();
    let waiting_permit = sandbox.creation_permit(&id).await;
    assert!(Arc::ptr_eq(&cleanup_permit, &waiting_permit));
    let waiter = tokio::spawn(async move {
        let _guard = waiting_permit.acquire().await.unwrap();
    });

    sandbox
        .retire_creation_permit(&id, &cleanup_permit, cleanup_guard)
        .await;

    let current = sandbox
        .creation_permits
        .read()
        .await
        .get(&id.key)
        .cloned()
        .unwrap();
    assert!(Arc::ptr_eq(&cleanup_permit, &current));
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap();
    drop(current);
    drop(cleanup_permit);
    sandbox.cleanup(&id).await.unwrap();
    assert!(!sandbox.creation_permits.read().await.contains_key(&id.key));
}

#[tokio::test]
async fn transient_revalidation_error_retains_cached_workspace() {
    let mut server = mockito::Server::new_async().await;
    let _get = server
        .mock("GET", "/api/v2/workspaces/ws-old")
        .with_status(503)
        .with_body("temporary")
        .create_async()
        .await;
    let sandbox = CoderSandbox::new(SandboxConfig::default(), sandbox_config(server.url()));
    let id = sandbox_id();
    sandbox
        .active
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(id.key.clone(), tracked_session("ws-old"));
    assert!(sandbox.ensure_ready(&id, None).await.is_err());
    assert_eq!(
        sandbox
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id.key)
            .and_then(TrackedWorkspace::workspace_id),
        Some("ws-old")
    );
}

#[tokio::test]
async fn stopped_cached_workspace_is_started_before_becoming_ready() {
    let mut server = mockito::Server::new_async().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let response_calls = Arc::clone(&calls);
    let workspace = server
        .mock("GET", "/api/v2/workspaces/ws-old")
        .expect(2)
        .with_status(200)
        .with_body_from_request(move |_| {
            if response_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                serde_json::json!({
                    "id": "ws-old",
                    "name": "tracked",
                    "latest_build": {
                        "status": "stopped",
                        "resources": [{
                            "agents": [{
                                "id": "stale-agent",
                                "status": "connected",
                                "lifecycle_state": "ready"
                            }]
                        }]
                    }
                })
                .to_string()
                .into_bytes()
            } else {
                ready_workspace_json("ws-old", "tracked").into_bytes()
            }
        })
        .create_async()
        .await;
    let start = server
        .mock("POST", "/api/v2/workspaces/ws-old/builds")
        .match_body(Matcher::PartialJson(
            serde_json::json!({"transition": "start"}),
        ))
        .with_status(201)
        .create_async()
        .await;
    let sandbox = CoderSandbox::new(SandboxConfig::default(), sandbox_config(server.url()));
    let id = sandbox_id();
    sandbox
        .active
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(id.key.clone(), tracked_session("ws-old"));

    sandbox.ensure_ready(&id, None).await.unwrap();

    workspace.assert_async().await;
    start.assert_async().await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn confirmed_404_recreates_cached_workspace() {
    let mut server = mockito::Server::new_async().await;
    let id = sandbox_id();
    let _missing = server
        .mock("GET", "/api/v2/workspaces/ws-old")
        .with_status(404)
        .create_async()
        .await;
    let _lookup = server
        .mock("GET", workspace_lookup_path(&id).as_str())
        .with_status(404)
        .create_async()
        .await;
    let _template = server
        .mock("GET", "/api/v2/templates/template-1")
        .with_status(200)
        .with_body(r#"{"id":"template-1","name":"dev","active_version_id":"v1"}"#)
        .create_async()
        .await;
    let _create = server
        .mock("POST", "/api/v2/users/me/workspaces")
        .with_status(200)
        .with_body(r#"{"id":"ws-new","name":"new","latest_build":null}"#)
        .create_async()
        .await;
    let _ready = server
        .mock("GET", "/api/v2/workspaces/ws-new")
        .with_status(200)
        .with_body(ready_workspace_json("ws-new", "new"))
        .create_async()
        .await;
    let sandbox = CoderSandbox::new(SandboxConfig::default(), sandbox_config(server.url()));
    sandbox
        .active
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(id.key.clone(), tracked_session("ws-old"));
    sandbox.ensure_ready(&id, None).await.unwrap();
    assert_eq!(
        sandbox
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id.key)
            .and_then(TrackedWorkspace::workspace_id),
        Some("ws-new")
    );
}

#[test]
fn backend_properties_and_working_directory_translation() {
    let sandbox = CoderSandbox::new(
        SandboxConfig::default(),
        sandbox_config("https://coder.example.com".into()),
    );
    assert_eq!(sandbox.backend_name(), "coder");
    assert!(sandbox.is_isolated());
    assert!(sandbox.provides_fs_isolation());
    assert_eq!(
        CoderSandbox::translate_working_dir(Some("/home/sandbox/proj"), "/home/coder"),
        "/home/coder/proj"
    );
}
