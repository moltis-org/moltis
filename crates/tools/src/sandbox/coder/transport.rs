use std::{sync::LazyLock, time::Duration};

use {
    base64::Engine,
    serde::Serialize,
    sha2::{Digest, Sha256},
    tokio_tungstenite::tungstenite::Message,
};

use crate::error::{Error, Result};

pub(super) const PTY_COLS: u16 = 1000;
pub(super) const PTY_ROWS: u16 = 200;
pub(super) const STDIN_CHUNK_BYTES: usize = 1024;
pub(super) const CLIENT_MESSAGE_CAP: usize = 32 * 1024;

#[derive(Clone)]
pub(super) struct PtyMarkers {
    pub(super) ready: String,
    pub(super) eof: String,
    pub(super) exit: String,
    pub(super) stderr_begin: String,
    pub(super) stderr_end: String,
}

impl PtyMarkers {
    pub(super) fn new() -> Self {
        Self::with_nonce(&uuid::Uuid::new_v4().simple().to_string())
    }

    pub(super) fn with_nonce(nonce: &str) -> Self {
        Self {
            ready: format!("__MOLTIS_READY_{nonce}__"),
            eof: format!("__MOLTIS_EOF_{nonce}__"),
            exit: format!("__MOLTIS_EXIT_{nonce}__"),
            stderr_begin: format!("__MOLTIS_STDERR_{nonce}_BEGIN__"),
            stderr_end: format!("__MOLTIS_STDERR_{nonce}_END__"),
        }
    }
}

#[derive(Serialize)]
struct ResizeMessage {
    height: u16,
    width: u16,
}

#[derive(Serialize)]
struct DataMessage<'a> {
    data: &'a str,
}

fn binary_json<T: Serialize>(value: &T) -> Result<Message> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| Error::message(format!("coder: failed to encode pty message: {error}")))?;
    if bytes.len() >= CLIENT_MESSAGE_CAP {
        return Err(Error::message(format!(
            "coder: pty message is {} bytes, exceeding the safe {} byte limit",
            bytes.len(),
            CLIENT_MESSAGE_CAP - 1
        )));
    }
    Ok(Message::Binary(bytes.into()))
}

pub(super) fn resize_message() -> Result<Message> {
    binary_json(&ResizeMessage {
        height: PTY_ROWS,
        width: PTY_COLS,
    })
}

pub(super) fn stdin_message(data: &str) -> Result<Message> {
    binary_json(&DataMessage { data })
}

pub(super) fn ctrl_c_message() -> Result<Message> {
    stdin_message("\u{3}")
}

pub(super) fn encoded_script(script: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(script.as_bytes())
}

pub(super) fn framed_stdin_chunk(chunk: &str) -> String {
    let mut framed = String::with_capacity(chunk.len() + 1);
    framed.push_str(chunk);
    framed.push('\n');
    framed
}

pub(super) fn eof_stdin(markers: &PtyMarkers) -> String {
    framed_stdin_chunk(&markers.eof)
}

pub(super) fn bootstrap_command(markers: &PtyMarkers) -> String {
    let PtyMarkers { ready, eof, .. } = markers;
    format!(
        "stty raw -echo 2>/dev/null; TERM=dumb; export TERM; \
         printf '\n{ready}\n'; \
         t=$(mktemp /tmp/moltis-rx.XXXXXX) || exit 125; \
         sed -n '/^{eof}$/q;p' | base64 -d > \"$t\"; \
         sh \"$t\" </dev/null; rm -f \"$t\""
    )
}

fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub(super) fn wrapped_command(
    command: &str,
    cwd: &str,
    env: &[(String, String)],
    markers: &PtyMarkers,
    timeout: Duration,
) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(command.as_bytes());
    let encoded = shell_words::quote(&encoded);
    let cwd = shell_words::quote(cwd);
    let env_prefix = env
        .iter()
        .filter(|(key, _)| valid_env_key(key))
        .map(|(key, value)| format!("{}={}", key, shell_words::quote(value)))
        .collect::<Vec<_>>()
        .join(" ");
    let env_prefix = if env_prefix.is_empty() {
        String::new()
    } else {
        format!("env {env_prefix} ")
    };
    let timeout_secs = timeout
        .as_secs()
        .saturating_add(u64::from(timeout.subsec_nanos() != 0))
        .max(1);
    let PtyMarkers {
        exit: exit_marker,
        stderr_begin,
        stderr_end,
        ..
    } = markers;

    format!(
        "tmp=$(mktemp /tmp/moltis-cmd.XXXXXX) || exit 125; \
         err=$(mktemp /tmp/moltis-stderr.XXXXXX) || {{ rm -f \"$tmp\"; exit 125; }}; \
         printf %s {encoded} | base64 -d > \"$tmp\"; decode_status=$?; \
         if [ \"$decode_status\" -ne 0 ]; then rm -f \"$tmp\" \"$err\"; exit \"$decode_status\"; fi; \
         if ! command -v timeout >/dev/null 2>&1; then \
           printf '%s\n' 'moltis: required timeout utility is unavailable' >\"$err\"; status=125; \
         else \
           (cd {cwd} && {env_prefix}timeout --signal=TERM --kill-after=2s {timeout_secs}s sh \"$tmp\") 2>\"$err\"; status=$?; \
         fi; \
         printf '\n{exit_marker}%s\n' \"$status\"; \
         printf '{stderr_begin}\n'; cat \"$err\" 2>/dev/null; printf '\n{stderr_end}\n'; \
         rm -f \"$tmp\" \"$err\"; exit \"$status\""
    )
}

#[derive(Debug, PartialEq, Eq)]
enum ParseState {
    AwaitReady,
    Stdout,
    ExitCode,
    AwaitStderr,
    Stderr,
    Done,
}

#[derive(Debug)]
pub(super) struct ParsedPtyOutput {
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) exit_code: i32,
}

pub(super) struct PtyOutputParser {
    markers: PtyMarkers,
    state: ParseState,
    pending: Vec<u8>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: Option<i32>,
    max_output_bytes: usize,
    trim_stdout_start: bool,
    trim_stderr_start: bool,
}

impl PtyOutputParser {
    pub(super) fn new(markers: PtyMarkers, max_output_bytes: usize) -> Self {
        Self {
            markers,
            state: ParseState::AwaitReady,
            pending: Vec::new(),
            stdout: Vec::with_capacity(max_output_bytes.min(8192)),
            stderr: Vec::with_capacity(max_output_bytes.min(8192)),
            exit_code: None,
            max_output_bytes,
            trim_stdout_start: true,
            trim_stderr_start: true,
        }
    }

    pub(super) fn is_done(&self) -> bool {
        self.state == ParseState::Done
    }

    pub(super) fn feed_message(&mut self, message: &Message) -> Result<bool> {
        match message {
            Message::Text(text) => self.feed(text.as_bytes()),
            Message::Binary(bytes) => self.feed(bytes),
            _ => Ok(false),
        }
    }

    /// Returns true exactly when this feed observes the ready marker.
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Result<bool> {
        if self.is_done() {
            return Ok(false);
        }
        let mut became_ready = false;
        for chunk in bytes.chunks(4096) {
            self.pending.extend_from_slice(chunk);
            became_ready |= self.process_pending()?;
            if self.is_done() {
                self.pending.clear();
                break;
            }
        }
        Ok(became_ready)
    }

    fn process_pending(&mut self) -> Result<bool> {
        let mut became_ready = false;
        loop {
            match self.state {
                ParseState::AwaitReady => {
                    let marker = self.markers.ready.as_bytes();
                    let Some(before) = take_through_marker(&mut self.pending, marker, false) else {
                        break;
                    };
                    drop(before);
                    self.state = ParseState::Stdout;
                    became_ready = true;
                },
                ParseState::Stdout => {
                    trim_leading_newlines(&mut self.pending, &mut self.trim_stdout_start);
                    let marker = self.markers.exit.as_bytes();
                    let Some(before) = take_through_marker(&mut self.pending, marker, true) else {
                        append_safe_prefix(
                            &mut self.pending,
                            marker,
                            &mut self.stdout,
                            self.max_output_bytes,
                        );
                        break;
                    };
                    append_bounded(&mut self.stdout, &before, self.max_output_bytes);
                    trim_trailing_newlines(&mut self.stdout);
                    self.state = ParseState::ExitCode;
                },
                ParseState::ExitCode => {
                    let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') else {
                        if self.pending.len() > 32 {
                            return Err(Error::message("coder: pty exit code is too long"));
                        }
                        break;
                    };
                    let line = self.pending.drain(..=newline).collect::<Vec<_>>();
                    let text = String::from_utf8_lossy(&line);
                    let text = text.trim_matches(['\r', '\n', ' ']);
                    self.exit_code = Some(text.parse::<i32>().map_err(|error| {
                        Error::message(format!("coder: invalid pty exit code {text:?}: {error}"))
                    })?);
                    self.state = ParseState::AwaitStderr;
                },
                ParseState::AwaitStderr => {
                    let marker = self.markers.stderr_begin.as_bytes();
                    if take_through_marker(&mut self.pending, marker, false).is_none() {
                        break;
                    }
                    self.state = ParseState::Stderr;
                },
                ParseState::Stderr => {
                    trim_leading_newlines(&mut self.pending, &mut self.trim_stderr_start);
                    let marker = self.markers.stderr_end.as_bytes();
                    let Some(before) = take_through_marker(&mut self.pending, marker, true) else {
                        append_safe_prefix(
                            &mut self.pending,
                            marker,
                            &mut self.stderr,
                            self.max_output_bytes,
                        );
                        break;
                    };
                    append_bounded(&mut self.stderr, &before, self.max_output_bytes);
                    trim_trailing_newlines(&mut self.stderr);
                    self.state = ParseState::Done;
                },
                ParseState::Done => break,
            }
        }
        Ok(became_ready)
    }

    pub(super) fn finish(self) -> Result<ParsedPtyOutput> {
        if self.state != ParseState::Done {
            return Err(Error::message(
                "coder: pty output missing completion markers",
            ));
        }
        let exit_code = self
            .exit_code
            .ok_or_else(|| Error::message("coder: pty output missing exit code"))?;
        Ok(ParsedPtyOutput {
            stdout: output_string(&self.stdout, self.max_output_bytes),
            stderr: output_string(&self.stderr, self.max_output_bytes),
            exit_code,
        })
    }

    #[cfg(test)]
    pub(super) fn buffered_bytes(&self) -> usize {
        self.pending.len() + self.stdout.len() + self.stderr.len()
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn take_through_marker(
    pending: &mut Vec<u8>,
    marker: &[u8],
    return_prefix: bool,
) -> Option<Vec<u8>> {
    if let Some(position) = find_subslice(pending, marker) {
        let prefix = if return_prefix {
            pending[..position].to_vec()
        } else {
            Vec::new()
        };
        pending.drain(..position + marker.len());
        return Some(prefix);
    }
    if !return_prefix {
        let keep = marker.len().saturating_sub(1);
        if pending.len() > keep {
            pending.drain(..pending.len() - keep);
        }
    }
    None
}

fn append_safe_prefix(pending: &mut Vec<u8>, marker: &[u8], output: &mut Vec<u8>, limit: usize) {
    let keep = marker.len().saturating_sub(1);
    let count = pending.len().saturating_sub(keep);
    append_bounded(output, &pending[..count], limit);
    pending.drain(..count);
}

fn append_bounded(output: &mut Vec<u8>, bytes: &[u8], limit: usize) {
    let remaining = limit.saturating_sub(output.len());
    output.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
}

fn trim_leading_newlines(pending: &mut Vec<u8>, enabled: &mut bool) {
    if !*enabled {
        return;
    }
    let count = pending
        .iter()
        .take_while(|byte| matches!(byte, b'\r' | b'\n'))
        .count();
    pending.drain(..count);
    if !pending.is_empty() {
        *enabled = false;
    }
}

fn trim_trailing_newlines(output: &mut Vec<u8>) {
    while matches!(output.last(), Some(b'\r' | b'\n')) {
        output.pop();
    }
}

static ANSI_RE: LazyLock<Option<regex::Regex>> =
    LazyLock::new(|| regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").ok());

fn output_string(bytes: &[u8], limit: usize) -> String {
    let decoded = String::from_utf8_lossy(bytes).replace('\r', "");
    let mut output = match ANSI_RE.as_ref() {
        Some(regex) => regex.replace_all(&decoded, "").into_owned(),
        None => decoded,
    };
    if output.len() > limit {
        output.truncate(output.floor_char_boundary(limit));
    }
    output
}

pub(super) fn workspace_digest(prefix: &str, key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    hasher.update([0]);
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    digest[..5]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
