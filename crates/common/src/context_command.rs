use std::{path::Path, process::Stdio, time::Duration};

use {
    tokio::{io::AsyncReadExt, process::Command},
    tracing::{debug, warn},
};

const CONTEXT_COMMAND_TIMEOUT_SECS: u64 = 30;

/// Maximum number of stdout bytes buffered and forwarded to the prompt.
///
/// Mirrors the 32k default of `workspace_file_max_chars` used for file-based
/// context. Output beyond this cap is truncated so a misconfigured command
/// (e.g. `cat /var/log/app.log`) cannot exhaust server memory or blow past the
/// model context window. stderr is bounded to the same limit so a chatty
/// failing command cannot balloon memory via its logs either.
const CONTEXT_COMMAND_MAX_BYTES: usize = 32_000;

/// Run a configured context command and return stdout when it succeeds.
///
/// The command is operator-configured trusted input. When `working_dir` is
/// provided the command runs in that directory (typically the active project
/// or session worktree); otherwise it inherits the server process's current
/// directory. Failures are logged and treated as missing context so a broken
/// context generator does not block chat.
pub async fn run_context_command(
    command: Option<&str>,
    working_dir: Option<&Path>,
) -> Option<String> {
    let command = command.map(str::trim).filter(|value| !value.is_empty())?;

    let mut cmd = shell_command(command);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // Ensure the child is reaped if we abandon it on timeout below.
    cmd.kill_on_drop(true);
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    match tokio::time::timeout(
        Duration::from_secs(CONTEXT_COMMAND_TIMEOUT_SECS),
        run_capped(cmd),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            warn!(
                timeout_secs = CONTEXT_COMMAND_TIMEOUT_SECS,
                "context_command timed out"
            );
            None
        },
    }
}

/// Spawn the command, reading at most [`CONTEXT_COMMAND_MAX_BYTES`] of stdout so
/// unbounded output cannot exhaust memory or the model context window.
async fn run_capped(mut cmd: Command) -> Option<String> {
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            warn!(%error, "context_command failed to start");
            return None;
        },
    };

    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        warn!("context_command stdio pipes unavailable");
        return None;
    };

    // Drain stderr concurrently in a bounded task so a command that is chatty on
    // stderr cannot fill its pipe and deadlock while we read stdout — and so it
    // cannot balloon memory via its logs either.
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = (&mut stderr)
            .take(CONTEXT_COMMAND_MAX_BYTES as u64)
            .read_to_end(&mut buf)
            .await;
        buf
    });

    // Read one byte past the cap so truncation is detectable.
    let mut stdout_buf = Vec::new();
    let stdout_res = (&mut stdout)
        .take(CONTEXT_COMMAND_MAX_BYTES as u64 + 1)
        .read_to_end(&mut stdout_buf)
        .await;
    if let Err(error) = stdout_res {
        warn!(%error, "context_command failed to read stdout");
        return None;
    }

    let truncated = stdout_buf.len() > CONTEXT_COMMAND_MAX_BYTES;
    if truncated {
        stdout_buf.truncate(CONTEXT_COMMAND_MAX_BYTES);
        // We already have all we will use; stop the process so it cannot keep
        // running (and blocking on the full pipe) until the outer timeout.
        let _ = child.start_kill();
        warn!(
            max_bytes = CONTEXT_COMMAND_MAX_BYTES,
            "context_command output truncated"
        );
    }

    let status = match child.wait().await {
        Ok(status) => status,
        Err(error) => {
            warn!(%error, "context_command failed to wait");
            return None;
        },
    };

    // A truncated run was killed deliberately, so its exit status is meaningless.
    if !truncated && !status.success() {
        // The process has exited, so stderr has closed and the drain completes
        // promptly; use its output to explain the failure.
        let stderr_buf = stderr_task.await.unwrap_or_default();
        let stderr = String::from_utf8_lossy(&stderr_buf);
        warn!(
            exit_code = status.code(),
            stderr = %stderr,
            "context_command failed"
        );
        return None;
    }

    let text = String::from_utf8_lossy(&stdout_buf).to_string();
    if text.trim().is_empty() {
        debug!("context_command produced no output");
        None
    } else {
        debug!(len = text.len(), "context_command produced dynamic context");
        Some(text)
    }
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", command]);
        cmd
    }

    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_context_command_is_none() {
        assert_eq!(run_context_command(Some("   "), None).await, None);
        assert_eq!(run_context_command(None, None).await, None);
    }

    #[tokio::test]
    async fn successful_context_command_returns_stdout() {
        let output = run_context_command(Some("printf 'dynamic context'"), None)
            .await
            .expect("context output");
        assert_eq!(output, "dynamic context");
    }

    #[tokio::test]
    async fn failed_context_command_is_none() {
        assert_eq!(run_context_command(Some("exit 12"), None).await, None);
    }

    #[tokio::test]
    async fn working_dir_is_respected() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("ctx.txt"), "from-working-dir").expect("write file");
        let output = run_context_command(Some("cat ctx.txt"), Some(dir.path()))
            .await
            .expect("context output");
        assert_eq!(output, "from-working-dir");
    }

    #[tokio::test]
    async fn large_output_is_truncated() {
        // Emit far more than the cap; output must be truncated, not unbounded.
        let output = run_context_command(Some("yes aaaaaaaaaa | head -c 200000"), None)
            .await
            .expect("context output");
        assert_eq!(output.len(), CONTEXT_COMMAND_MAX_BYTES);
    }
}
