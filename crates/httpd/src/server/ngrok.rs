//! ngrok tunnel types and controller.

#[cfg(feature = "ngrok")]
use std::sync::Arc;

#[cfg(feature = "ngrok")]
use {
    moltis_gateway::{auth_webauthn::SharedWebAuthnRegistry, state::GatewayState},
    tracing::{info, warn},
};

#[cfg(feature = "ngrok")]
use tokio_util::sync::CancellationToken;

#[cfg(feature = "ngrok")]
#[derive(Clone, Debug)]
pub struct NgrokRuntimeStatus {
    pub public_url: String,
    pub passkey_warning: Option<String>,
}

#[cfg(feature = "ngrok")]
pub(super) struct NgrokActiveTunnel {
    pub(super) session: ngrok::Session,
    pub(super) forwarder: ngrok::forwarder::Forwarder<ngrok::tunnel::HttpTunnel>,
    pub(super) loopback_shutdown: CancellationToken,
    pub(super) loopback_task: tokio::task::JoinHandle<()>,
    pub(super) status: NgrokRuntimeStatus,
}

#[cfg(feature = "ngrok")]
struct NgrokControllerInner {
    gateway: Arc<GatewayState>,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
    runtime: Arc<tokio::sync::RwLock<Option<NgrokRuntimeStatus>>>,
    app: tokio::sync::RwLock<Option<axum::Router>>,
    active_tunnel: tokio::sync::Mutex<Option<NgrokActiveTunnel>>,
}

#[cfg(feature = "ngrok")]
#[derive(Clone)]
pub struct NgrokController {
    inner: Arc<NgrokControllerInner>,
}

#[cfg(feature = "ngrok")]
impl NgrokController {
    pub(super) fn new(
        gateway: Arc<GatewayState>,
        webauthn_registry: Option<SharedWebAuthnRegistry>,
        runtime: Arc<tokio::sync::RwLock<Option<NgrokRuntimeStatus>>>,
    ) -> Self {
        Self {
            inner: Arc::new(NgrokControllerInner {
                gateway,
                webauthn_registry,
                runtime,
                app: tokio::sync::RwLock::new(None),
                active_tunnel: tokio::sync::Mutex::new(None),
            }),
        }
    }

    pub async fn configure_app(&self, app: axum::Router) {
        let mut stored = self.inner.app.write().await;
        *stored = Some(app);
    }

    pub async fn apply(
        &self,
        ngrok_config: &moltis_config::NgrokConfig,
    ) -> crate::error::Result<Option<NgrokRuntimeStatus>> {
        self.stop().await?;

        if !ngrok_config.enabled {
            info!("ngrok tunnel disabled");
            return Ok(None);
        }

        let active_tunnel = self.start(ngrok_config).await?;
        let status = active_tunnel.status.clone();
        {
            let mut runtime = self.inner.runtime.write().await;
            *runtime = Some(status.clone());
        }
        {
            let mut active = self.inner.active_tunnel.lock().await;
            *active = Some(active_tunnel);
        }
        info!(url = %status.public_url, "ngrok tunnel started");
        Ok(Some(status))
    }

    async fn start(
        &self,
        ngrok_config: &moltis_config::NgrokConfig,
    ) -> crate::error::Result<NgrokActiveTunnel> {
        let app = {
            let stored = self.inner.app.read().await;
            stored.clone().ok_or_else(|| {
                crate::Error::Ngrok("ngrok tunnel cannot start before the HTTP app is ready".into())
            })?
        };

        super::runtime::start_ngrok_tunnel(
            app,
            Arc::clone(&self.inner.gateway),
            self.inner.webauthn_registry.clone(),
            ngrok_config,
        )
        .await
    }

    pub async fn stop(&self) -> crate::error::Result<()> {
        use ngrok::prelude::TunnelCloser;

        let active_tunnel = {
            let mut active = self.inner.active_tunnel.lock().await;
            active.take()
        };

        let Some(mut active_tunnel) = active_tunnel else {
            let mut runtime = self.inner.runtime.write().await;
            *runtime = None;
            return Ok(());
        };

        let stopped_url = active_tunnel.status.public_url.clone();
        active_tunnel.loopback_shutdown.cancel();

        if let Err(error) = active_tunnel.forwarder.close().await {
            warn!(url = %stopped_url, %error, "failed to close ngrok tunnel");
        }
        if let Err(error) = active_tunnel.session.close().await {
            warn!(url = %stopped_url, %error, "failed to close ngrok session");
        }

        match active_tunnel.forwarder.join().await {
            Ok(Ok(())) => {},
            Ok(Err(error)) => {
                warn!(url = %stopped_url, %error, "ngrok tunnel forwarder exited with error");
            },
            Err(error) => {
                warn!(url = %stopped_url, %error, "ngrok tunnel join failed");
            },
        }

        match active_tunnel.loopback_task.await {
            Ok(()) => {},
            Err(error) => {
                warn!(url = %stopped_url, %error, "ngrok loopback server task join failed");
            },
        }

        let mut runtime = self.inner.runtime.write().await;
        *runtime = None;
        info!(url = %stopped_url, "ngrok tunnel stopped");
        Ok(())
    }
}
