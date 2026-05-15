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
        port: u16,
        tls: bool,
    ) -> crate::error::Result<Option<NetbirdRuntimeStatus>> {
        self.stop().await;

        if config.mode != "serve" {
            info!("NetBird forwarder disabled");
            return Ok(None);
        }

        let shutdown = CancellationToken::new();
        let startup = resolve_netbird_status(port, tls).await?;
        let status = startup.clone();
        let runtime = Arc::clone(&self.runtime);
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move {
            run_forwarder_loop(port, tls, task_shutdown, runtime).await;
        });

        *self.runtime.write().await = Some(status.clone());
        *self.active_forwarder.lock().await = Some(NetbirdActiveForwarder { shutdown, task });
        info!(url = %status.url, peer_ip = %status.peer_ip, "NetBird forwarder started");
        Ok(Some(status))
    }

    pub async fn stop(&self) {
        let active_forwarder = self.active_forwarder.lock().await.take();
        if let Some(active_forwarder) = active_forwarder {
            active_forwarder.shutdown.cancel();
            active_forwarder.task.abort();
            info!("NetBird forwarder stopped");
        }
        *self.runtime.write().await = None;
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

        loop {
            tokio::select! {
                () = shutdown.cancelled() => return,
                () = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
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
                    tokio::spawn(async move {
                        match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
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
