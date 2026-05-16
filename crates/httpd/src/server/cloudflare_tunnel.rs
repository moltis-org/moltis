//! Cloudflare Tunnel controller.

#[cfg(feature = "cloudflare-tunnel")]
use std::{
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(feature = "cloudflare-tunnel")]
use {
    moltis_config::schema::CloudflareTunnelConfig,
    moltis_gateway::{auth_webauthn::SharedWebAuthnRegistry, state::GatewayState},
    secrecy::ExposeSecret,
    tokio::{
        io::AsyncBufReadExt,
        process::{Child, Command},
    },
    tracing::{info, warn},
};

#[cfg(feature = "cloudflare-tunnel")]
#[derive(Clone, Debug)]
pub struct CloudflareTunnelRuntimeStatus {
    pub public_url: Option<String>,
    pub hostname: Option<String>,
    pub passkey_warning: Option<String>,
}

#[cfg(feature = "cloudflare-tunnel")]
pub struct CloudflareTunnelController {
    gateway: Arc<GatewayState>,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
    runtime: Arc<tokio::sync::RwLock<Option<CloudflareTunnelRuntimeStatus>>>,
    active_tunnel: Arc<tokio::sync::Mutex<Option<CloudflareActiveTunnel>>>,
    next_tunnel_id: AtomicU64,
}

#[cfg(feature = "cloudflare-tunnel")]
struct CloudflareActiveTunnel {
    id: u64,
    child: Child,
    log_task: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "cloudflare-tunnel")]
impl CloudflareTunnelController {
    pub fn new(
        gateway: Arc<GatewayState>,
        webauthn_registry: Option<SharedWebAuthnRegistry>,
        runtime: Arc<tokio::sync::RwLock<Option<CloudflareTunnelRuntimeStatus>>>,
    ) -> Self {
        Self {
            gateway,
            webauthn_registry,
            runtime,
            active_tunnel: Arc::new(tokio::sync::Mutex::new(None)),
            next_tunnel_id: AtomicU64::new(1),
        }
    }

    pub async fn apply(
        &self,
        config: &CloudflareTunnelConfig,
        bind_addr: &str,
        port: u16,
        tls: bool,
    ) -> crate::error::Result<Option<CloudflareTunnelRuntimeStatus>> {
        let mut active_tunnel = self.active_tunnel.lock().await;
        stop_active_tunnel(active_tunnel.take()).await;
        *self.runtime.write().await = None;

        if !config.enabled {
            info!("Cloudflare Tunnel disabled");
            return Ok(None);
        }

        let token = config
            .token
            .as_ref()
            .map(|token| token.expose_secret().to_string())
            .or_else(|| std::env::var("CLOUDFLARE_TUNNEL_TOKEN").ok())
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| {
                crate::Error::Config(
                    "Cloudflare Tunnel requires cloudflare_tunnel.token or CLOUDFLARE_TUNNEL_TOKEN"
                        .into(),
                )
            })?;

        let target = cloudflared_target_url(bind_addr, port, tls);
        let mut child = Command::new("cloudflared")
            .args(cloudflared_tunnel_args(&target))
            .env("TUNNEL_TOKEN", token)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| crate::Error::Config(format!("failed to run cloudflared: {error}")))?;

        let tunnel_id = self.next_tunnel_id.fetch_add(1, Ordering::Relaxed);
        let stderr = child.stderr.take();
        let active_tunnel_for_log = Arc::clone(&self.active_tunnel);
        let runtime_for_log = Arc::clone(&self.runtime);
        let log_task = tokio::spawn(async move {
            let Some(stderr) = stderr else {
                return;
            };
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        tracing::info!(target = "cloudflared", message = %line, "cloudflared")
                    },
                    Ok(None) => break,
                    Err(error) => {
                        warn!(%error, "failed to read cloudflared stderr");
                        break;
                    },
                }
            }
            clear_runtime_if_active_tunnel_exited(
                tunnel_id,
                active_tunnel_for_log,
                runtime_for_log,
            )
            .await;
        });

        let public_url = config
            .hostname
            .as_ref()
            .map(|host| format!("https://{host}"));
        let passkey_warning = moltis_gateway::server::sync_runtime_webauthn_host_and_notice(
            &self.gateway,
            self.webauthn_registry.as_ref(),
            config.hostname.as_deref(),
            public_url.as_deref(),
            "Cloudflare Tunnel",
        )
        .await;
        let status = CloudflareTunnelRuntimeStatus {
            public_url,
            hostname: config.hostname.clone(),
            passkey_warning,
        };

        *active_tunnel = Some(CloudflareActiveTunnel {
            id: tunnel_id,
            child,
            log_task,
        });
        *self.runtime.write().await = Some(status.clone());
        info!(target = %target, "Cloudflare Tunnel started");
        Ok(Some(status))
    }

    pub async fn stop(&self) -> crate::error::Result<()> {
        let active_tunnel = self.active_tunnel.lock().await.take();
        stop_active_tunnel(active_tunnel).await;
        *self.runtime.write().await = None;
        Ok(())
    }
}

#[cfg(feature = "cloudflare-tunnel")]
async fn stop_active_tunnel(active_tunnel: Option<CloudflareActiveTunnel>) {
    if let Some(mut active_tunnel) = active_tunnel {
        if let Err(error) = active_tunnel.child.kill().await {
            warn!(%error, "failed to stop cloudflared");
        }
        let _ = active_tunnel.child.wait().await;
        active_tunnel.log_task.abort();
        info!("Cloudflare Tunnel stopped");
    }
}

#[cfg(feature = "cloudflare-tunnel")]
async fn clear_runtime_if_active_tunnel_exited(
    tunnel_id: u64,
    active_tunnel: Arc<tokio::sync::Mutex<Option<CloudflareActiveTunnel>>>,
    runtime: Arc<tokio::sync::RwLock<Option<CloudflareTunnelRuntimeStatus>>>,
) {
    let is_active = active_tunnel
        .lock()
        .await
        .as_ref()
        .is_some_and(|active_tunnel| active_tunnel.id == tunnel_id);
    if is_active {
        *runtime.write().await = None;
        warn!("Cloudflare Tunnel process exited; runtime status cleared");
    }
}

#[cfg(feature = "cloudflare-tunnel")]
fn cloudflared_tunnel_args(target: &str) -> [&str; 5] {
    ["tunnel", "--no-autoupdate", "run", "--url", target]
}

#[cfg(feature = "cloudflare-tunnel")]
fn cloudflared_target_url(bind_addr: &str, port: u16, tls: bool) -> String {
    let scheme = if tls {
        "https"
    } else {
        "http"
    };
    if bind_addr.contains(':') && !bind_addr.starts_with('[') {
        format!("{scheme}://[{bind_addr}]:{port}")
    } else {
        format!("{scheme}://{bind_addr}:{port}")
    }
}

#[cfg(feature = "cloudflare-tunnel")]
pub async fn start_for_banner(
    controller: &CloudflareTunnelController,
    config: &CloudflareTunnelConfig,
    bind_addr: &str,
    port: u16,
    tls: bool,
) -> (Option<CloudflareTunnelRuntimeStatus>, Option<String>) {
    match controller.apply(config, bind_addr, port, tls).await {
        Ok(status) => (status, None),
        Err(error) => {
            warn!(%error, "Cloudflare Tunnel failed to start; gateway will continue without it");
            (None, Some(error.to_string()))
        },
    }
}

#[cfg(feature = "cloudflare-tunnel")]
pub fn startup_lines(
    status: Option<&CloudflareTunnelRuntimeStatus>,
    startup_error: Option<&str>,
) -> Vec<String> {
    let Some(status) = status else {
        return startup_error
            .map(|error| format!("cloudflare tunnel: failed to start ({error})"))
            .into_iter()
            .collect();
    };

    let mut lines = Vec::new();
    if let Some(public_url) = status.public_url.as_ref() {
        lines.push(format!("cloudflare tunnel: {public_url}"));
    } else {
        lines.push("cloudflare tunnel: started".into());
    }
    if let Some(passkey_warning) = status.passkey_warning.as_ref() {
        lines.push(format!("cloudflare tunnel note: {passkey_warning}"));
    }
    lines
}

#[cfg(all(test, feature = "cloudflare-tunnel"))]
mod tests {
    use std::sync::Arc;

    use moltis_gateway::{auth, services::GatewayServices, state::GatewayState};

    use super::*;

    #[tokio::test]
    async fn apply_disabled_clears_runtime_without_cloudflared() -> crate::error::Result<()> {
        let runtime = Arc::new(tokio::sync::RwLock::new(Some(
            CloudflareTunnelRuntimeStatus {
                public_url: Some("https://old.example.com".to_string()),
                hostname: Some("old.example.com".to_string()),
                passkey_warning: None,
            },
        )));
        let controller = CloudflareTunnelController::new(
            GatewayState::new(auth::resolve_auth(None, None), GatewayServices::noop()),
            None,
            Arc::clone(&runtime),
        );

        let status = controller
            .apply(&CloudflareTunnelConfig::default(), "127.0.0.1", 8080, false)
            .await?;

        assert!(status.is_none());
        assert!(runtime.read().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn exited_active_tunnel_clears_runtime_status() -> crate::error::Result<()> {
        let runtime = Arc::new(tokio::sync::RwLock::new(Some(
            CloudflareTunnelRuntimeStatus {
                public_url: Some("https://moltis.example.com".to_string()),
                hostname: Some("moltis.example.com".to_string()),
                passkey_warning: None,
            },
        )));
        let active_tunnel = Arc::new(tokio::sync::Mutex::new(Some(CloudflareActiveTunnel {
            id: 7,
            child: Command::new("sh")
                .args(["-c", "true"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| {
                    crate::Error::Config(format!("failed to run test child: {error}"))
                })?,
            log_task: tokio::spawn(async {}),
        })));

        clear_runtime_if_active_tunnel_exited(7, active_tunnel, Arc::clone(&runtime)).await;

        assert!(runtime.read().await.is_none());
        Ok(())
    }

    #[test]
    fn startup_lines_reports_url_warning_and_errors() {
        let status = CloudflareTunnelRuntimeStatus {
            public_url: Some("https://moltis.example.com".to_string()),
            hostname: Some("moltis.example.com".to_string()),
            passkey_warning: Some("register passkey origin".to_string()),
        };

        assert_eq!(startup_lines(Some(&status), None), vec![
            "cloudflare tunnel: https://moltis.example.com".to_string(),
            "cloudflare tunnel note: register passkey origin".to_string(),
        ]);
        assert_eq!(startup_lines(None, Some("boom")), vec![
            "cloudflare tunnel: failed to start (boom)".to_string()
        ]);
    }

    #[test]
    fn cloudflared_args_place_url_on_run_subcommand() {
        assert_eq!(cloudflared_tunnel_args("http://127.0.0.1:8080"), [
            "tunnel",
            "--no-autoupdate",
            "run",
            "--url",
            "http://127.0.0.1:8080",
        ]);
    }

    #[test]
    fn cloudflared_target_url_preserves_configured_loopback_bind() {
        assert_eq!(
            cloudflared_target_url("127.0.0.1", 8080, false),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            cloudflared_target_url("localhost", 8080, true),
            "https://localhost:8080"
        );
        assert_eq!(
            cloudflared_target_url("::1", 8080, false),
            "http://[::1]:8080"
        );
    }
}
