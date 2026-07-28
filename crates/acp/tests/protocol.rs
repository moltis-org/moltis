//! End-to-end protocol coverage over an in-memory loopback.
//!
//! A real `ClientSideConnection` drives a real `AgentSideConnection` over a
//! duplex pipe, so these exercise actual JSON-RPC framing, request/response
//! correlation, and notification ordering — no subprocess, no network.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    cell::RefCell,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use {
    agent_client_protocol::{self as acp, Agent as _},
    async_trait::async_trait,
    tokio::{
        io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf},
        sync::Notify,
        task::LocalSet,
        time::timeout,
    },
    tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt},
};

use moltis_acp::{
    AcpBackend, BackendCapabilities, EchoBackend, MoltisAgent, SessionKey, SessionSetup,
    TurnUpdates,
};

/// Every await in these tests should settle in milliseconds; the budget exists
/// to turn a hang into a failure rather than a stuck suite.
const TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------- test client

/// Client that records every `session/update` it receives, in order.
#[derive(Clone, Default)]
struct TestClient {
    updates: Rc<RefCell<Vec<acp::SessionUpdate>>>,
}

impl TestClient {
    /// Concatenates the streamed agent message chunks.
    fn agent_text(&self) -> String {
        self.updates
            .borrow()
            .iter()
            .filter_map(|update| match update {
                acp::SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                    acp::ContentBlock::Text(text) => Some(text.text.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    fn kinds(&self) -> Vec<&'static str> {
        self.updates
            .borrow()
            .iter()
            .map(|update| match update {
                acp::SessionUpdate::UserMessageChunk(_) => "user",
                acp::SessionUpdate::AgentMessageChunk(_) => "message",
                acp::SessionUpdate::AgentThoughtChunk(_) => "thought",
                acp::SessionUpdate::ToolCall(_) => "tool_call",
                acp::SessionUpdate::ToolCallUpdate(_) => "tool_call_update",
                _ => "other",
            })
            .collect()
    }
}

#[async_trait(?Send)]
impl acp::Client for TestClient {
    async fn request_permission(
        &self,
        _args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Cancelled,
        ))
    }

    async fn session_notification(&self, args: acp::SessionNotification) -> acp::Result<()> {
        self.updates.borrow_mut().push(args.update);
        Ok(())
    }
}

// -------------------------------------------------------------- test backends

/// Backend whose turn blocks until released, so a cancel can be delivered while
/// `session/prompt` is genuinely in flight.
#[derive(Default)]
struct BlockingBackend {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl AcpBackend for BlockingBackend {
    async fn create_session(&self, _setup: &SessionSetup) -> anyhow::Result<SessionKey> {
        Ok(SessionKey::namespaced("blocking"))
    }

    async fn prompt(
        &self,
        _key: &SessionKey,
        _prompt: String,
        updates: TurnUpdates,
    ) -> anyhow::Result<acp::StopReason> {
        updates.agent_message("working");
        self.started.notify_one();
        self.release.notified().await;
        // Reports success: the protocol layer is responsible for turning a
        // cancelled turn into `StopReason::Cancelled`.
        Ok(acp::StopReason::EndTurn)
    }

    async fn cancel(&self, _key: &SessionKey) -> anyhow::Result<()> {
        self.release.notify_one();
        Ok(())
    }
}

/// Backend that always fails, to check errors surface as JSON-RPC errors.
struct FailingBackend;

#[async_trait]
impl AcpBackend for FailingBackend {
    async fn create_session(&self, _setup: &SessionSetup) -> anyhow::Result<SessionKey> {
        Err(anyhow::anyhow!("no sessions today"))
    }

    async fn prompt(
        &self,
        _key: &SessionKey,
        _prompt: String,
        _updates: TurnUpdates,
    ) -> anyhow::Result<acp::StopReason> {
        Err(anyhow::anyhow!("turn exploded"))
    }

    async fn cancel(&self, _key: &SessionKey) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Backend advertising and serving `session/load`.
struct ResumableBackend;

#[async_trait]
impl AcpBackend for ResumableBackend {
    async fn create_session(&self, _setup: &SessionSetup) -> anyhow::Result<SessionKey> {
        Ok(SessionKey::namespaced("resumable"))
    }

    async fn load_session(
        &self,
        key: &SessionKey,
        _setup: &SessionSetup,
    ) -> anyhow::Result<Vec<acp::SessionUpdate>> {
        if key.as_str() == "acp:missing" {
            return Err(moltis_acp::SessionNotFound.into());
        }
        if key.as_str() == "acp:broken" {
            return Err(anyhow::anyhow!("history store unavailable"));
        }
        Ok(vec![
            acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(acp::ContentBlock::from(
                "earlier question".to_string(),
            ))),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::from(
                "earlier answer".to_string(),
            ))),
        ])
    }

    async fn prompt(
        &self,
        _key: &SessionKey,
        _prompt: String,
        _updates: TurnUpdates,
    ) -> anyhow::Result<acp::StopReason> {
        Ok(acp::StopReason::EndTurn)
    }

    async fn cancel(&self, _key: &SessionKey) -> anyhow::Result<()> {
        Ok(())
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities { load_session: true }
    }
}

/// Backend that mints keys outside the ACP namespace, and records whether
/// `load_session` was reached.
#[derive(Default)]
struct EscapingBackend {
    loaded: Arc<AtomicBool>,
}

#[async_trait]
impl AcpBackend for EscapingBackend {
    async fn create_session(&self, _setup: &SessionSetup) -> anyhow::Result<SessionKey> {
        Ok(SessionKey::new("web:escaped"))
    }

    async fn load_session(
        &self,
        _key: &SessionKey,
        _setup: &SessionSetup,
    ) -> anyhow::Result<Vec<acp::SessionUpdate>> {
        self.loaded.store(true, Ordering::SeqCst);
        Ok(Vec::new())
    }

    async fn prompt(
        &self,
        _key: &SessionKey,
        _prompt: String,
        _updates: TurnUpdates,
    ) -> anyhow::Result<acp::StopReason> {
        Ok(acp::StopReason::EndTurn)
    }

    async fn cancel(&self, _key: &SessionKey) -> anyhow::Result<()> {
        Ok(())
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities { load_session: true }
    }
}

// ------------------------------------------------------------------- plumbing

/// Duplex stream that records every byte written through it, so a test can
/// assert on exactly what the agent put on the wire.
struct Tee {
    inner: DuplexStream,
    seen: Arc<Mutex<Vec<u8>>>,
}

impl AsyncRead for Tee {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Tee {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let written = std::task::ready!(Pin::new(&mut self.inner).poll_write(cx, buf))?;
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(&buf[..written]);
        Poll::Ready(Ok(written))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// A live loopback connection. Holds the agent side alive for the duration of
/// the test: the agent keeps only a weak reference to its own connection.
struct Harness {
    client: Rc<acp::ClientSideConnection>,
    _agent: Rc<MoltisAgent>,
    _agent_conn: Rc<acp::AgentSideConnection>,
    wire: Arc<Mutex<Vec<u8>>>,
}

fn connect(backend: Arc<dyn AcpBackend>, client: TestClient) -> Harness {
    let (agent_io, client_io) = tokio::io::duplex(64 * 1024);
    let wire = Arc::new(Mutex::new(Vec::new()));
    let (agent_read, agent_write) = tokio::io::split(Tee {
        inner: agent_io,
        seen: Arc::clone(&wire),
    });
    let (client_read, client_write) = tokio::io::split(client_io);

    let agent = Rc::new(MoltisAgent::new(backend));
    let (agent_conn, agent_io_task) = acp::AgentSideConnection::new(
        Rc::clone(&agent),
        agent_write.compat_write(),
        agent_read.compat(),
        |future| {
            tokio::task::spawn_local(future);
        },
    );
    let agent_conn = Rc::new(agent_conn);
    agent.set_connection(&agent_conn);

    let (client_conn, client_io_task) = acp::ClientSideConnection::new(
        client,
        client_write.compat_write(),
        client_read.compat(),
        |future| {
            tokio::task::spawn_local(future);
        },
    );

    tokio::task::spawn_local(async move {
        let _ = agent_io_task.await;
    });
    tokio::task::spawn_local(async move {
        let _ = client_io_task.await;
    });

    Harness {
        client: Rc::new(client_conn),
        _agent: agent,
        _agent_conn: agent_conn,
        wire,
    }
}

async fn initialize(harness: &Harness) -> acp::InitializeResponse {
    timeout(
        TIMEOUT,
        harness.client.initialize(
            acp::InitializeRequest::new(acp::ProtocolVersion::LATEST)
                .client_info(acp::Implementation::new("test-client", "0.0.0").title("Test Client")),
        ),
    )
    .await
    .expect("initialize timed out")
    .expect("initialize failed")
}

async fn new_session(harness: &Harness) -> acp::SessionId {
    timeout(
        TIMEOUT,
        harness
            .client
            .new_session(acp::NewSessionRequest::new(std::env::temp_dir())),
    )
    .await
    .expect("session/new timed out")
    .expect("session/new failed")
    .session_id
}

/// Runs `body` on a `LocalSet`, since the protocol handler is `!Send`.
fn run_local<F: Future<Output = ()>>(body: F) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    LocalSet::new().block_on(&runtime, body);
}

// ---------------------------------------------------------------------- tests

#[test]
fn initialize_negotiates_version_and_reports_capabilities() {
    run_local(async {
        let harness = connect(Arc::new(EchoBackend::new()), TestClient::default());
        let response = initialize(&harness).await;

        assert_eq!(response.protocol_version, acp::ProtocolVersion::LATEST);
        // The echo backend has nothing to resume, so it must not claim it can.
        assert!(!response.agent_capabilities.load_session);
        let info = response.agent_info.expect("agent_info advertised");
        assert_eq!(info.name, "moltis");
    });
}

#[test]
fn initialize_reports_load_session_when_the_backend_supports_it() {
    run_local(async {
        let harness = connect(Arc::new(ResumableBackend), TestClient::default());
        let response = initialize(&harness).await;
        assert!(response.agent_capabilities.load_session);
    });
}

#[test]
fn authenticate_succeeds_without_credentials() {
    run_local(async {
        let harness = connect(Arc::new(EchoBackend::new()), TestClient::default());
        initialize(&harness).await;
        let result =
            timeout(
                TIMEOUT,
                harness.client.authenticate(acp::AuthenticateRequest::new(
                    acp::AuthMethodId::from("none".to_string()),
                )),
            )
            .await
            .expect("authenticate timed out");
        assert!(result.is_ok(), "local clients need no authentication");
    });
}

#[test]
fn prompt_streams_updates_before_returning_a_stop_reason() {
    run_local(async {
        let client = TestClient::default();
        let harness = connect(Arc::new(EchoBackend::new()), client.clone());
        initialize(&harness).await;
        let session_id = new_session(&harness).await;

        let response = timeout(
            TIMEOUT,
            harness
                .client
                .prompt(acp::PromptRequest::new(session_id, vec![
                    acp::ContentBlock::from("hello world".to_string()),
                ])),
        )
        .await
        .expect("prompt timed out")
        .expect("prompt failed");

        assert_eq!(response.stop_reason, acp::StopReason::EndTurn);
        // Every update must already have arrived by the time the call resolves,
        // otherwise the agent looks mute and then dumps everything at once.
        assert_eq!(client.agent_text(), "hello world");
        let kinds = client.kinds();
        assert_eq!(
            kinds.first(),
            Some(&"thought"),
            "thought precedes the reply: {kinds:?}"
        );
        assert!(kinds.iter().filter(|kind| **kind == "message").count() > 1);
    });
}

#[test]
fn cancel_resolves_the_pending_prompt_as_cancelled() {
    run_local(async {
        let backend = Arc::new(BlockingBackend::default());
        let started = Arc::clone(&backend.started);
        let harness = connect(backend, TestClient::default());
        initialize(&harness).await;
        let session_id = new_session(&harness).await;

        let client = Rc::clone(&harness.client);
        let prompt_session = session_id.clone();
        let turn = tokio::task::spawn_local(async move {
            client
                .prompt(acp::PromptRequest::new(prompt_session, vec![
                    acp::ContentBlock::from("start working".to_string()),
                ]))
                .await
        });

        // Cancel only once the turn is genuinely in flight.
        timeout(TIMEOUT, started.notified())
            .await
            .expect("turn never started");
        timeout(
            TIMEOUT,
            harness
                .client
                .cancel(acp::CancelNotification::new(session_id)),
        )
        .await
        .expect("cancel timed out")
        .expect("cancel failed");

        let response = timeout(TIMEOUT, turn)
            .await
            .expect("prompt did not resolve promptly after cancel")
            .expect("prompt task panicked")
            .expect("prompt failed");
        assert_eq!(response.stop_reason, acp::StopReason::Cancelled);
    });
}

#[test]
fn unknown_session_id_is_rejected_with_invalid_params() {
    run_local(async {
        let harness = connect(Arc::new(EchoBackend::new()), TestClient::default());
        initialize(&harness).await;

        let error = timeout(
            TIMEOUT,
            harness.client.prompt(acp::PromptRequest::new(
                acp::SessionId::from("acp:never-created".to_string()),
                vec![acp::ContentBlock::from("hello".to_string())],
            )),
        )
        .await
        .expect("prompt timed out")
        .expect_err("unknown session must be rejected");

        assert_eq!(error.code, acp::Error::invalid_params().code);
    });
}

#[test]
fn cancel_for_unknown_session_does_not_panic() {
    run_local(async {
        let harness = connect(Arc::new(EchoBackend::new()), TestClient::default());
        initialize(&harness).await;

        // Notifications carry no reply, so the check here is that the
        // connection survives and still serves the next request.
        let _ = timeout(
            TIMEOUT,
            harness
                .client
                .cancel(acp::CancelNotification::new(acp::SessionId::from(
                    "acp:never-created".to_string(),
                ))),
        )
        .await
        .expect("cancel timed out");

        let session_id = new_session(&harness).await;
        assert!(session_id.to_string().starts_with("acp:"));
    });
}

#[test]
fn backend_failures_surface_as_errors_not_panics() {
    run_local(async {
        let harness = connect(Arc::new(FailingBackend), TestClient::default());
        initialize(&harness).await;

        let error = timeout(
            TIMEOUT,
            harness
                .client
                .new_session(acp::NewSessionRequest::new(std::env::temp_dir())),
        )
        .await
        .expect("session/new timed out")
        .expect_err("failing backend must produce an error");
        assert_eq!(error.code, acp::Error::internal_error().code);
    });
}

#[test]
fn load_session_replays_history_before_responding() {
    run_local(async {
        let client = TestClient::default();
        let harness = connect(Arc::new(ResumableBackend), client.clone());
        initialize(&harness).await;
        let session_id = new_session(&harness).await;

        timeout(
            TIMEOUT,
            harness.client.load_session(acp::LoadSessionRequest::new(
                session_id,
                std::env::temp_dir(),
            )),
        )
        .await
        .expect("session/load timed out")
        .expect("session/load failed");

        assert_eq!(client.kinds(), vec!["user", "message"]);
    });
}

#[test]
fn load_session_is_method_not_found_when_unsupported() {
    run_local(async {
        let harness = connect(Arc::new(EchoBackend::new()), TestClient::default());
        initialize(&harness).await;
        let session_id = new_session(&harness).await;

        let error = timeout(
            TIMEOUT,
            harness.client.load_session(acp::LoadSessionRequest::new(
                session_id,
                std::env::temp_dir(),
            )),
        )
        .await
        .expect("session/load timed out")
        .expect_err("echo backend cannot resume");
        assert_eq!(error.code, acp::Error::method_not_found().code);
    });
}

#[test]
fn load_session_classifies_missing_and_operational_failures() {
    run_local(async {
        let harness = connect(Arc::new(ResumableBackend), TestClient::default());
        initialize(&harness).await;

        let missing = harness
            .client
            .load_session(acp::LoadSessionRequest::new(
                acp::SessionId::from("acp:missing".to_string()),
                std::env::temp_dir(),
            ))
            .await
            .expect_err("missing session should fail");
        assert_eq!(missing.code, acp::Error::invalid_params().code);

        let broken = harness
            .client
            .load_session(acp::LoadSessionRequest::new(
                acp::SessionId::from("acp:broken".to_string()),
                std::env::temp_dir(),
            ))
            .await
            .expect_err("history read failure should fail");
        assert_eq!(broken.code, acp::Error::internal_error().code);
    });
}

#[test]
fn load_session_refuses_ids_outside_the_acp_namespace() {
    run_local(async {
        let backend = Arc::new(EscapingBackend::default());
        let loaded = Arc::clone(&backend.loaded);
        let harness = connect(backend, TestClient::default());
        initialize(&harness).await;

        // A Web UI session key, named directly by the client. Nothing in the
        // registry rules it out — only the namespace does.
        let error = timeout(
            TIMEOUT,
            harness.client.load_session(acp::LoadSessionRequest::new(
                acp::SessionId::from("web:someone-elses-session".to_string()),
                std::env::temp_dir(),
            )),
        )
        .await
        .expect("session/load timed out")
        .expect_err("foreign session ids must be refused");

        assert_eq!(error.code, acp::Error::invalid_params().code);
        assert!(
            !loaded.load(Ordering::SeqCst),
            "backend must never be asked to resolve an out-of-namespace key"
        );
    });
}

#[test]
fn new_session_rejects_a_backend_key_outside_the_acp_namespace() {
    run_local(async {
        let harness = connect(Arc::new(EscapingBackend::default()), TestClient::default());
        initialize(&harness).await;

        let error = timeout(
            TIMEOUT,
            harness
                .client
                .new_session(acp::NewSessionRequest::new(std::env::temp_dir())),
        )
        .await
        .expect("session/new timed out")
        .expect_err("an out-of-namespace backend key must not reach the client");
        assert_eq!(error.code, acp::Error::internal_error().code);
    });
}

#[test]
fn sessions_are_namespaced_so_they_cannot_collide_with_other_surfaces() {
    run_local(async {
        let harness = connect(Arc::new(EchoBackend::new()), TestClient::default());
        initialize(&harness).await;
        let session_id = new_session(&harness).await;
        assert!(
            session_id.to_string().starts_with("acp:"),
            "ACP sessions must live in their own namespace: {session_id}"
        );
    });
}

/// stdout is the wire: with a subscriber installed and `#[instrument]` spans
/// firing throughout the turn, every byte the agent emits must still be
/// protocol framing.
#[test]
fn wire_carries_only_protocol_framing_while_tracing_is_active() {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::TRACE)
        .try_init();

    run_local(async {
        let harness = connect(Arc::new(EchoBackend::new()), TestClient::default());
        initialize(&harness).await;
        let session_id = new_session(&harness).await;
        timeout(
            TIMEOUT,
            harness
                .client
                .prompt(acp::PromptRequest::new(session_id, vec![
                    acp::ContentBlock::from("trace me".to_string()),
                ])),
        )
        .await
        .expect("prompt timed out")
        .expect("prompt failed");

        let bytes = harness
            .wire
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let text = String::from_utf8(bytes).expect("wire must be UTF-8");
        assert!(!text.is_empty(), "expected protocol traffic");

        let mut frames = 0;
        for line in text.split('\n').filter(|line| !line.is_empty()) {
            let frame: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("non-JSON byte on the wire: {error}: {line:?}"));
            assert_eq!(
                frame.get("jsonrpc").and_then(serde_json::Value::as_str),
                Some("2.0"),
                "every frame must be JSON-RPC: {line}"
            );
            frames += 1;
        }
        assert!(frames >= 3, "expected initialize + session/new + prompt");
    });
}
