//! NetBird private mesh forwarder controller.

#[cfg(feature = "netbird")]
use std::sync::Arc;

#[cfg(feature = "netbird")]
use {
    moltis_config::schema::NetbirdConfig,
    moltis_gateway::netbird::{CliNetbirdManager, NetbirdManager, NetbirdMode},
    tokio_util::sync::CancellationToken,
    tracing::{info, warn},
};

#[cfg(feature = "netbird")]
#[derive(Clone, Debug)]
pub struct NetbirdRuntimeStatus {
    pub url: String,
    pub peer_ip: String,
}

#[cfg(feature = "netbird")]
pub struct NetbirdController {
    runtime: Arc<tokio::sync::RwLock<Option<NetbirdRuntimeStatus>>>,
    active_forwarder: tokio::sync::Mutex<Option<NetbirdActiveForwarder>>,
}

#[cfg(feature = "netbird")]
struct NetbirdActiveForwarder {
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "netbird")]
impl NetbirdController {
    pub fn new(runtime: Arc<tokio::sync::RwLock<Option<NetbirdRuntimeStatus>>>) -> Self {
        Self {
            runtime,
            active_forwarder: tokio::sync::Mutex::new(None),
        }
    }

    pub async fn apply(
        &self,
        config: &NetbirdConfig,
        bind_addr: &str,
        port: u16,
        tls: bool,
    ) -> crate::error::Result<Option<NetbirdRuntimeStatus>> {
        let mut active_forwarder = self.active_forwarder.lock().await;
        stop_active_forwarder(active_forwarder.take()).await;
        *self.runtime.write().await = None;

        if config.mode != "serve" {
            info!("NetBird forwarder disabled");
            return Ok(None);
        }

        let shutdown = CancellationToken::new();
        let startup = resolve_netbird_status(port, tls).await?;
        let status = startup.clone();
        let bind_addr = bind_addr.to_string();
        let runtime = Arc::clone(&self.runtime);
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move {
            run_forwarder_loop(bind_addr, port, tls, task_shutdown, runtime).await;
        });

        *self.runtime.write().await = Some(status.clone());
        *active_forwarder = Some(NetbirdActiveForwarder { shutdown, task });
        info!(url = %status.url, peer_ip = %status.peer_ip, "NetBird forwarder started");
        Ok(Some(status))
    }

    pub async fn stop(&self) {
        let active_forwarder = self.active_forwarder.lock().await.take();
        stop_active_forwarder(active_forwarder).await;
        *self.runtime.write().await = None;
    }
}

#[cfg(feature = "netbird")]
async fn stop_active_forwarder(active_forwarder: Option<NetbirdActiveForwarder>) {
    if let Some(active_forwarder) = active_forwarder {
        active_forwarder.shutdown.cancel();
        active_forwarder.task.abort();
        match active_forwarder.task.await {
            Ok(()) => {},
            Err(error) if error.is_cancelled() => {},
            Err(error) => warn!(%error, "NetBird forwarder task failed while stopping"),
        }
        info!("NetBird forwarder stopped");
    }
}

#[cfg(feature = "netbird")]
pub async fn start_for_banner(
    controller: &NetbirdController,
    config: &NetbirdConfig,
    bind_addr: &str,
    port: u16,
    tls: bool,
) -> (Option<NetbirdRuntimeStatus>, Option<String>) {
    match controller.apply(config, bind_addr, port, tls).await {
        Ok(status) => (status, None),
        Err(error) => {
            warn!(%error, "NetBird forwarder failed to start; gateway will continue without it");
            (None, Some(error.to_string()))
        },
    }
}

#[cfg(feature = "netbird")]
pub fn startup_lines(
    status: Option<&NetbirdRuntimeStatus>,
    startup_error: Option<&str>,
) -> Vec<String> {
    if let Some(status) = status {
        vec![format!("netbird: {}", status.url)]
    } else {
        startup_error
            .map(|error| format!("netbird: failed to start ({error})"))
            .into_iter()
            .collect()
    }
}

#[cfg(feature = "netbird")]
async fn resolve_netbird_status(
    port: u16,
    tls: bool,
) -> crate::error::Result<NetbirdRuntimeStatus> {
    let manager = CliNetbirdManager::new(NetbirdMode::Serve, port, tls);
    let status = manager
        .status()
        .await
        .map_err(|error| crate::Error::Config(format!("NetBird status failed: {error}")))?;
    let peer_ip = status
        .peer_ip
        .ok_or_else(|| crate::Error::Config("NetBird is not connected".into()))?;
    let url = status.url.unwrap_or_else(|| {
        format!(
            "{}://{}:{}",
            if tls {
                "https"
            } else {
                "http"
            },
            peer_ip,
            port
        )
    });
    Ok(NetbirdRuntimeStatus { url, peer_ip })
}

#[cfg(feature = "netbird")]
async fn run_forwarder_loop(
    bind_addr: String,
    port: u16,
    tls: bool,
    shutdown: CancellationToken,
    runtime: Arc<tokio::sync::RwLock<Option<NetbirdRuntimeStatus>>>,
) {
    loop {
        if shutdown.is_cancelled() {
            break;
        }

        let status = match resolve_netbird_status(port, tls).await {
            Ok(status) => status,
            Err(error) => {
                warn!(%error, "NetBird status unavailable; retrying");
                *runtime.write().await = None;
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(std::time::Duration::from_secs(30)) => continue,
                }
            },
        };

        let listener = match tokio::net::TcpListener::bind(format!("{}:{port}", status.peer_ip))
            .await
        {
            Ok(listener) => listener,
            Err(error) => {
                warn!(peer_ip = %status.peer_ip, %error, "failed to bind NetBird forwarder; retrying");
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(std::time::Duration::from_secs(30)) => continue,
                }
            },
        };

        let current_peer_ip = status.peer_ip.clone();
        *runtime.write().await = Some(status.clone());
        info!(url = %status.url, peer_ip = %status.peer_ip, "NetBird forwarder listening");
        let mut status_check = tokio::time::interval(std::time::Duration::from_secs(30));
        status_check.tick().await;

        loop {
            tokio::select! {
                () = shutdown.cancelled() => return,
                _ = status_check.tick() => {
                    match resolve_netbird_status(port, tls).await {
                        Ok(next_status) if next_status.peer_ip != current_peer_ip => {
                            info!(old_peer_ip = %current_peer_ip, new_peer_ip = %next_status.peer_ip, "NetBird peer IP changed; rebinding forwarder");
                            break;
                        },
                        Ok(next_status) => {
                            *runtime.write().await = Some(next_status);
                        },
                        Err(error) => {
                            warn!(%error, "NetBird status unavailable; rebinding after retry");
                            *runtime.write().await = None;
                            break;
                        },
                    }
                },
                accepted = listener.accept() => {
                    let Ok((mut inbound, _peer)) = accepted else {
                        break;
                    };
                    let target = gateway_forward_target(&bind_addr, port);
                    tokio::spawn(async move {
                        match tokio::net::TcpStream::connect(target).await {
                            Ok(mut outbound) => {
                                let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                            },
                            Err(error) => {
                                warn!(%error, "NetBird forwarder failed to connect to local gateway");
                            },
                        }
                    });
                },
            }
        }
    }

    *runtime.write().await = None;
}

#[cfg(feature = "netbird")]
fn gateway_forward_target(bind_addr: &str, port: u16) -> String {
    if bind_addr.contains(':') && !bind_addr.starts_with('[') {
        format!("[{bind_addr}]:{port}")
    } else {
        format!("{bind_addr}:{port}")
    }
}

#[cfg(all(test, feature = "netbird"))]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct NotifyOnDrop(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for NotifyOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    #[tokio::test]
    async fn apply_off_clears_runtime_without_starting_forwarder() -> crate::error::Result<()> {
        let runtime = Arc::new(tokio::sync::RwLock::new(Some(NetbirdRuntimeStatus {
            url: "https://100.64.0.1:8080".to_string(),
            peer_ip: "100.64.0.1".to_string(),
        })));
        let controller = NetbirdController::new(Arc::clone(&runtime));

        let status = controller
            .apply(&NetbirdConfig::default(), "127.0.0.1", 8080, false)
            .await?;

        assert!(status.is_none());
        assert!(runtime.read().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn stop_cancels_active_forwarder_and_clears_runtime() {
        let runtime = Arc::new(tokio::sync::RwLock::new(Some(NetbirdRuntimeStatus {
            url: "https://100.64.0.1:8080".to_string(),
            peer_ip: "100.64.0.1".to_string(),
        })));
        let controller = NetbirdController::new(Arc::clone(&runtime));
        let shutdown = CancellationToken::new();
        let observed_shutdown = shutdown.clone();
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _drop_notifier = NotifyOnDrop(Some(dropped_tx));
            std::future::pending::<()>().await;
        });

        *controller.active_forwarder.lock().await = Some(NetbirdActiveForwarder { shutdown, task });
        tokio::task::yield_now().await;

        controller.stop().await;

        assert!(observed_shutdown.is_cancelled());
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx).await,
            Ok(Ok(()))
        ));
        assert!(controller.active_forwarder.lock().await.is_none());
        assert!(runtime.read().await.is_none());
    }

    #[test]
    fn startup_lines_reports_url_and_errors() {
        let status = NetbirdRuntimeStatus {
            url: "https://100.64.0.1:8080".to_string(),
            peer_ip: "100.64.0.1".to_string(),
        };

        assert_eq!(startup_lines(Some(&status), None), vec![
            "netbird: https://100.64.0.1:8080".to_string()
        ]);
        assert_eq!(startup_lines(None, Some("not connected")), vec![
            "netbird: failed to start (not connected)".to_string()
        ]);
    }

    #[test]
    fn gateway_forward_target_preserves_configured_loopback_bind() {
        assert_eq!(gateway_forward_target("127.0.0.1", 8080), "127.0.0.1:8080");
        assert_eq!(gateway_forward_target("localhost", 8080), "localhost:8080");
        assert_eq!(gateway_forward_target("::1", 8080), "[::1]:8080");
    }
}
