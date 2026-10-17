//! Signal handling for the standalone gateway: flush instrumentation, stop
//! connectors and tunnels, and shut the browser pool down before exiting.

use super::*;

use super::launch::StartupStatus;

/// Resources released when the gateway receives a shutdown signal.
pub(super) struct ShutdownHooks {
    pub state: Arc<GatewayState>,
    pub browser: Arc<dyn moltis_gateway::services::BrowserService>,
    pub status: Option<Arc<dyn StartupStatus>>,
    #[cfg(feature = "mdns")]
    pub mdns: Option<Box<dyn FnOnce() + Send>>,
    /// Tailscale mode to reset on exit, when configured to do so.
    #[cfg(feature = "tailscale")]
    pub tailscale_reset: Option<TailscaleMode>,
}

#[allow(clippy::expect_used)]
pub(super) fn spawn_shutdown_handler(hooks: ShutdownHooks) {
    tokio::spawn(async move {
        #[cfg(unix)]
        let signal_name: &str = {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigterm =
                signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
            let mut sighup =
                signal(SignalKind::hangup()).expect("failed to register SIGHUP handler");
            tokio::select! {
                result = tokio::signal::ctrl_c() => {
                    if result.is_err() { return; }
                    "SIGINT"
                }
                result = sigterm.recv() => {
                    if result.is_none() { return; }
                    "SIGTERM"
                }
                result = sighup.recv() => {
                    if result.is_none() { return; }
                    "SIGHUP"
                }
            }
        };
        #[cfg(not(unix))]
        let signal_name: &str = {
            if tokio::signal::ctrl_c().await.is_err() {
                return;
            }
            "SIGINT"
        };
        if let Some(status) = hooks.status.as_ref() {
            status.clear();
        }
        let instrumentation = Arc::clone(&hooks.state.instrumentation);
        if signal_name == "SIGINT" {
            info!("received SIGINT, flushing instrumentation before exit");
            instrumentation
                .shutdown(std::time::Duration::from_secs(1))
                .await;
            std::process::exit(0);
        }
        info!(signal = signal_name, "starting graceful shutdown");
        #[cfg(feature = "connectors")]
        hooks.state.shutdown_connectors().await;
        #[cfg(feature = "mdns")]
        if let Some(shutdown_mdns) = hooks.mdns {
            shutdown_mdns();
        }
        #[cfg(feature = "tailscale")]
        if let Some(ts_mode) = hooks.tailscale_reset {
            info!("shutting down tailscale {ts_mode}");
            let manager = CliTailscaleManager::new();
            if let Err(e) = manager.disable().await {
                warn!("failed to reset tailscale on exit: {e}");
            }
        }
        let shutdown_grace = std::time::Duration::from_secs(5);
        instrumentation.shutdown(shutdown_grace).await;
        info!(
            grace_secs = shutdown_grace.as_secs(),
            "shutting down browser pool"
        );
        if hooks.browser.shutdown_with_grace(shutdown_grace).await {
            info!(
                grace_secs = shutdown_grace.as_secs(),
                "browser pool shut down"
            );
        } else {
            warn!(
                grace_secs = shutdown_grace.as_secs(),
                "browser pool shutdown exceeded grace period, forcing process exit"
            );
        }

        std::process::exit(0);
    });
}
