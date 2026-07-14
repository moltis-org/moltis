use std::process::Command;

pub const MOLTIS_HOST_TERMINAL_TMUX_SOCKET_NAME: &str = "moltis-host-terminal";
pub const MOLTIS_HOST_TERMINAL_TMUX_CONFIG_PATH: &str = "/dev/null";

/// Coarse state of a tmux pane before channel input is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxPaneState {
    ReadyPrompt,
    PermissionPrompt,
    Busy,
    Unknown,
}

/// Result of attempting to deliver channel input to a live tmux pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxDeliveryStatus {
    Applied,
    Busy,
    Unknown,
}

/// Explicit delivery receipt for channel-to-terminal control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxDeliveryReceipt {
    pub status: TmuxDeliveryStatus,
    pub pane_state_before: TmuxPaneState,
    pub pane_state_after: TmuxPaneState,
}

/// Validated tmux pane target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxTarget(String);

impl TmuxTarget {
    /// Create a target from a tmux target string.
    ///
    /// This intentionally accepts only the small subset needed for channel
    /// control: session/window/pane names, indexes, and tmux ids. The value is
    /// still passed as an argv item, not through a shell.
    pub fn new(target: impl Into<String>) -> anyhow::Result<Self> {
        let target = target.into();
        let trimmed = target.trim();
        if trimmed.is_empty() {
            anyhow::bail!("tmux target cannot be empty");
        }
        if trimmed.len() > 128 {
            anyhow::bail!("tmux target must be 128 bytes or fewer");
        }
        if !trimmed.chars().all(is_allowed_target_char) {
            anyhow::bail!("tmux target contains unsupported characters");
        }
        Ok(Self(trimmed.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Minimal tmux controller for driving live terminal sessions.
///
/// This intentionally shells out to the tmux CLI to target the same named
/// socket/config used by the existing web host-terminal implementation.
#[derive(Debug, Clone)]
pub struct TmuxController {
    socket_name: Option<String>,
    config_path: Option<String>,
}

impl TmuxController {
    #[must_use]
    pub fn new() -> Self {
        Self {
            socket_name: None,
            config_path: None,
        }
    }

    /// Controller for the same tmux server used by Moltis' web host terminal.
    #[must_use]
    pub fn moltis_host_terminal() -> Self {
        Self {
            socket_name: Some(MOLTIS_HOST_TERMINAL_TMUX_SOCKET_NAME.to_string()),
            config_path: Some(MOLTIS_HOST_TERMINAL_TMUX_CONFIG_PATH.to_string()),
        }
    }

    #[must_use]
    pub fn with_socket_name(socket_name: impl Into<String>) -> Self {
        Self {
            socket_name: Some(socket_name.into()),
            config_path: None,
        }
    }

    #[must_use]
    pub fn with_config_path(mut self, config_path: impl Into<String>) -> Self {
        self.config_path = Some(config_path.into());
        self
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        which::which("tmux").is_ok()
    }

    pub fn capture_pane(&self, target: &TmuxTarget) -> anyhow::Result<String> {
        let output = self
            .tmux_command()
            .args(["capture-pane", "-p", "-J", "-t", target.as_str()])
            .output()?;
        command_stdout(output, "capture tmux pane")
    }

    pub fn send_text_with_receipt(
        &self,
        target: &TmuxTarget,
        text: &str,
    ) -> anyhow::Result<TmuxDeliveryReceipt> {
        if text.trim().is_empty() {
            anyhow::bail!("tmux input cannot be empty");
        }
        let before = self.capture_pane(target)?;
        let pane_state_before = classify_pane_state(&before);
        if pane_state_before == TmuxPaneState::Busy {
            return Ok(TmuxDeliveryReceipt {
                status: TmuxDeliveryStatus::Busy,
                pane_state_before,
                pane_state_after: pane_state_before,
            });
        }

        let buffer_name = tmux_buffer_name();
        let set_buffer = self
            .tmux_command()
            .args(["set-buffer", "-b", &buffer_name, "--", text])
            .output()?;
        command_success(&set_buffer, "set tmux buffer")?;

        let paste = self
            .tmux_command()
            .args([
                "paste-buffer",
                "-d",
                "-b",
                &buffer_name,
                "-t",
                target.as_str(),
            ])
            .output()?;
        command_success(&paste, "paste tmux buffer")?;

        let enter = self
            .tmux_command()
            .args(["send-keys", "-t", target.as_str(), "Enter"])
            .output()?;
        command_success(&enter, "send tmux enter")?;

        let after = self.capture_pane(target)?;
        let pane_state_after = classify_pane_state(&after);
        let status = if after != before {
            TmuxDeliveryStatus::Applied
        } else {
            TmuxDeliveryStatus::Unknown
        };

        Ok(TmuxDeliveryReceipt {
            status,
            pane_state_before,
            pane_state_after,
        })
    }

    fn tmux_command(&self) -> Command {
        let mut command = Command::new("tmux");
        if let Some(socket_name) = &self.socket_name {
            command.args(["-L", socket_name]);
        }
        if let Some(config_path) = &self.config_path {
            command.args(["-f", config_path]);
        }
        command
    }
}

impl Default for TmuxController {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use]
pub fn classify_pane_state(output: &str) -> TmuxPaneState {
    let visible = strip_ansi(output).to_lowercase();
    let trimmed = visible.trim_end();
    if trimmed.is_empty() {
        return TmuxPaneState::Unknown;
    }
    if trimmed.contains("do you want to proceed")
        || trimmed.contains("allow this action")
        || trimmed.contains("allow this operation")
        || trimmed.contains("grant permission")
    {
        return TmuxPaneState::PermissionPrompt;
    }
    if trimmed.ends_with("$") || trimmed.ends_with("#") || trimmed.ends_with(">") {
        return TmuxPaneState::ReadyPrompt;
    }
    if trimmed.contains("thinking")
        || trimmed.contains("esc to interrupt")
        || trimmed.contains("ctrl-c")
    {
        return TmuxPaneState::Busy;
    }
    TmuxPaneState::Unknown
}

fn command_success(output: &std::process::Output, action: &str) -> anyhow::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        anyhow::bail!("failed to {action}: {}", output.status);
    }
    anyhow::bail!("failed to {action}: {stderr}");
}

fn command_stdout(output: std::process::Output, action: &str) -> anyhow::Result<String> {
    command_success(&output, action)?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn is_allowed_target_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '@' | '%' | '/')
}

fn strip_ansi(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

fn tmux_buffer_name() -> String {
    let nanos = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    format!("moltis-channel-{nanos}-{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_accepts_common_tmux_targets() {
        for target in ["moltis", "moltis:0", "moltis:0.1", "@12", "%34"] {
            let parsed = match TmuxTarget::new(target) {
                Ok(parsed) => parsed,
                Err(error) => panic!("expected valid tmux target {target}: {error}"),
            };
            assert_eq!(parsed.as_str(), target);
        }
    }

    #[test]
    fn target_rejects_shell_like_values() {
        for target in ["", "moltis;rm -rf /", "$(touch x)", "name with spaces"] {
            assert!(TmuxTarget::new(target).is_err());
        }
    }

    #[test]
    fn classify_ready_shell_prompt() {
        assert_eq!(classify_pane_state("~/repo $"), TmuxPaneState::ReadyPrompt);
        assert_eq!(classify_pane_state("root #"), TmuxPaneState::ReadyPrompt);
    }

    #[test]
    fn classify_permission_prompt() {
        assert_eq!(
            classify_pane_state("Do you want to proceed?\n> 1. Yes"),
            TmuxPaneState::PermissionPrompt
        );
        assert_eq!(
            classify_pane_state("Allow this action?\n1. Yes"),
            TmuxPaneState::PermissionPrompt
        );
    }

    #[test]
    fn classify_permission_denied_as_ready_prompt() {
        assert_eq!(
            classify_pane_state("ls /root\nPermission denied\n~/repo $"),
            TmuxPaneState::ReadyPrompt
        );
    }

    #[test]
    fn classify_busy_pane() {
        assert_eq!(
            classify_pane_state("Thinking... press Esc to interrupt"),
            TmuxPaneState::Busy
        );
    }

    #[test]
    fn classify_running_text_as_unknown_without_interrupt_hint() {
        assert_eq!(
            classify_pane_state("Tests are running"),
            TmuxPaneState::Unknown
        );
    }

    #[test]
    fn ansi_sequences_do_not_hide_state() {
        assert_eq!(
            classify_pane_state("\u{1b}[32m~/repo $\u{1b}[0m"),
            TmuxPaneState::ReadyPrompt
        );
    }
}
