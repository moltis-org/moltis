//! Reach a remote daemon through an SSH port forward.
//!
//! This runs the system `ssh` rather than an SSH crate on purpose: it honours
//! the user's `~/.ssh/config`, agent, hardware keys and known-hosts exactly as
//! their terminal does. `BatchMode` keeps it from prompting, since there is no
//! terminal to prompt on. The daemon keeps listening on its loopback address
//! only; the tunnel lands there, and the client still presents its API key.

use std::{path::PathBuf, process::Stdio, time::Duration};

use {
    serde::{Deserialize, Serialize},
    tokio::{io::AsyncReadExt, net::TcpStream, process::Child},
};

use crate::{
    endpoint::Endpoint,
    error::{Error, Result},
    tls::TlsTrust,
};

/// How long to wait for the forward to start accepting connections.
const OPEN_TIMEOUT: Duration = Duration::from_secs(20);

/// A remote daemon reached over SSH.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTarget {
    /// `host`, `user@host` or an alias from `~/.ssh/config`.
    pub destination: String,
    /// Port the daemon listens on, on the remote machine's loopback.
    pub remote_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
    /// Whether the remote daemon serves TLS (the default).
    #[serde(default = "default_true")]
    pub tls: bool,
}

fn default_true() -> bool {
    true
}

impl SshTarget {
    fn validate(&self) -> Result<()> {
        let destination = self.destination.trim();
        // A leading dash would be parsed by ssh as an option.
        if destination.is_empty()
            || destination.starts_with('-')
            || destination
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(Error::InvalidUrl(format!(
                "invalid SSH destination {:?}",
                self.destination
            )));
        }
        if self.remote_port == 0 {
            return Err(Error::InvalidUrl("remote port must not be 0".into()));
        }
        Ok(())
    }

    /// Arguments for `ssh` forwarding `local_port` to the daemon.
    fn args(&self, local_port: u16) -> Vec<String> {
        let mut args: Vec<String> = [
            "-N",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
            "-L",
        ]
        .map(String::from)
        .into();
        args.push(format!(
            "127.0.0.1:{local_port}:127.0.0.1:{}",
            self.remote_port
        ));
        if let Some(port) = self.ssh_port {
            args.extend(["-p".into(), port.to_string()]);
        }
        if let Some(identity) = &self.identity_file {
            args.extend(["-i".into(), identity.display().to_string()]);
        }
        args.extend(["--".into(), self.destination.trim().into()]);
        args
    }
}

/// A running forward. Dropping it stops `ssh`.
#[derive(Debug)]
pub struct SshTunnel {
    child: Child,
    local_port: u16,
    tls: bool,
}

impl SshTunnel {
    /// Start `ssh` and wait until the forwarded port accepts connections.
    pub async fn open(target: &SshTarget) -> Result<Self> {
        Self::open_with_program(target, "ssh").await
    }

    async fn open_with_program(target: &SshTarget, program: &str) -> Result<Self> {
        target.validate()?;
        let local_port = free_local_port()?;
        let mut child = tokio::process::Command::new(program)
            .args(target.args(local_port))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::Connect(format!("could not run {program}: {e}")))?;

        let deadline = tokio::time::Instant::now() + OPEN_TIMEOUT;
        loop {
            if let Some(status) = child.try_wait()? {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr).await;
                }
                let detail = stderr.trim();
                return Err(Error::Connect(if detail.is_empty() {
                    format!("ssh to {} exited with {status}", target.destination)
                } else {
                    format!("ssh to {}: {detail}", target.destination)
                }));
            }
            if TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
                return Ok(Self {
                    child,
                    local_port,
                    tls: target.tls,
                });
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.start_kill();
                return Err(Error::Timeout);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[must_use]
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Endpoint of the forwarded daemon. The certificate is the remote one,
    /// issued for `localhost`, so `tls` is usually a pinned fingerprint.
    pub fn endpoint(&self, tls: TlsTrust) -> Result<Endpoint> {
        let scheme = if self.tls {
            "https"
        } else {
            "http"
        };
        Endpoint::parse(&format!("{scheme}://localhost:{}", self.local_port), tls)
    }

    /// Whether `ssh` is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Stop `ssh` and wait for it to exit.
    pub async fn close(mut self) {
        let _ = self.child.kill().await;
    }
}

fn free_local_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn target(destination: &str) -> SshTarget {
        SshTarget {
            destination: destination.into(),
            remote_port: 18789,
            ssh_port: None,
            identity_file: None,
            tls: true,
        }
    }

    #[test]
    fn builds_forward_args() {
        let mut t = target("me@studio");
        t.ssh_port = Some(2222);
        t.identity_file = Some("/k/id".into());
        let args = t.args(5000);
        assert!(args.contains(&"BatchMode=yes".to_string()));
        assert!(args.contains(&"127.0.0.1:5000:127.0.0.1:18789".to_string()));
        assert_eq!(&args[args.len() - 2..], ["--", "me@studio"]);
        let p = args.iter().position(|a| a == "-p").unwrap();
        assert_eq!(args[p + 1], "2222");
    }

    #[test]
    fn rejects_option_injection() {
        assert!(target("-oProxyCommand=evil").validate().is_err());
        assert!(target("host name").validate().is_err());
        assert!(target("").validate().is_err());
        assert!(
            SshTarget {
                remote_port: 0,
                ..target("h")
            }
            .validate()
            .is_err()
        );
        assert!(target("studio").validate().is_ok());
    }

    #[tokio::test]
    async fn failing_ssh_reports_error() {
        let error = SshTunnel::open_with_program(&target("studio"), "false")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Connect(_)), "{error:?}");
    }

    #[tokio::test]
    async fn missing_program_reports_error() {
        let error = SshTunnel::open_with_program(&target("studio"), "/nonexistent/ssh")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("could not run"));
    }

    #[test]
    fn serde_defaults_tls() {
        let t: SshTarget = serde_json::from_str(r#"{"destination":"h","remote_port":1}"#).unwrap();
        assert!(t.tls);
    }
}
