//! First-class adapter for the OAuth-authenticated Antigravity (`agy`) CLI.
//!
//! AGY does not implement ACP. Each turn is one official `agy` process whose
//! NDJSON stdout is translated into Moltis' existing external-agent stream.

mod translate;
mod wire;

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use {
    async_trait::async_trait,
    futures::Stream,
    tokio::{
        io::{AsyncBufReadExt, BufReader},
        process::{Child, ChildStderr, ChildStdout, Command},
        sync::{mpsc, watch},
    },
    tokio_stream::wrappers::ReceiverStream,
};

use crate::{
    runtimes::{env::inject_managed_files_dir, process::build_process_input},
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{
        AgentTransportKind, ContextSnapshot, ExternalAgentEvent, ExternalAgentSpec,
        ExternalAgentStatus,
    },
};

use self::{translate::Translator, wire::parse_line};

const BINARY_NAME: &str = "agy";
const DEFAULT_TURN_TIMEOUT_SECS: u64 = 60 * 60;
const EVENT_CHANNEL_CAPACITY: usize = 256;
const STDERR_TAIL_LINES: usize = 20;

pub struct AgyTransport;

impl AgyTransport {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for AgyTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExternalAgentTransport for AgyTransport {
    fn name(&self) -> &str {
        "agy-stream-json"
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    async fn is_available(&self) -> bool {
        which::which(BINARY_NAME).is_ok()
    }

    fn supported_kinds(&self) -> &[AgentTransportKind] {
        &[AgentTransportKind::Agy]
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, spec)))]
    async fn start_session(
        &self,
        spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
        Ok(Box::new(AgySession::new(spec)))
    }
}

struct AgySession {
    binary: String,
    extra_args: Vec<String>,
    env: HashMap<String, String>,
    configured_working_dir: Option<PathBuf>,
    timeout: Duration,
    initial_session_id: Option<String>,
    session_id: Arc<RwLock<Option<String>>>,
    status: Arc<RwLock<ExternalAgentStatus>>,
    cancel_tx: Option<watch::Sender<bool>>,
    model: Option<String>,
    effort: Option<String>,
}

impl AgySession {
    fn new(spec: &ExternalAgentSpec) -> Self {
        let initial_session_id = spec
            .external_session_id
            .as_ref()
            .filter(|id| !id.trim().is_empty())
            .cloned();
        Self {
            binary: spec
                .binary
                .clone()
                .unwrap_or_else(|| BINARY_NAME.to_string()),
            extra_args: spec.args.clone(),
            env: spec.env.clone(),
            configured_working_dir: spec.working_dir.clone(),
            timeout: Duration::from_secs(spec.timeout_secs.unwrap_or(DEFAULT_TURN_TIMEOUT_SECS)),
            initial_session_id: initial_session_id.clone(),
            session_id: Arc::new(RwLock::new(initial_session_id)),
            status: Arc::new(RwLock::new(ExternalAgentStatus::Idle)),
            cancel_tx: None,
            model: spec.model.clone(),
            effort: spec.effort.clone(),
        }
    }

    fn current_session_id(&self) -> Option<String> {
        self.session_id
            .read()
            .map(|session_id| session_id.clone())
            .unwrap_or_default()
    }

    fn set_status(&self, next: ExternalAgentStatus) {
        match self.status.write() {
            Ok(mut status) => *status = next,
            Err(error) => *error.into_inner() = next,
        }
    }
}

#[async_trait]
impl ExternalAgentSession for AgySession {
    fn external_session_id(&self) -> Option<&str> {
        self.initial_session_id.as_deref()
    }

    async fn send_prompt(
        &mut self,
        prompt: &str,
        context: Option<&ContextSnapshot>,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = ExternalAgentEvent> + Send>>> {
        if self.status() == ExternalAgentStatus::Running {
            anyhow::bail!("an AGY turn is already running for this session")
        }

        let resume_session_id = self.current_session_id();
        let prompt = if resume_session_id.is_some() {
            prompt.to_string()
        } else {
            build_process_input(prompt, context)
        };
        let working_dir = context
            .and_then(|snapshot| snapshot.working_dir.clone())
            .or_else(|| self.configured_working_dir.clone());
        let args = args_for_turn(
            &prompt,
            resume_session_id.as_deref(),
            working_dir.as_deref(),
            self.model.as_deref(),
            self.effort.as_deref(),
            self.timeout,
            &self.extra_args,
        );

        let mut command = Command::new(&self.binary);
        command.args(args);
        if let Some(working_dir) = &working_dir {
            command.current_dir(working_dir);
        }
        command.envs(&self.env);
        inject_managed_files_dir(&mut command);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.kill_on_drop(true);

        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("AGY stdout is unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("AGY stderr is unavailable"))?;
        let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.cancel_tx = Some(cancel_tx);
        self.set_status(ExternalAgentStatus::Running);

        tokio::spawn(run_turn(
            child,
            stdout,
            stderr,
            event_tx,
            cancel_rx,
            Arc::clone(&self.session_id),
            Arc::clone(&self.status),
            self.timeout,
            resume_session_id,
        ));

        Ok(Box::pin(ReceiverStream::new(event_rx)))
    }

    async fn is_alive(&self) -> bool {
        !matches!(
            self.status(),
            ExternalAgentStatus::Stopped | ExternalAgentStatus::Error
        )
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        if let Some(cancel_tx) = self.cancel_tx.take() {
            let _ = cancel_tx.send(true);
        }
        self.set_status(ExternalAgentStatus::Stopped);
        Ok(())
    }

    fn status(&self) -> ExternalAgentStatus {
        self.status
            .read()
            .map(|status| *status)
            .unwrap_or(ExternalAgentStatus::Error)
    }
}

#[allow(clippy::too_many_arguments)]
fn args_for_turn(
    prompt: &str,
    resume_session_id: Option<&str>,
    working_dir: Option<&Path>,
    model: Option<&str>,
    effort: Option<&str>,
    timeout: Duration,
    extra_args: &[String],
) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        prompt.to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--print-timeout".to_string(),
        format!("{}s", timeout.as_secs()),
    ];
    if let Some(session_id) = resume_session_id.filter(|id| !id.trim().is_empty()) {
        args.extend(["--conversation".to_string(), session_id.to_string()]);
    } else {
        args.push("--new-project".to_string());
    }
    if let Some(working_dir) = working_dir {
        args.extend([
            "--add-dir".to_string(),
            working_dir.to_string_lossy().into_owned(),
        ]);
    }
    if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    if let Some(effort) = effort.filter(|effort| !effort.trim().is_empty()) {
        args.extend(["--effort".to_string(), effort.to_string()]);
    }
    args.extend(extra_args.iter().cloned());
    args
}

async fn run_turn(
    mut child: Child,
    stdout: ChildStdout,
    stderr: ChildStderr,
    event_tx: mpsc::Sender<ExternalAgentEvent>,
    mut cancel_rx: watch::Receiver<bool>,
    session_id: Arc<RwLock<Option<String>>>,
    status: Arc<RwLock<ExternalAgentStatus>>,
    idle_timeout: Duration,
    initial_session_id: Option<String>,
) {
    let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
    let stderr_task = tokio::spawn(collect_stderr(stderr, Arc::clone(&stderr_tail)));
    let mut lines = BufReader::new(stdout).lines();
    let mut translator = Translator::with_session_id(initial_session_id);
    let mut terminal_seen = false;
    let mut terminal_failed = false;

    loop {
        let next_line = tokio::select! {
            _ = cancel_rx.changed() => {
                let _ = child.kill().await;
                send_partial(&event_tx, &mut translator).await;
                let _ = event_tx
                    .send(ExternalAgentEvent::Error("AGY turn cancelled".to_string()))
                    .await;
                set_shared_status(&status, ExternalAgentStatus::Stopped);
                let _ = stderr_task.await;
                return;
            },
            result = tokio::time::timeout(idle_timeout, lines.next_line()) => {
                match result {
                    Ok(Ok(line)) => line,
                    Ok(Err(error)) => {
                        let _ = child.kill().await;
                        send_partial(&event_tx, &mut translator).await;
                        let _ = event_tx.send(ExternalAgentEvent::Error(format!(
                            "failed to read AGY output: {error}{}",
                            stderr_suffix(&stderr_tail),
                        ))).await;
                        set_shared_status(&status, ExternalAgentStatus::Error);
                        let _ = stderr_task.await;
                        return;
                    },
                    Err(_) => {
                        let _ = child.kill().await;
                        send_partial(&event_tx, &mut translator).await;
                        let _ = event_tx.send(ExternalAgentEvent::Error(format!(
                            "AGY produced no output for {} seconds{}",
                            idle_timeout.as_secs(),
                            stderr_suffix(&stderr_tail),
                        ))).await;
                        set_shared_status(&status, ExternalAgentStatus::Error);
                        let _ = stderr_task.await;
                        return;
                    },
                }
            },
        };

        let Some(line) = next_line else {
            break;
        };
        let Some(frame) = parse_line(&line) else {
            continue;
        };
        for event in translator.translate(frame) {
            if let ExternalAgentEvent::SessionBound {
                external_session_id,
            } = &event
            {
                set_shared_session_id(&session_id, external_session_id.clone());
            }
            if matches!(event, ExternalAgentEvent::Done { .. }) {
                terminal_seen = true;
            } else if matches!(event, ExternalAgentEvent::Error(_)) {
                terminal_seen = true;
                terminal_failed = true;
            }
            if event_tx.send(event).await.is_err() {
                let _ = child.kill().await;
                set_shared_status(&status, ExternalAgentStatus::Stopped);
                let _ = stderr_task.await;
                return;
            }
        }
        if terminal_seen {
            break;
        }
    }

    let exit = match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(result) => result.ok(),
        Err(_) => {
            let _ = child.kill().await;
            child.wait().await.ok()
        },
    };
    let _ = stderr_task.await;
    if !terminal_seen {
        send_partial(&event_tx, &mut translator).await;
        let exit_description = exit
            .map(|status| status.to_string())
            .unwrap_or_else(|| "unknown status".to_string());
        let _ = event_tx
            .send(ExternalAgentEvent::Error(format!(
                "AGY exited with {exit_description} before a result frame{}",
                stderr_suffix(&stderr_tail),
            )))
            .await;
        terminal_failed = true;
    }
    set_shared_status(
        &status,
        if terminal_failed {
            ExternalAgentStatus::Error
        } else {
            ExternalAgentStatus::Idle
        },
    );
}

async fn send_partial(event_tx: &mpsc::Sender<ExternalAgentEvent>, translator: &mut Translator) {
    if let Some(text) = translator.take_partial_text() {
        let _ = event_tx.send(ExternalAgentEvent::TextDelta(text)).await;
    }
}

async fn collect_stderr(stderr: ChildStderr, tail: Arc<Mutex<VecDeque<String>>>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut tail = tail.lock().unwrap_or_else(|error| error.into_inner());
        if tail.len() == STDERR_TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }
}

fn stderr_suffix(tail: &Arc<Mutex<VecDeque<String>>>) -> String {
    let tail = tail.lock().unwrap_or_else(|error| error.into_inner());
    if tail.is_empty() {
        String::new()
    } else {
        format!(
            "; stderr: {}",
            tail.iter().cloned().collect::<Vec<_>>().join(" | ")
        )
    }
}

fn set_shared_session_id(session_id: &Arc<RwLock<Option<String>>>, next: String) {
    match session_id.write() {
        Ok(mut session_id) => *session_id = Some(next),
        Err(error) => *error.into_inner() = Some(next),
    }
}

fn set_shared_status(status: &Arc<RwLock<ExternalAgentStatus>>, next: ExternalAgentStatus) {
    match status.write() {
        Ok(mut status) => *status = next,
        Err(error) => *error.into_inner() = next,
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Instant};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use futures::StreamExt;

    use super::*;

    fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    }

    #[test]
    fn fresh_and_resumed_turns_use_the_official_stream_contract() {
        let fresh = args_for_turn(
            "hello",
            None,
            Some(Path::new("/work")),
            Some("gemini-3.8-flash-high"),
            Some("high"),
            Duration::from_secs(90),
            &[],
        );
        assert_eq!(flag_value(&fresh, "-p"), Some("hello"));
        assert_eq!(flag_value(&fresh, "--output-format"), Some("stream-json"));
        assert_eq!(flag_value(&fresh, "--print-timeout"), Some("90s"));
        assert_eq!(flag_value(&fresh, "--add-dir"), Some("/work"));
        assert_eq!(flag_value(&fresh, "--model"), Some("gemini-3.8-flash-high"));
        assert_eq!(flag_value(&fresh, "--effort"), Some("high"));
        assert!(fresh.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(fresh.contains(&"--new-project".to_string()));

        let resumed = args_for_turn(
            "again",
            Some("conv-1"),
            None,
            None,
            None,
            Duration::from_secs(90),
            &[],
        );
        assert_eq!(flag_value(&resumed, "--conversation"), Some("conv-1"));
        assert!(!resumed.contains(&"--new-project".to_string()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn progress_is_streamed_before_the_process_exits() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let script = dir.path().join("fake-agy");
        fs::write(
            &script,
            r#"#!/bin/sh
printf '%s\n' '{"event":"init","conversation_id":"conv-live","init":{}}'
printf '%s\n' '{"event":"step_update","step_update":{"conversation_id":"conv-live","step_index":1,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"pwd"}}}}'
sleep 1
printf '%s\n' '{"event":"step_update","step_update":{"conversation_id":"conv-live","step_index":1,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","output":"/tmp"}}}'
printf '%s\n' '{"event":"result","result":{"conversation_id":"conv-live","status":"SUCCESS","response":"done","usage":{"input_tokens":2,"output_tokens":1}}}'
"#,
        )?;
        let mut permissions = fs::metadata(&script)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions)?;

        let mut spec = ExternalAgentSpec::new(AgentTransportKind::Agy);
        spec.binary = Some(script.to_string_lossy().into_owned());
        spec.working_dir = Some(dir.path().to_path_buf());
        let mut session = AgyTransport::new().start_session(&spec).await?;
        let started = Instant::now();
        let mut stream = session.send_prompt("hello", None).await?;
        assert!(started.elapsed() < Duration::from_millis(500));

        let bound = tokio::time::timeout(Duration::from_millis(500), stream.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing session-bound event"))?;
        assert!(matches!(
            bound,
            ExternalAgentEvent::SessionBound { external_session_id }
                if external_session_id == "conv-live"
        ));
        let progress = tokio::time::timeout(Duration::from_millis(500), stream.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing tool-start event"))?;
        assert!(matches!(
            progress,
            ExternalAgentEvent::ToolCallStart { name, .. } if name == "run_command"
        ));

        let remaining: Vec<_> = stream.collect().await;
        assert!(
            remaining
                .iter()
                .any(|event| matches!(event, ExternalAgentEvent::ToolCallEnd {
                    success: true,
                    ..
                }))
        );
        assert!(
            remaining.iter().any(
                |event| matches!(event, ExternalAgentEvent::TextDelta(text) if text == "done")
            )
        );
        assert!(
            remaining
                .iter()
                .any(|event| matches!(event, ExternalAgentEvent::Done { .. }))
        );
        Ok(())
    }
}
