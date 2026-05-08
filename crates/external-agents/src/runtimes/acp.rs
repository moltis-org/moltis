use std::{
    collections::HashMap,
    path::PathBuf,
    pin::Pin,
    process::Stdio,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};

use {
    agent_client_protocol::{self as acp, Agent as _},
    async_trait::async_trait,
    futures::{Stream, stream},
    serde_json::Value,
    tokio::{
        io::{AsyncBufReadExt, BufReader},
        process::{ChildStderr, Command},
        sync::{mpsc, oneshot},
    },
    tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt},
};

use crate::{
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{
        AcpPermissionHandler, AcpPermissionOption, AcpPermissionOptionKind, AcpPermissionRequest,
        AgentTransportKind, ContextSnapshot, ExternalAgentEvent, ExternalAgentSpec,
        ExternalAgentStatus,
    },
};

/// Transport for ACP (Agent Client Protocol) agents over JSON-RPC stdio.
pub struct AcpTransport {
    binary: String,
    permission_handler: Option<Arc<dyn AcpPermissionHandler>>,
}

impl AcpTransport {
    #[must_use]
    pub fn new(binary: String) -> Self {
        Self {
            binary,
            permission_handler: None,
        }
    }

    #[must_use]
    pub fn with_permission_handler(
        mut self,
        permission_handler: Arc<dyn AcpPermissionHandler>,
    ) -> Self {
        self.permission_handler = Some(permission_handler);
        self
    }
}

#[async_trait]
impl ExternalAgentTransport for AcpTransport {
    fn name(&self) -> &str {
        "acp"
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    async fn is_available(&self) -> bool {
        which::which(&self.binary).is_ok()
    }

    fn supported_kinds(&self) -> &[AgentTransportKind] {
        &[AgentTransportKind::Acp]
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, spec)))]
    async fn start_session(
        &self,
        spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
        let binary = spec.binary.clone().unwrap_or_else(|| self.binary.clone());
        Ok(Box::new(
            AcpSession::start(binary, spec.clone(), self.permission_handler.clone()).await?,
        ))
    }
}

struct AcpSession {
    command_tx: mpsc::UnboundedSender<SessionCommand>,
    worker: Mutex<Option<JoinHandle<()>>>,
    session_id: String,
    status: Arc<Mutex<ExternalAgentStatus>>,
}

enum SessionCommand {
    RunTurn {
        prompt: String,
        response_tx: oneshot::Sender<anyhow::Result<Vec<ExternalAgentEvent>>>,
    },
    Stop {
        response_tx: oneshot::Sender<anyhow::Result<()>>,
    },
}

#[derive(Default)]
struct AcpClientState {
    session_id: Option<String>,
    events: Vec<ExternalAgentEvent>,
    terminals: HashMap<String, AcpTerminalState>,
    next_terminal_id: u64,
}

#[derive(Default)]
struct AcpTerminalState {
    output: String,
    truncated: bool,
    exit_status: Option<acp::TerminalExitStatus>,
}

#[derive(Clone)]
struct AcpClient {
    state: Arc<Mutex<AcpClientState>>,
    moltis_session_key: Option<String>,
    permission_handler: Option<Arc<dyn AcpPermissionHandler>>,
}

impl AcpClient {
    fn new(
        state: Arc<Mutex<AcpClientState>>,
        moltis_session_key: Option<String>,
        permission_handler: Option<Arc<dyn AcpPermissionHandler>>,
    ) -> Self {
        Self {
            state,
            moltis_session_key,
            permission_handler,
        }
    }

    fn set_session_id(&self, session_id: String) {
        if let Ok(mut state) = self.state.lock() {
            state.session_id = Some(session_id);
        }
    }

    fn clear_events(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.events.clear();
        }
    }

    fn push_event(&self, event: ExternalAgentEvent) {
        if let Ok(mut state) = self.state.lock() {
            state.events.push(event);
        }
    }

    fn take_events(&self) -> Vec<ExternalAgentEvent> {
        match self.state.lock() {
            Ok(mut state) => std::mem::take(&mut state.events),
            Err(_) => vec![ExternalAgentEvent::Error(
                "ACP client state lock poisoned".to_string(),
            )],
        }
    }

    fn validate_session_id(&self, session_id: &acp::SessionId) -> acp::Result<()> {
        let expected = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.session_id.clone());
        match expected {
            Some(expected) if expected == session_id.to_string() => Ok(()),
            Some(expected) => Err(acp::Error::invalid_params().data(format!(
                "unknown session_id {}, expected {expected}",
                session_id
            ))),
            None => Err(acp::Error::internal_error().data("ACP session not initialized")),
        }
    }

    fn next_terminal_id(&self) -> acp::Result<String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| acp::Error::internal_error().data("ACP client state lock poisoned"))?;
        state.next_terminal_id = state.next_terminal_id.saturating_add(1);
        Ok(format!("terminal-{}", state.next_terminal_id))
    }
}

#[async_trait(?Send)]
impl acp::Client for AcpClient {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        if let Some(handler) = &self.permission_handler {
            let request = AcpPermissionRequest {
                moltis_session_key: self.moltis_session_key.clone(),
                acp_session_id: args.session_id.to_string(),
                tool_call: tool_call_update_summary(&args.tool_call),
                options: args
                    .options
                    .iter()
                    .map(|option| AcpPermissionOption {
                        id: option.option_id.to_string(),
                        name: option.name.clone(),
                        kind: permission_option_kind(option.kind),
                    })
                    .collect(),
            };
            let selected = handler.select_option(request).await.map_err(|error| {
                acp::Error::internal_error().data(format!("ACP permission bridge failed: {error}"))
            })?;
            return Ok(acp::RequestPermissionResponse::new(match selected {
                Some(option_id) => acp::RequestPermissionOutcome::Selected(
                    acp::SelectedPermissionOutcome::new(option_id),
                ),
                None => acp::RequestPermissionOutcome::Cancelled,
            }));
        }

        let selected = args
            .options
            .iter()
            .find(|option| matches!(option.kind, acp::PermissionOptionKind::AllowOnce))
            .or_else(|| args.options.first());
        let Some(option) = selected else {
            return Ok(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Cancelled,
            ));
        };
        self.push_event(ExternalAgentEvent::ThinkingDelta(format!(
            "ACP permission selected: {}",
            option.name
        )));
        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                option.option_id.clone(),
            )),
        ))
    }

    async fn write_text_file(
        &self,
        args: acp::WriteTextFileRequest,
    ) -> acp::Result<acp::WriteTextFileResponse> {
        tokio::fs::write(&args.path, args.content)
            .await
            .map_err(acp_error_from_io)?;
        Ok(acp::WriteTextFileResponse::new())
    }

    async fn read_text_file(
        &self,
        args: acp::ReadTextFileRequest,
    ) -> acp::Result<acp::ReadTextFileResponse> {
        let content = tokio::fs::read_to_string(&args.path)
            .await
            .map_err(acp_error_from_io)?;
        Ok(acp::ReadTextFileResponse::new(slice_lines(
            &content, args.line, args.limit,
        )))
    }

    async fn create_terminal(
        &self,
        args: acp::CreateTerminalRequest,
    ) -> acp::Result<acp::CreateTerminalResponse> {
        self.validate_session_id(&args.session_id)?;
        let terminal_id = self.next_terminal_id()?;
        let output_byte_limit = args.output_byte_limit.unwrap_or(128 * 1024) as usize;
        let mut command = Command::new(&args.command);
        command.args(&args.args);
        if let Some(cwd) = &args.cwd {
            command.current_dir(cwd);
        }
        command.envs(args.env.iter().map(|env| (&env.name, &env.value)));
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.kill_on_drop(true);

        let result = tokio::time::timeout(Duration::from_secs(300), command.output()).await;
        let terminal_state = match result {
            Ok(Ok(output)) => {
                let mut text = String::from_utf8_lossy(&output.stdout).to_string();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                let truncated = text.len() > output_byte_limit;
                if truncated {
                    let keep_from = text.floor_char_boundary(text.len() - output_byte_limit);
                    text = text[keep_from..].to_string();
                }
                AcpTerminalState {
                    output: text,
                    truncated,
                    exit_status: Some(
                        acp::TerminalExitStatus::new().exit_code(
                            output
                                .status
                                .code()
                                .and_then(|code| u32::try_from(code).ok()),
                        ),
                    ),
                }
            },
            Ok(Err(error)) => AcpTerminalState {
                output: format!("failed to run terminal command: {error}"),
                truncated: false,
                exit_status: Some(acp::TerminalExitStatus::new().exit_code(1)),
            },
            Err(_) => AcpTerminalState {
                output: "terminal command timed out".to_string(),
                truncated: false,
                exit_status: Some(acp::TerminalExitStatus::new().exit_code(124)),
            },
        };
        self.state
            .lock()
            .map_err(|_| acp::Error::internal_error().data("ACP client state lock poisoned"))?
            .terminals
            .insert(terminal_id.clone(), terminal_state);
        Ok(acp::CreateTerminalResponse::new(terminal_id))
    }

    async fn terminal_output(
        &self,
        args: acp::TerminalOutputRequest,
    ) -> acp::Result<acp::TerminalOutputResponse> {
        self.validate_session_id(&args.session_id)?;
        let state = self
            .state
            .lock()
            .map_err(|_| acp::Error::internal_error().data("ACP client state lock poisoned"))?;
        let terminal = state
            .terminals
            .get(&args.terminal_id.to_string())
            .ok_or_else(|| acp::Error::invalid_params().data("unknown ACP terminal id"))?;
        Ok(
            acp::TerminalOutputResponse::new(terminal.output.clone(), terminal.truncated)
                .exit_status(terminal.exit_status.clone()),
        )
    }

    async fn release_terminal(
        &self,
        args: acp::ReleaseTerminalRequest,
    ) -> acp::Result<acp::ReleaseTerminalResponse> {
        self.validate_session_id(&args.session_id)?;
        self.state
            .lock()
            .map_err(|_| acp::Error::internal_error().data("ACP client state lock poisoned"))?
            .terminals
            .remove(&args.terminal_id.to_string());
        Ok(acp::ReleaseTerminalResponse::new())
    }

    async fn wait_for_terminal_exit(
        &self,
        args: acp::WaitForTerminalExitRequest,
    ) -> acp::Result<acp::WaitForTerminalExitResponse> {
        self.validate_session_id(&args.session_id)?;
        let state = self
            .state
            .lock()
            .map_err(|_| acp::Error::internal_error().data("ACP client state lock poisoned"))?;
        let terminal = state
            .terminals
            .get(&args.terminal_id.to_string())
            .ok_or_else(|| acp::Error::invalid_params().data("unknown ACP terminal id"))?;
        Ok(acp::WaitForTerminalExitResponse::new(
            terminal.exit_status.clone().unwrap_or_default(),
        ))
    }

    async fn kill_terminal(
        &self,
        args: acp::KillTerminalRequest,
    ) -> acp::Result<acp::KillTerminalResponse> {
        self.validate_session_id(&args.session_id)?;
        if let Some(terminal) = self
            .state
            .lock()
            .map_err(|_| acp::Error::internal_error().data("ACP client state lock poisoned"))?
            .terminals
            .get_mut(&args.terminal_id.to_string())
        {
            terminal.exit_status = Some(acp::TerminalExitStatus::new().exit_code(130));
        }
        Ok(acp::KillTerminalResponse::new())
    }

    async fn session_notification(&self, args: acp::SessionNotification) -> acp::Result<()> {
        match &args.update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => {
                self.push_event(ExternalAgentEvent::TextDelta(content_chunk_summary(chunk)));
            },
            acp::SessionUpdate::AgentThoughtChunk(chunk) => {
                self.push_event(ExternalAgentEvent::ThinkingDelta(content_chunk_summary(
                    chunk,
                )));
            },
            acp::SessionUpdate::ToolCall(tool_call) => {
                self.push_event(ExternalAgentEvent::ToolCallStart {
                    id: tool_call.tool_call_id.to_string(),
                    name: tool_call.title.clone(),
                    arguments: serde_json::to_string(&tool_call).unwrap_or_default(),
                });
            },
            acp::SessionUpdate::ToolCallUpdate(update) => {
                self.push_event(ExternalAgentEvent::ThinkingDelta(tool_call_update_summary(
                    update,
                )));
            },
            acp::SessionUpdate::SessionInfoUpdate(update) => {
                self.push_event(ExternalAgentEvent::ThinkingDelta(session_info_summary(
                    update,
                )));
            },
            other => {
                self.push_event(ExternalAgentEvent::ThinkingDelta(format!(
                    "ACP update: {}",
                    session_update_kind(other)
                )));
            },
        }
        Ok(())
    }

    async fn ext_method(&self, _args: acp::ExtRequest) -> acp::Result<acp::ExtResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn ext_notification(&self, _args: acp::ExtNotification) -> acp::Result<()> {
        Ok(())
    }
}

impl AcpSession {
    async fn start(
        binary: String,
        spec: ExternalAgentSpec,
        permission_handler: Option<Arc<dyn AcpPermissionHandler>>,
    ) -> anyhow::Result<Self> {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (startup_tx, startup_rx) = oneshot::channel();
        let status = Arc::new(Mutex::new(ExternalAgentStatus::Starting));
        let thread_status = Arc::clone(&status);
        let worker = std::thread::Builder::new()
            .name("moltis-acp-session".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = startup_tx.send(Err(anyhow::anyhow!(error)));
                        return;
                    },
                };
                let local_set = tokio::task::LocalSet::new();
                runtime.block_on(local_set.run_until(run_acp_controller(
                    binary,
                    spec,
                    permission_handler,
                    command_rx,
                    startup_tx,
                    thread_status,
                )));
            })?;

        let session_id = startup_rx
            .await
            .map_err(|_| anyhow::anyhow!("ACP worker failed during startup"))??;
        Ok(Self {
            command_tx,
            worker: Mutex::new(Some(worker)),
            session_id,
            status,
        })
    }
}

#[async_trait]
impl ExternalAgentSession for AcpSession {
    fn external_session_id(&self) -> Option<&str> {
        Some(&self.session_id)
    }

    async fn send_prompt(
        &mut self,
        prompt: &str,
        _context: Option<&ContextSnapshot>,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = ExternalAgentEvent> + Send>>> {
        set_status(&self.status, ExternalAgentStatus::Running);
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(SessionCommand::RunTurn {
                prompt: prompt.to_string(),
                response_tx,
            })
            .map_err(|_| anyhow::anyhow!("ACP session controller stopped"))?;
        let events = response_rx
            .await
            .map_err(|_| anyhow::anyhow!("ACP session controller stopped"))??;
        set_status(&self.status, ExternalAgentStatus::Idle);
        Ok(Box::pin(stream::iter(events)))
    }

    async fn is_alive(&self) -> bool {
        self.status() != ExternalAgentStatus::Stopped
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        set_status(&self.status, ExternalAgentStatus::Stopped);
        let (response_tx, response_rx) = oneshot::channel();
        let _ = self.command_tx.send(SessionCommand::Stop { response_tx });
        let stop_result = response_rx
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("ACP session controller stopped")));
        let worker = self.worker.lock().ok().and_then(|mut worker| worker.take());
        if let Some(worker) = worker {
            tokio::task::spawn_blocking(move || worker.join())
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?
                .map_err(|_| anyhow::anyhow!("ACP worker thread panicked"))?;
        }
        stop_result
    }

    fn status(&self) -> ExternalAgentStatus {
        self.status
            .lock()
            .map(|status| *status)
            .unwrap_or(ExternalAgentStatus::Error)
    }
}

async fn run_acp_controller(
    binary: String,
    spec: ExternalAgentSpec,
    permission_handler: Option<Arc<dyn AcpPermissionHandler>>,
    mut command_rx: mpsc::UnboundedReceiver<SessionCommand>,
    startup_tx: oneshot::Sender<anyhow::Result<String>>,
    status: Arc<Mutex<ExternalAgentStatus>>,
) {
    let startup_result = async {
        let mut command = Command::new(binary);
        command.args(&spec.args);
        if let Some(working_dir) = &spec.working_dir {
            command.current_dir(working_dir);
        }
        command.envs(&spec.env);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.kill_on_drop(true);

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("ACP child missing stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("ACP child missing stdout"))?;
        let client_state = Arc::new(Mutex::new(AcpClientState::default()));
        let client = Arc::new(AcpClient::new(
            client_state,
            spec.session_key.clone(),
            permission_handler,
        ));
        if let Some(stderr) = child.stderr.take() {
            tokio::task::spawn_local(forward_stderr(stderr, Arc::clone(&client)));
        }

        let (conn, io_task) = acp::ClientSideConnection::new(
            Arc::clone(&client),
            stdin.compat_write(),
            stdout.compat(),
            |future| {
                tokio::task::spawn_local(future);
            },
        );
        tokio::task::spawn_local(async move {
            let _ = io_task.await;
        });

        let initialize = acp::InitializeRequest::new(acp::ProtocolVersion::V1)
            .client_capabilities(
                acp::ClientCapabilities::new()
                    .fs(acp::FileSystemCapabilities::new()
                        .read_text_file(true)
                        .write_text_file(true))
                    .terminal(true),
            )
            .client_info(
                acp::Implementation::new("moltis", env!("CARGO_PKG_VERSION")).title("Moltis"),
            );
        conn.initialize(initialize)
            .await
            .map_err(acp_error_to_anyhow)?;
        let cwd = spec
            .working_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("."));
        let session = conn
            .new_session(acp::NewSessionRequest::new(cwd))
            .await
            .map_err(acp_error_to_anyhow)?;
        let session_id = session.session_id.to_string();
        client.set_session_id(session_id.clone());
        set_status(&status, ExternalAgentStatus::Idle);
        Ok::<_, anyhow::Error>((child, conn, client, session_id))
    }
    .await;

    let (mut child, conn, client, session_id) = match startup_result {
        Ok(started) => {
            let session_id = started.3.clone();
            let _ = startup_tx.send(Ok(session_id));
            started
        },
        Err(error) => {
            set_status(&status, ExternalAgentStatus::Error);
            let _ = startup_tx.send(Err(error));
            return;
        },
    };

    while let Some(command) = command_rx.recv().await {
        match command {
            SessionCommand::RunTurn {
                prompt,
                response_tx,
            } => {
                client.clear_events();
                let result = conn
                    .prompt(acp::PromptRequest::new(session_id.clone(), vec![
                        acp::ContentBlock::from(prompt),
                    ]))
                    .await
                    .map_err(acp_error_to_anyhow)
                    .and_then(|response| prompt_response_to_events(response, &client));
                let _ = response_tx.send(result);
            },
            SessionCommand::Stop { response_tx } => {
                let cancel_result = conn
                    .cancel(acp::CancelNotification::new(session_id.clone()))
                    .await
                    .map_err(acp_error_to_anyhow);
                let _ = child.kill().await;
                let _ = child.wait().await;
                set_status(&status, ExternalAgentStatus::Stopped);
                let _ = response_tx.send(cancel_result);
                return;
            },
        }
    }

    let _ = child.kill().await;
    let _ = child.wait().await;
    set_status(&status, ExternalAgentStatus::Stopped);
}

async fn forward_stderr(stderr: ChildStderr, client: Arc<AcpClient>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        client.push_event(ExternalAgentEvent::ThinkingDelta(format!(
            "ACP stderr: {line}"
        )));
    }
}

fn prompt_response_to_events(
    response: acp::PromptResponse,
    client: &AcpClient,
) -> anyhow::Result<Vec<ExternalAgentEvent>> {
    let mut events = client.take_events();
    match response.stop_reason {
        acp::StopReason::EndTurn => {
            events.push(ExternalAgentEvent::Done { usage: None });
            Ok(events)
        },
        acp::StopReason::Cancelled => {
            events.push(ExternalAgentEvent::Error("ACP turn cancelled".to_string()));
            Ok(events)
        },
        reason => {
            events.push(ExternalAgentEvent::Error(format!(
                "ACP turn ended with {reason:?}"
            )));
            Ok(events)
        },
    }
}

fn content_chunk_summary(chunk: &acp::ContentChunk) -> String {
    content_block_summary(&chunk.content)
}

fn content_block_summary(block: &acp::ContentBlock) -> String {
    match block {
        acp::ContentBlock::Text(content) => content.text.clone(),
        acp::ContentBlock::Image(_) => "<image>".into(),
        acp::ContentBlock::Audio(_) => "<audio>".into(),
        acp::ContentBlock::ResourceLink(resource) => resource.uri.to_string(),
        acp::ContentBlock::Resource(_) => "<resource>".into(),
        _ => "<content>".into(),
    }
}

fn tool_call_update_summary(update: &acp::ToolCallUpdate) -> String {
    let title = update
        .fields
        .title
        .clone()
        .unwrap_or_else(|| format!("tool {}", update.tool_call_id));
    if let Some(status) = update.fields.status {
        format!("tool update: {title} ({status:?})")
    } else {
        format!("tool update: {title}")
    }
}

fn permission_option_kind(kind: acp::PermissionOptionKind) -> AcpPermissionOptionKind {
    match kind {
        acp::PermissionOptionKind::AllowOnce => AcpPermissionOptionKind::AllowOnce,
        acp::PermissionOptionKind::AllowAlways => AcpPermissionOptionKind::AllowAlways,
        acp::PermissionOptionKind::RejectOnce => AcpPermissionOptionKind::RejectOnce,
        acp::PermissionOptionKind::RejectAlways => AcpPermissionOptionKind::RejectAlways,
        _ => AcpPermissionOptionKind::RejectOnce,
    }
}

fn session_info_summary(update: &acp::SessionInfoUpdate) -> String {
    maybe_undefined_to_string(&update.title)
        .map(|title| format!("session title: {title}"))
        .unwrap_or_else(|| "session info updated".to_string())
}

fn maybe_undefined_to_string<T: ToString>(value: &acp::MaybeUndefined<T>) -> Option<String> {
    match value {
        acp::MaybeUndefined::Value(value) => Some(value.to_string()),
        acp::MaybeUndefined::Null | acp::MaybeUndefined::Undefined => None,
    }
}

fn session_update_kind(update: &acp::SessionUpdate) -> &'static str {
    match update {
        acp::SessionUpdate::UserMessageChunk(_) => "user_message_chunk",
        acp::SessionUpdate::AgentMessageChunk(_) => "agent_message_chunk",
        acp::SessionUpdate::AgentThoughtChunk(_) => "agent_thought_chunk",
        acp::SessionUpdate::ToolCall(_) => "tool_call",
        acp::SessionUpdate::ToolCallUpdate(_) => "tool_call_update",
        acp::SessionUpdate::Plan(_) => "plan",
        acp::SessionUpdate::AvailableCommandsUpdate(_) => "available_commands_update",
        acp::SessionUpdate::CurrentModeUpdate(_) => "current_mode_update",
        acp::SessionUpdate::ConfigOptionUpdate(_) => "config_option_update",
        acp::SessionUpdate::SessionInfoUpdate(_) => "session_info_update",
        _ => "unknown",
    }
}

fn slice_lines(content: &str, line: Option<u32>, limit: Option<u32>) -> String {
    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    let limit = limit.unwrap_or(u32::MAX) as usize;
    content
        .lines()
        .skip(start)
        .take(limit)
        .collect::<Vec<_>>()
        .join("\n")
}

fn acp_error_to_anyhow(error: acp::Error) -> anyhow::Error {
    anyhow::anyhow!(error.to_string())
}

fn acp_error_from_io(error: std::io::Error) -> acp::Error {
    acp::Error::internal_error().data(Value::String(error.to_string()))
}

fn set_status(status: &Mutex<ExternalAgentStatus>, next: ExternalAgentStatus) {
    if let Ok(mut status) = status.lock() {
        *status = next;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::*,
        crate::types::{AcpPermissionHandler, AcpPermissionRequest},
        agent_client_protocol::Client as _,
    };

    struct AllowFirstPermissionHandler;

    #[async_trait]
    impl AcpPermissionHandler for AllowFirstPermissionHandler {
        async fn select_option(
            &self,
            request: AcpPermissionRequest,
        ) -> anyhow::Result<Option<String>> {
            Ok(request.options.first().map(|option| option.id.clone()))
        }
    }

    fn test_client() -> (AcpClient, Arc<Mutex<AcpClientState>>) {
        let state = Arc::new(Mutex::new(AcpClientState::default()));
        let client = AcpClient::new(Arc::clone(&state), Some("main".to_string()), None);
        client.set_session_id("session-1".to_string());
        (client, state)
    }

    #[tokio::test]
    async fn session_notifications_map_acp_chunks_to_external_events() {
        let (client, _state) = test_client();
        client
            .session_notification(acp::SessionNotification::new(
                "session-1",
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    acp::ContentBlock::from("hello"),
                )),
            ))
            .await
            .expect("agent message notification");
        client
            .session_notification(acp::SessionNotification::new(
                "session-1",
                acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
                    acp::ContentBlock::from("thinking"),
                )),
            ))
            .await
            .expect("thought notification");
        client
            .session_notification(acp::SessionNotification::new(
                "session-1",
                acp::SessionUpdate::ToolCall(acp::ToolCall::new("tool-1", "read file")),
            ))
            .await
            .expect("tool call notification");

        let events = client.take_events();
        assert!(matches!(
            &events[0],
            ExternalAgentEvent::TextDelta(text) if text == "hello"
        ));
        assert!(matches!(
            &events[1],
            ExternalAgentEvent::ThinkingDelta(text) if text == "thinking"
        ));
        assert!(matches!(
            &events[2],
            ExternalAgentEvent::ToolCallStart { id, name, .. }
                if id == "tool-1" && name == "read file"
        ));
    }

    #[test]
    fn prompt_response_end_turn_appends_done_event() {
        let (client, _state) = test_client();
        client.push_event(ExternalAgentEvent::TextDelta("done".to_string()));

        let events =
            prompt_response_to_events(acp::PromptResponse::new(acp::StopReason::EndTurn), &client)
                .expect("prompt response events");

        assert!(matches!(
            &events[0],
            ExternalAgentEvent::TextDelta(text) if text == "done"
        ));
        assert!(matches!(
            events.last(),
            Some(ExternalAgentEvent::Done { usage: None })
        ));
    }

    #[test]
    fn prompt_response_cancelled_maps_to_error_event() {
        let (client, _state) = test_client();

        let events = prompt_response_to_events(
            acp::PromptResponse::new(acp::StopReason::Cancelled),
            &client,
        )
        .expect("prompt response events");

        assert!(matches!(
            events.as_slice(),
            [ExternalAgentEvent::Error(message)] if message == "ACP turn cancelled"
        ));
    }

    #[test]
    fn slice_lines_honors_one_based_line_and_limit() {
        assert_eq!(
            slice_lines("one\ntwo\nthree\nfour", Some(2), Some(2)),
            "two\nthree"
        );
        assert_eq!(slice_lines("one\ntwo", None, Some(1)), "one");
        assert_eq!(slice_lines("one\ntwo", Some(10), Some(2)), "");
    }

    #[tokio::test]
    async fn permission_handler_selects_acp_option() {
        let state = Arc::new(Mutex::new(AcpClientState::default()));
        let client = AcpClient::new(
            Arc::clone(&state),
            Some("main".to_string()),
            Some(Arc::new(AllowFirstPermissionHandler)),
        );
        let response = client
            .request_permission(acp::RequestPermissionRequest::new(
                "session-1",
                acp::ToolCallUpdate::new(
                    "tool-1",
                    acp::ToolCallUpdateFields::new().title("run tool".to_string()),
                ),
                vec![acp::PermissionOption::new(
                    "allow-once",
                    "Allow once",
                    acp::PermissionOptionKind::AllowOnce,
                )],
            ))
            .await
            .expect("permission response");

        assert!(matches!(
            response.outcome,
            acp::RequestPermissionOutcome::Selected(outcome) if outcome.option_id.to_string() == "allow-once"
        ));
    }

    #[tokio::test]
    async fn terminal_methods_capture_command_output() {
        let (client, _state) = test_client();
        let response = client
            .create_terminal(
                acp::CreateTerminalRequest::new("session-1", "/bin/sh")
                    .args(vec!["-c".to_string(), "printf hello".to_string()]),
            )
            .await
            .expect("create terminal");
        let output = client
            .terminal_output(acp::TerminalOutputRequest::new(
                "session-1",
                response.terminal_id.clone(),
            ))
            .await
            .expect("terminal output");

        assert_eq!(output.output, "hello");
        assert_eq!(
            output.exit_status.and_then(|status| status.exit_code),
            Some(0)
        );

        client
            .release_terminal(acp::ReleaseTerminalRequest::new(
                "session-1",
                response.terminal_id,
            ))
            .await
            .expect("release terminal");
    }
}
