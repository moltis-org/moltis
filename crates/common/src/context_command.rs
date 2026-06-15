use std::{path::Path, time::Duration};

use {
    tokio::process::Command,
    tracing::{debug, warn},
};

const CONTEXT_COMMAND_TIMEOUT_SECS: u64 = 30;

/// Run a configured context command and return stdout when it succeeds.
///
/// The command is operator-configured trusted input. Failures are logged and
/// treated as missing context so a broken context generator does not block chat.
pub async fn run_context_command(
    command: Option<&str>,
    working_dir: Option<&Path>,
) -> Option<String> {
    let command = command.map(str::trim).filter(|value| !value.is_empty())?;

    let mut cmd = shell_command(command);
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    let output = match tokio::time::timeout(
        Duration::from_secs(CONTEXT_COMMAND_TIMEOUT_SECS),
        cmd.output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            warn!(%error, "context_command failed to start");
            return None;
        },
        Err(_) => {
            warn!(
                timeout_secs = CONTEXT_COMMAND_TIMEOUT_SECS,
                "context_command timed out"
            );
            return None;
        },
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(
            exit_code = output.status.code(),
            stderr = %stderr,
            "context_command failed"
        );
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        warn!("context_command produced no output");
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
}
