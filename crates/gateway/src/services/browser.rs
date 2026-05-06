// ── Browser (Real implementation — depends on moltis-browser) ───────────────

use super::*;

/// Real browser service using BrowserManager.
pub struct RealBrowserService {
    config: moltis_browser::BrowserConfig,
    manager: tokio::sync::OnceCell<Arc<moltis_browser::BrowserManager>>,
}

impl RealBrowserService {
    pub fn new(config: &moltis_config::schema::BrowserConfig, container_prefix: String) -> Self {
        let mut browser_config = moltis_browser::BrowserConfig::from(config);
        browser_config.container_prefix = container_prefix;
        Self {
            config: browser_config,
            manager: tokio::sync::OnceCell::new(),
        }
    }

    pub fn from_config(
        config: &moltis_config::schema::MoltisConfig,
        container_prefix: String,
    ) -> Option<Self> {
        if !config.tools.browser.enabled {
            return None;
        }
        Some(Self::new(&config.tools.browser, container_prefix))
    }

    async fn manager(&self) -> Arc<moltis_browser::BrowserManager> {
        Arc::clone(
            self.manager
                .get_or_init(|| async {
                    let config = self.config.clone();
                    match tokio::task::spawn_blocking(move || {
                        // Browser detection and stale-container cleanup can block;
                        // run these off the async runtime worker threads.
                        moltis_browser::detect::check_and_warn(config.chrome_path.as_deref());
                        Arc::new(moltis_browser::BrowserManager::new(config))
                    })
                    .await
                    {
                        Ok(manager) => manager,
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                "browser warmup worker failed, falling back to inline initialization"
                            );
                            let config = self.config.clone();
                            moltis_browser::detect::check_and_warn(config.chrome_path.as_deref());
                            Arc::new(moltis_browser::BrowserManager::new(config))
                        },
                    }
                })
                .await,
        )
    }

    fn manager_if_initialized(&self) -> Option<Arc<moltis_browser::BrowserManager>> {
        self.manager.get().map(Arc::clone)
    }
}

#[async_trait]
impl BrowserService for RealBrowserService {
    async fn request(&self, params: Value) -> ServiceResult {
        let request: moltis_browser::BrowserRequest =
            serde_json::from_value(params).map_err(|e| format!("invalid request: {e}"))?;

        let manager = self.manager().await;
        let response = manager.handle_request(request).await;

        Ok(serde_json::to_value(&response).map_err(|e| format!("serialization error: {e}"))?)
    }

    async fn warmup(&self) {
        let started = std::time::Instant::now();
        let _ = self.manager().await;
        tracing::debug!(
            elapsed_ms = started.elapsed().as_millis(),
            "browser service warmup complete"
        );
    }

    async fn cleanup_idle(&self) {
        if let Some(manager) = self.manager_if_initialized() {
            manager.cleanup_idle().await;
        }
    }

    async fn shutdown(&self) {
        if let Some(manager) = self.manager_if_initialized() {
            manager.shutdown().await;
        }
    }

    async fn close_all(&self) {
        if let Some(manager) = self.manager_if_initialized() {
            manager.shutdown().await;
        }
    }
}
