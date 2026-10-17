//! Interactive startup: open the web UI in the local browser, offer to trust
//! the local CA, and keep the URL visible until onboarding is finished.

use std::{
    net::IpAddr,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use {
    moltis_config::OpenBrowser,
    moltis_gateway::state::GatewayState,
    secrecy::ExposeSecret,
    tracing::{info, warn},
};

/// How often the onboarding watcher re-checks setup progress.
const WATCH_INTERVAL: Duration = Duration::from_secs(2);
/// How often to repeat the "finish setup" hint when no status line is pinned.
const REMINDER_INTERVAL: Duration = Duration::from_secs(60);
/// Let the post-listen burst of background startup logs pass before printing
/// the "next step" box, so it is the last thing on screen.
const NEXT_STEP_DELAY: Duration = Duration::from_millis(1500);

/// A status line pinned below the scrolling logs (provided by the CLI when
/// it owns an interactive terminal).
pub trait StartupStatus: Send + Sync {
    fn set(&self, message: &str);
    fn clear(&self);
}

/// Interactive startup options supplied by the CLI.
#[derive(Clone, Default)]
pub struct LaunchOptions {
    /// Overrides `server.open_browser` (`--open` / `--no-open`).
    pub open_browser: Option<OpenBrowser>,
    /// Pinned status line, when stdout is an interactive terminal.
    pub status: Option<Arc<dyn StartupStatus>>,
}

/// How the process was launched: is there a person at this machine's
/// desktop watching the terminal?
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LaunchContext {
    terminal: bool,
    stdin_terminal: bool,
    ssh: bool,
    ci: bool,
    container: bool,
    service: bool,
    display: bool,
}

impl LaunchContext {
    pub(crate) fn detect() -> Self {
        use std::io::IsTerminal;
        Self::from_env(
            env_var,
            std::io::stdout().is_terminal() || std::io::stderr().is_terminal(),
            std::io::stdin().is_terminal(),
            Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists(),
        )
    }

    fn from_env(
        var: impl Fn(&str) -> Option<String>,
        terminal: bool,
        stdin_terminal: bool,
        container_marker: bool,
    ) -> Self {
        let any = |keys: &[&str]| keys.iter().any(|key| var(key).is_some());
        Self {
            terminal,
            stdin_terminal,
            ssh: any(&["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]),
            ci: any(&["CI"]),
            container: container_marker || any(&["container", "MOLTIS_DEPLOY_PLATFORM"]),
            // systemd sets INVOCATION_ID for every unit it starts.
            service: any(&["INVOCATION_ID"]),
            display: !cfg!(target_os = "linux")
                || any(&["DISPLAY", "WAYLAND_DISPLAY", "WSL_DISTRO_NAME"]),
        }
    }

    /// A person is at this machine's desktop and can see the terminal.
    pub(crate) fn local_desktop(&self) -> bool {
        self.terminal && self.display && !self.ssh && !self.ci && !self.container && !self.service
    }

    /// The person can also answer a yes/no prompt.
    pub(crate) fn can_prompt(&self) -> bool {
        self.local_desktop() && self.stdin_terminal
    }

    fn ssh(&self) -> bool {
        self.ssh
    }
}

fn env_var(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

pub(crate) fn should_open_browser(
    policy: OpenBrowser,
    onboarded: bool,
    context: &LaunchContext,
) -> bool {
    context.local_desktop()
        && match policy {
            OpenBrowser::FirstRun => !onboarded,
            OpenBrowser::Always => true,
            OpenBrowser::Never => false,
        }
}

/// Host a local browser should use to reach a gateway bound to `bind`.
///
/// Only binds that `localhost` actually reaches (the standard loopback
/// addresses and wildcards) become `localhost`. Other loopback addresses
/// such as `127.1.2.3` keep their literal form, since the listener does not
/// accept connections to `127.0.0.1` / `::1`.
pub(crate) fn browse_host(bind: &str) -> String {
    let host = bind.trim().trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return "localhost".to_string();
    }
    match host.parse::<IpAddr>() {
        Ok(ip) if ip.is_unspecified() || ip == IpAddr::from([127, 0, 0, 1]) => {
            "localhost".to_string()
        },
        Ok(IpAddr::V6(ip)) if ip.is_loopback() => "localhost".to_string(),
        Ok(IpAddr::V6(ip)) => format!("[{ip}]"),
        _ => host.to_string(),
    }
}

/// URL that lands the user on the right page. The setup code travels in the
/// fragment, which browsers never send to the server, and the onboarding UI
/// strips it from the address bar after reading it.
pub(crate) fn launch_url(base: &str, onboarded: bool, setup_code: Option<&str>) -> String {
    match (onboarded, setup_code) {
        (true, _) => base.to_string(),
        (false, Some(code)) => format!("{base}/onboarding#setup={code}"),
        (false, None) => format!("{base}/onboarding"),
    }
}

/// `ssh -L` command that makes a loopback-bound gateway reachable from the
/// machine the user is SSHing from.
fn ssh_forward_command(var: impl Fn(&str) -> Option<String>, port: u16) -> Option<String> {
    let connection = var("SSH_CONNECTION")?;
    // SSH_CONNECTION = "<client ip> <client port> <server ip> <server port>"
    let server_ip = connection.split_whitespace().nth(2)?;
    let target = match var("USER").or_else(|| var("LOGNAME")) {
        Some(user) => format!("{user}@{server_ip}"),
        None => server_ip.to_string(),
    };
    Some(format!("ssh -L {port}:localhost:{port} {target}"))
}

/// What happened when we tried to open the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowserOutcome {
    Opened,
    Failed,
    Skipped,
}

pub(crate) struct NextStep<'a> {
    pub base_url: &'a str,
    pub onboarded: bool,
    pub setup_code: Option<&'a str>,
    pub browser: BrowserOutcome,
    pub ssh_forward: Option<&'a str>,
}

/// Lines for the boxed "next step" summary printed after startup.
pub(crate) fn next_step_lines(step: &NextStep<'_>) -> Vec<String> {
    let page = launch_url(step.base_url, step.onboarded, None);
    let mut lines = Vec::new();
    match (step.browser, step.ssh_forward) {
        (BrowserOutcome::Opened, _) => lines.push(format!("opened {page} in your browser")),
        (_, Some(forward)) => {
            lines.push("connected over SSH: forward the port from your computer first".into());
            lines.push(format!("  {forward}"));
            lines.push(format!("then open {page}"));
        },
        _ => lines.push(format!("open {page} in your browser")),
    }
    if !step.onboarded {
        lines.push("to finish setting up Moltis".into());
    }
    if step.setup_code.is_some() {
        lines.push(if step.browser == BrowserOutcome::Opened {
            "the setup code is already filled in".into()
        } else {
            "setup code: see the startup banner above".into()
        });
    }
    lines
}

/// Text for the pinned status line.
pub(crate) fn status_message(base_url: &str, onboarded: bool, setup_code: Option<&str>) -> String {
    if onboarded {
        return format!("● Moltis running at {base_url}  ·  Ctrl+C to stop");
    }
    let page = launch_url(base_url, false, None);
    match setup_code {
        Some(code) => format!("▶ Finish setup at {page}  ·  setup code {code}"),
        None => format!("▶ Finish setup at {page}"),
    }
}

/// Log `lines` inside a box so they stand out in the log stream.
pub(crate) fn log_box(lines: &[String]) {
    let width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
        + 4;
    info!("┌{}┐", "─".repeat(width));
    for line in lines {
        let pad = width - 2 - line.chars().count();
        info!("│  {line}{}│", " ".repeat(pad));
    }
    info!("└{}┘", "─".repeat(width));
}

async fn current_setup_code(state: &GatewayState) -> Option<String> {
    state
        .inner
        .read()
        .await
        .setup_code
        .as_ref()
        .map(|code| code.expose_secret().clone())
}

async fn is_onboarded() -> bool {
    tokio::fs::try_exists(moltis_config::onboarded_sentinel_path())
        .await
        .unwrap_or(false)
}

/// Inputs for [`spawn_after_listen`].
pub(crate) struct AfterListen {
    pub base_url: String,
    pub port: u16,
    pub bind_is_loopback: bool,
    pub policy: OpenBrowser,
    pub status: Option<Arc<dyn StartupStatus>>,
    pub state: Arc<GatewayState>,
}

/// Once the listener is bound: open the browser if appropriate, print the
/// "next step" box, and keep the pinned status line (or periodic reminders)
/// current until onboarding is done.
pub(crate) fn spawn_after_listen(args: AfterListen) {
    tokio::spawn(async move {
        let context = LaunchContext::detect();
        let onboarded = is_onboarded().await;
        let setup_code = current_setup_code(&args.state).await;

        let browser = if should_open_browser(args.policy, onboarded, &context) {
            open_browser(launch_url(&args.base_url, onboarded, setup_code.as_deref())).await
        } else {
            BrowserOutcome::Skipped
        };
        let ssh_forward = (context.ssh() && args.bind_is_loopback)
            .then(|| ssh_forward_command(env_var, args.port))
            .flatten();

        tokio::time::sleep(NEXT_STEP_DELAY).await;
        log_box(&next_step_lines(&NextStep {
            base_url: &args.base_url,
            onboarded,
            setup_code: setup_code.as_deref(),
            browser,
            ssh_forward: ssh_forward.as_deref(),
        }));

        watch_onboarding(&args).await;
    });
}

#[tracing::instrument(skip_all)]
async fn open_browser(url: String) -> BrowserOutcome {
    let result = tokio::task::spawn_blocking(move || webbrowser::open(&url)).await;
    match result {
        Ok(Ok(())) => BrowserOutcome::Opened,
        Ok(Err(error)) => {
            warn!(%error, "could not open a browser");
            BrowserOutcome::Failed
        },
        Err(error) => {
            warn!(%error, "browser launch task failed");
            BrowserOutcome::Failed
        },
    }
}

async fn watch_onboarding(args: &AfterListen) {
    let mut shown = String::new();
    let mut last_reminder = Instant::now();
    loop {
        let onboarded = is_onboarded().await;
        let setup_code = current_setup_code(&args.state).await;
        match args.status.as_ref() {
            Some(status) => {
                let message = status_message(&args.base_url, onboarded, setup_code.as_deref());
                if message != shown {
                    status.set(&message);
                    shown = message;
                }
                if onboarded {
                    return;
                }
            },
            None if onboarded => return,
            // Logs reach the web UI log buffer and exporters, so reminders
            // never repeat the setup code (it is in the startup banner).
            None if last_reminder.elapsed() >= REMINDER_INTERVAL => {
                info!("{}", status_message(&args.base_url, false, None));
                last_reminder = Instant::now();
            },
            None => {},
        }
        tokio::time::sleep(WATCH_INTERVAL).await;
    }
}

/// How the user answered the `[Y/n]` CA trust prompt.
#[cfg(any(feature = "tls", test))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TrustAnswer {
    Accept,
    Decline,
    /// EOF (Ctrl+D) or a read error: ask again next time.
    Cancel,
}

/// Interpret the prompt's `read_line` result. Only a submitted line counts
/// as an answer, so an empty line accepts the default but EOF cancels.
#[cfg(any(feature = "tls", test))]
pub(crate) fn trust_answer(read: std::io::Result<usize>, answer: &str) -> TrustAnswer {
    if !matches!(read, Ok(bytes) if bytes > 0) {
        return TrustAnswer::Cancel;
    }
    match answer.trim().to_ascii_lowercase().as_str() {
        "" | "y" | "yes" => TrustAnswer::Accept,
        _ => TrustAnswer::Decline,
    }
}

/// Offer to install the local CA into the OS and browser trust stores, once
/// per CA, when someone at this desktop can answer.
#[cfg(feature = "tls")]
pub(crate) async fn offer_ca_trust(ca_path: &Path) {
    use moltis_tls::trust::{self, TrustDecision};

    if !LaunchContext::detect().can_prompt() || trust::recorded_decision(ca_path).is_some() {
        return;
    }
    let ca_path = ca_path.to_path_buf();
    let outcome = tokio::task::spawn_blocking(move || {
        eprint!(
            "\nTrust the Moltis local CA so your browser opens https:// pages without warnings?\n\
             It only covers localhost, .local names and private network addresses.\n\
             Your OS may ask for your password. [Y/n] "
        );
        let mut answer = String::new();
        let read = std::io::stdin().read_line(&mut answer);
        match trust_answer(read, &answer) {
            TrustAnswer::Accept => Some(trust::install_ca(&ca_path)),
            TrustAnswer::Decline => {
                let _ = trust::record_decision(&ca_path, TrustDecision::Declined);
                None
            },
            TrustAnswer::Cancel => {
                eprintln!();
                None
            },
        }
    })
    .await;

    match outcome {
        Ok(Some(report)) => {
            for step in &report.steps {
                match &step.result {
                    Ok(()) => info!(store = %step.store, "trusted Moltis local CA"),
                    Err(error) => {
                        warn!(store = %step.store, %error, "could not trust Moltis local CA")
                    },
                }
            }
            for hint in &report.hints {
                info!("{hint}");
            }
            if report.any_installed() {
                info!("restart your browser if it was already open");
            }
        },
        Ok(None) => info!("skipped trusting the local CA; run `moltis trust-ca` later"),
        Err(error) => warn!(%error, "CA trust prompt failed"),
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn context(vars: &[(&str, &str)], terminal: bool) -> LaunchContext {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        LaunchContext::from_env(
            |key| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()),
            terminal,
            terminal,
            false,
        )
    }

    fn desktop() -> Vec<(&'static str, &'static str)> {
        vec![("DISPLAY", ":0")]
    }

    #[test]
    fn desktop_terminal_can_open_and_prompt() {
        let ctx = context(&desktop(), true);
        assert!(ctx.local_desktop());
        assert!(ctx.can_prompt());
    }

    #[test]
    fn remote_or_unattended_sessions_never_open_a_browser() {
        for var in [
            ("SSH_CONNECTION", "10.0.0.2 5000 10.0.0.1 22"),
            ("SSH_TTY", "/dev/pts/1"),
            ("CI", "true"),
            ("container", "podman"),
            ("MOLTIS_DEPLOY_PLATFORM", "fly"),
            ("INVOCATION_ID", "abc"),
        ] {
            let mut vars = desktop();
            vars.push(var);
            assert!(!context(&vars, true).local_desktop(), "{var:?}");
        }
        assert!(!context(&desktop(), false).local_desktop(), "no terminal");
    }

    #[test]
    fn container_marker_file_disables_browser() {
        let display = |key: &str| (key == "DISPLAY").then(|| ":0".to_string());
        assert!(LaunchContext::from_env(display, true, true, false).local_desktop());
        assert!(!LaunchContext::from_env(display, true, true, true).local_desktop());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_without_display_is_headless() {
        assert!(!context(&[], true).local_desktop());
        assert!(context(&[("WAYLAND_DISPLAY", "wayland-0")], true).local_desktop());
    }

    #[test]
    fn policy_controls_when_browser_opens() {
        let ctx = context(&desktop(), true);
        assert!(should_open_browser(OpenBrowser::FirstRun, false, &ctx));
        assert!(!should_open_browser(OpenBrowser::FirstRun, true, &ctx));
        assert!(should_open_browser(OpenBrowser::Always, true, &ctx));
        assert!(!should_open_browser(OpenBrowser::Never, false, &ctx));
        let headless = context(&[("CI", "1")], true);
        assert!(!should_open_browser(OpenBrowser::Always, false, &headless));
    }

    #[test]
    fn browse_host_maps_loopback_and_wildcard_to_localhost() {
        assert_eq!(browse_host("127.0.0.1"), "localhost");
        assert_eq!(browse_host("::1"), "localhost");
        assert_eq!(browse_host("0.0.0.0"), "localhost");
        assert_eq!(browse_host("::"), "localhost");
        assert_eq!(browse_host("192.168.1.5"), "192.168.1.5");
        assert_eq!(browse_host("fd00::5"), "[fd00::5]");
        assert_eq!(browse_host("[::1]"), "localhost");
        assert_eq!(browse_host("localhost"), "localhost");
    }

    #[test]
    fn trust_answer_treats_eof_as_cancel_not_default_yes() {
        assert_eq!(trust_answer(Ok(0), ""), TrustAnswer::Cancel);
        assert_eq!(
            trust_answer(Err(std::io::Error::other("closed")), ""),
            TrustAnswer::Cancel
        );
        assert_eq!(trust_answer(Ok(1), "\n"), TrustAnswer::Accept);
        assert_eq!(trust_answer(Ok(4), "Yes\n"), TrustAnswer::Accept);
        assert_eq!(trust_answer(Ok(2), "n\n"), TrustAnswer::Decline);
        assert_eq!(trust_answer(Ok(6), "maybe\n"), TrustAnswer::Decline);
    }

    #[test]
    fn browse_host_keeps_nonstandard_loopback_binds() {
        // The listener only accepts the exact address, so `localhost` would
        // not reach it.
        assert_eq!(browse_host("127.1.2.3"), "127.1.2.3");
        assert_eq!(browse_host("moltis.localhost"), "moltis.localhost");
    }

    #[test]
    fn launch_url_carries_setup_code_in_fragment_only_before_onboarding() {
        let base = "http://localhost:4000";
        assert_eq!(
            launch_url(base, false, Some("123456")),
            "http://localhost:4000/onboarding#setup=123456"
        );
        assert_eq!(
            launch_url(base, false, None),
            "http://localhost:4000/onboarding"
        );
        assert_eq!(launch_url(base, true, Some("123456")), base);
    }

    #[test]
    fn ssh_forward_command_uses_server_address_and_user() {
        let vars = |key: &str| match key {
            "SSH_CONNECTION" => Some("203.0.113.9 52000 10.0.0.7 22".to_string()),
            "USER" => Some("ada".to_string()),
            _ => None,
        };
        assert_eq!(
            ssh_forward_command(vars, 4000).unwrap(),
            "ssh -L 4000:localhost:4000 ada@10.0.0.7"
        );
        assert_eq!(ssh_forward_command(|_| None, 4000), None);
    }

    #[test]
    fn next_step_explains_ssh_forwarding() {
        let lines = next_step_lines(&NextStep {
            base_url: "http://localhost:4000",
            onboarded: false,
            setup_code: Some("123456"),
            browser: BrowserOutcome::Skipped,
            ssh_forward: Some("ssh -L 4000:localhost:4000 ada@host"),
        });
        assert!(
            lines
                .iter()
                .any(|l| l.contains("ssh -L 4000:localhost:4000"))
        );
        assert!(lines.contains(&"then open http://localhost:4000/onboarding".to_string()));
        assert!(lines.contains(&"setup code: see the startup banner above".to_string()));
        assert!(
            !lines.iter().any(|l| l.contains("123456")),
            "code must not be logged"
        );
    }

    #[test]
    fn next_step_reports_opened_browser() {
        let lines = next_step_lines(&NextStep {
            base_url: "https://localhost:4000",
            onboarded: true,
            setup_code: None,
            browser: BrowserOutcome::Opened,
            ssh_forward: None,
        });
        assert_eq!(lines, vec![
            "opened https://localhost:4000 in your browser".to_string()
        ]);
    }

    #[test]
    fn status_message_tracks_onboarding_progress() {
        let base = "http://localhost:4000";
        assert_eq!(
            status_message(base, false, Some("123456")),
            "▶ Finish setup at http://localhost:4000/onboarding  ·  setup code 123456"
        );
        assert_eq!(
            status_message(base, false, None),
            "▶ Finish setup at http://localhost:4000/onboarding"
        );
        assert_eq!(
            status_message(base, true, Some("123456")),
            "● Moltis running at http://localhost:4000  ·  Ctrl+C to stop"
        );
    }
}
