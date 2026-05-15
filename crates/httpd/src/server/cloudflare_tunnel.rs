//! Cloudflare Tunnel controller.

#[cfg(feature = "cloudflare-tunnel")]
use std::{process::Stdio, sync::Arc};

#[cfg(feature = "cloudflare-tunnel")]
use {
    moltis_config::schema::CloudflareTunnelConfig,
    moltis_gateway::{auth_webauthn::SharedWebAuthnRegistry, state::GatewayState},
    secrecy::ExposeSecret,
    tokio::process::{Child, Command},
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
    child: tokio::sync::Mutex<Option<Child>>,
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
            child: tokio::sync::Mutex::new(None),
        }
    }

    pub async fn apply(
        &self,
        config: &CloudflareTunnelConfig,
        port: u16,
        tls: bool,
    ) -> crate::error::Result<Option<CloudflareTunnelRuntimeStatus>> {
        self.stop().await?;

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

        let target = format!(
            "{}://127.0.0.1:{port}",
            if tls {
                "https"
            } else {
                "http"
            }
        );
        let child = Command::new("cloudflared")
            .args([
                "tunnel",
                "--no-autoupdate",
                "--url",
                &target,
                "run",
                "--token",
                &token,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| crate::Error::Config(format!("failed to run cloudflared: {error}")))?;

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

        *self.child.lock().await = Some(child);
        *self.runtime.write().await = Some(status.clone());
        info!(target = %target, "Cloudflare Tunnel started");
        Ok(Some(status))
    }

    pub async fn stop(&self) -> crate::error::Result<()> {
        let child = self.child.lock().await.take();
        if let Some(mut child) = child {
            if let Err(error) = child.kill().await {
                warn!(%error, "failed to stop cloudflared");
            }
            let _ = child.wait().await;
            info!("Cloudflare Tunnel stopped");
        }
        *self.runtime.write().await = None;
        Ok(())
    }
}
