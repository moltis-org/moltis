use std::{ffi::OsString, process::Stdio, time::Duration};

use {
    async_trait::async_trait,
    moltis_common::process_tree::OwnedProcessTree,
    tokio::{io::AsyncReadExt, process::Command},
};

use crate::{HimalayaBackend, HimalayaConnectorError, Result};

const JSON_OUTPUT_CAP: usize = 4 * 1024 * 1024;
const RAW_OUTPUT_CAP: usize = 10 * 1024 * 1024 + 1;
const STDERR_CAP: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HimalayaInvocation {
    Version,
    AccountList,
    MailboxList {
        account: String,
        backend: HimalayaBackend,
    },
    EnvelopePage {
        account: String,
        backend: HimalayaBackend,
        mailbox: String,
        query: Option<String>,
        page: u32,
        page_size: u32,
    },
    ImapStatus {
        account: String,
        mailbox: String,
    },
    MessageReadRaw {
        account: String,
        backend: HimalayaBackend,
        mailbox: String,
        id: String,
    },
}

impl HimalayaInvocation {
    #[must_use]
    pub fn argv(&self) -> Vec<OsString> {
        match self {
            Self::Version => vec!["--version".into()],
            Self::AccountList => vec!["--json".into(), "account".into(), "list".into()],
            Self::MailboxList { account, backend } => vec![
                "--json".into(),
                format!("--account={account}").into(),
                format!("--backend={}", backend.as_str()).into(),
                "mailbox".into(),
                "list".into(),
            ],
            Self::EnvelopePage {
                account,
                backend,
                mailbox,
                query,
                page,
                page_size,
            } => {
                let mut args = vec![
                    OsString::from("--json"),
                    format!("--account={account}").into(),
                    format!("--backend={}", backend.as_str()).into(),
                    OsString::from("envelope"),
                    OsString::from(if query.is_some() {
                        "search"
                    } else {
                        "list"
                    }),
                    format!("--mailbox={mailbox}").into(),
                    format!("--page={page}").into(),
                    format!("--page-size={page_size}").into(),
                ];
                if let Some(query) = query {
                    args.push("--".into());
                    args.push(query.into());
                }
                args
            },
            Self::ImapStatus { account, mailbox } => vec![
                "--json".into(),
                format!("--account={account}").into(),
                "--backend=imap".into(),
                "imap".into(),
                "status".into(),
                format!("--mailbox={mailbox}").into(),
            ],
            Self::MessageReadRaw {
                account,
                backend,
                mailbox,
                id,
            } => vec![
                format!("--account={account}").into(),
                format!("--backend={}", backend.as_str()).into(),
                "message".into(),
                "read".into(),
                format!("--mailbox={mailbox}").into(),
                "--raw".into(),
                "--".into(),
                id.into(),
            ],
        }
    }

    const fn output_cap(&self) -> usize {
        match self {
            Self::MessageReadRaw { .. } => RAW_OUTPUT_CAP,
            _ => JSON_OUTPUT_CAP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HimalayaOutput {
    pub stdout: Vec<u8>,
    pub stdout_truncated: bool,
}

impl HimalayaOutput {
    #[must_use]
    pub fn complete(stdout: impl Into<Vec<u8>>) -> Self {
        Self {
            stdout: stdout.into(),
            stdout_truncated: false,
        }
    }
}

#[async_trait]
pub trait HimalayaRunner: Send + Sync {
    async fn run(&self, invocation: &HimalayaInvocation) -> Result<HimalayaOutput>;
}

#[derive(Debug, Clone)]
pub struct ProcessHimalayaRunner {
    timeout: Duration,
}

impl Default for ProcessHimalayaRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
        }
    }
}

impl ProcessHimalayaRunner {
    #[must_use]
    pub const fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }

    async fn execute(&self, invocation: &HimalayaInvocation) -> Result<HimalayaOutput> {
        let executable =
            which::which("himalaya").map_err(|_| HimalayaConnectorError::ExecutableNotFound)?;
        let mut command = Command::new(executable);
        command
            .args(invocation.argv())
            .env_clear()
            .envs(allowed_environment())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = OwnedProcessTree::spawn(command).map_err(HimalayaConnectorError::Spawn)?;
        let stdout = child.take_stdout().ok_or_else(|| {
            HimalayaConnectorError::Response("failed to capture command output".to_owned())
        })?;
        let stderr = child.take_stderr().ok_or_else(|| {
            HimalayaConnectorError::Response("failed to capture command diagnostics".to_owned())
        })?;
        let execution = async {
            tokio::try_join!(
                child.wait(),
                drain_capped(stdout, invocation.output_cap()),
                drain_capped(stderr, STDERR_CAP),
            )
        };
        let result = tokio::time::timeout(self.timeout, execution).await;
        let (status, stdout, _stderr) = match result {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => return Err(HimalayaConnectorError::Spawn(error)),
            Err(_) => {
                let _ = child.kill().await;
                return Err(HimalayaConnectorError::Timeout);
            },
        };
        if !status.success() {
            return Err(HimalayaConnectorError::CommandFailed);
        }
        Ok(HimalayaOutput {
            stdout: stdout.bytes,
            stdout_truncated: stdout.truncated,
        })
    }
}

fn allowed_environment() -> impl Iterator<Item = (&'static str, OsString)> {
    const KEYS: &[&str] = &[
        "HOME",
        "USERPROFILE",
        "PATH",
        "LANG",
        "LC_ALL",
        "TMPDIR",
        "TMP",
        "TEMP",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "GNUPGHOME",
        "GPG_AGENT_INFO",
        "SSH_AUTH_SOCK",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "SYSTEMROOT",
    ];
    KEYS.iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (*key, value)))
}

#[async_trait]
impl HimalayaRunner for ProcessHimalayaRunner {
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    async fn run(&self, invocation: &HimalayaInvocation) -> Result<HimalayaOutput> {
        self.execute(invocation).await
    }
}

struct CappedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn drain_capped(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    cap: usize,
) -> std::io::Result<CappedOutput> {
    let mut retained = Vec::with_capacity(cap.min(64 * 1024));
    let mut truncated = false;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let remaining = cap.saturating_sub(retained.len());
        let copy = count.min(remaining);
        retained.extend_from_slice(&buffer[..copy]);
        truncated |= copy < count;
    }
    Ok(CappedOutput {
        bytes: retained,
        truncated,
    })
}

pub fn validate_v2_version(output: &[u8]) -> Result<()> {
    let output =
        std::str::from_utf8(output).map_err(|_| HimalayaConnectorError::UnsupportedVersion)?;
    let major = output
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .find(|part| part.contains('.'))
        .and_then(|version| version.split('.').next())
        .and_then(|major| major.parse::<u32>().ok());
    if major != Some(2) {
        return Err(HimalayaConnectorError::UnsupportedVersion);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: Vec<OsString>) -> Vec<String> {
        values
            .iter()
            .map(|value| value.to_string_lossy())
            .map(|value| value.into_owned())
            .collect()
    }

    #[test]
    fn argv_preserves_injection_looking_values_as_single_arguments() {
        let invocation = HimalayaInvocation::EnvelopePage {
            account: "--evil=$(touch /tmp/x)".to_owned(),
            backend: HimalayaBackend::Imap,
            mailbox: "INBOX; rm -rf /".to_owned(),
            query: Some("--all; whoami".to_owned()),
            page: 3,
            page_size: 100,
        };
        assert_eq!(strings(invocation.argv()), vec![
            "--json",
            "--account=--evil=$(touch /tmp/x)",
            "--backend=imap",
            "envelope",
            "search",
            "--mailbox=INBOX; rm -rf /",
            "--page=3",
            "--page-size=100",
            "--",
            "--all; whoami",
        ]);
    }

    #[test]
    fn raw_read_uses_positional_separator() {
        let invocation = HimalayaInvocation::MessageReadRaw {
            account: "a".to_owned(),
            backend: HimalayaBackend::Jmap,
            mailbox: "m".to_owned(),
            id: "--help".to_owned(),
        };
        assert_eq!(strings(invocation.argv()), vec![
            "--account=a",
            "--backend=jmap",
            "message",
            "read",
            "--mailbox=m",
            "--raw",
            "--",
            "--help",
        ]);
    }

    #[test]
    fn discovery_and_status_argv_are_fixed_v2_forms() {
        assert_eq!(strings(HimalayaInvocation::Version.argv()), vec![
            "--version"
        ]);
        assert_eq!(strings(HimalayaInvocation::AccountList.argv()), vec![
            "--json", "account", "list"
        ]);
        assert_eq!(
            strings(
                HimalayaInvocation::MailboxList {
                    account: "work".to_owned(),
                    backend: HimalayaBackend::M2dir,
                }
                .argv()
            ),
            vec![
                "--json",
                "--account=work",
                "--backend=m2dir",
                "mailbox",
                "list",
            ]
        );
        assert_eq!(
            strings(
                HimalayaInvocation::ImapStatus {
                    account: "work".to_owned(),
                    mailbox: "INBOX".to_owned(),
                }
                .argv()
            ),
            vec![
                "--json",
                "--account=work",
                "--backend=imap",
                "imap",
                "status",
                "--mailbox=INBOX",
            ]
        );
    }

    #[test]
    fn version_requires_major_two() {
        assert!(validate_v2_version(b"himalaya 2.1.0").is_ok());
        assert!(validate_v2_version(b"himalaya 1.9.0").is_err());
        assert!(validate_v2_version(b"unexpected").is_err());
    }
}
