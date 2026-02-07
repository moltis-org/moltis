//! Browser instance pool management.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use {
    chromiumoxide::{
        Browser, BrowserConfig as CdpBrowserConfig, Page,
        cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams, handler::HandlerConfig,
    },
    futures::StreamExt,
    sysinfo::System,
    tokio::sync::{Mutex, RwLock},
    tracing::{debug, info, warn},
};

use crate::{container::BrowserContainer, error::BrowserError, types::BrowserConfig};

/// Get current system memory usage as a percentage (0-100).
fn get_memory_usage_percent() -> u8 {
    let mut sys = System::new();
    sys.refresh_memory();

    let total = sys.total_memory();
    if total == 0 {
        return 0;
    }

    let used = sys.used_memory();
    let percent = (used as f64 / total as f64 * 100.0) as u8;
    percent.min(100)
}

/// A pooled browser instance with one or more pages.
struct BrowserInstance {
    browser: Browser,
    pages: HashMap<String, Page>,
    last_used: Instant,
    /// Whether this instance is running in sandbox mode.
    #[allow(dead_code)]
    sandboxed: bool,
    /// Container for sandboxed instances (None for host browser).
    #[allow(dead_code)]
    container: Option<BrowserContainer>,
}

/// Pool of browser instances for reuse.
pub struct BrowserPool {
    config: BrowserConfig,
    instances: RwLock<HashMap<String, Arc<Mutex<BrowserInstance>>>>,
    #[cfg(feature = "metrics")]
    active_count: std::sync::atomic::AtomicUsize,
}

impl BrowserPool {
    /// Create a new browser pool with the given configuration.
    pub fn new(config: BrowserConfig) -> Self {
        Self {
            config,
            instances: RwLock::new(HashMap::new()),
            #[cfg(feature = "metrics")]
            active_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Get or create a browser instance for the given session ID.
    /// Returns the session ID for the browser instance.
    ///
    /// The `sandbox` parameter determines whether to run the browser in a
    /// Docker container (true) or on the host (false). This is set when
    /// creating a new session and cannot be changed for existing sessions.
    pub async fn get_or_create(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<String, BrowserError> {
        // Treat empty string as None (generate new session ID)
        let session_id = session_id.filter(|s| !s.is_empty());

        // Check if we have an existing instance
        if let Some(sid) = session_id {
            let instances = self.instances.read().await;
            if instances.contains_key(sid) {
                debug!(session_id = sid, "reusing existing browser instance");
                return Ok(sid.to_string());
            }
        }

        // Check pool capacity using memory-based limits
        {
            // If max_instances is set (> 0), enforce it as a hard limit
            if self.config.max_instances > 0 {
                let instances = self.instances.read().await;
                if instances.len() >= self.config.max_instances {
                    drop(instances);
                    self.cleanup_idle().await;

                    let instances = self.instances.read().await;
                    if instances.len() >= self.config.max_instances {
                        return Err(BrowserError::PoolExhausted);
                    }
                }
            }

            // Check memory usage - block new instances if above threshold
            let memory_percent = get_memory_usage_percent();
            if memory_percent >= self.config.memory_limit_percent {
                // Try to clean up idle instances first
                self.cleanup_idle().await;

                // Re-check memory after cleanup
                let memory_after = get_memory_usage_percent();
                if memory_after >= self.config.memory_limit_percent {
                    warn!(
                        memory_usage = memory_after,
                        threshold = self.config.memory_limit_percent,
                        "blocking new browser instance due to high memory usage"
                    );
                    return Err(BrowserError::PoolExhausted);
                }
            }
        }

        // Create new instance
        let sid = session_id
            .map(String::from)
            .unwrap_or_else(generate_session_id);

        let instance = self.launch_browser(&sid, sandbox).await?;
        let instance = Arc::new(Mutex::new(instance));

        {
            let mut instances = self.instances.write().await;
            instances.insert(sid.clone(), instance);
        }

        #[cfg(feature = "metrics")]
        {
            self.active_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            moltis_metrics::gauge!(moltis_metrics::browser::INSTANCES_ACTIVE)
                .set(self.active_count.load(std::sync::atomic::Ordering::Relaxed) as f64);
            moltis_metrics::counter!(moltis_metrics::browser::INSTANCES_CREATED_TOTAL).increment(1);
        }

        let mode = if sandbox {
            "sandboxed"
        } else {
            "host"
        };
        info!(session_id = sid, mode, "launched new browser instance");
        Ok(sid)
    }

    /// Get the page for a session, creating one if needed.
    pub async fn get_page(&self, session_id: &str) -> Result<Page, BrowserError> {
        let instances = self.instances.read().await;
        let instance = instances
            .get(session_id)
            .ok_or(BrowserError::ElementNotFound(0))?;

        let mut inst = instance.lock().await;
        inst.last_used = Instant::now();

        // Get or create the main page
        if let Some(page) = inst.pages.get("main") {
            debug!(session_id, "reusing existing page");
            return Ok(page.clone());
        }

        // Create a new page
        let page = inst
            .browser
            .new_page("about:blank")
            .await
            .map_err(|e| BrowserError::LaunchFailed(e.to_string()))?;

        // Explicitly set viewport on page to ensure it matches config
        // (browser-level viewport may not always be applied to new pages)
        let viewport_cmd = SetDeviceMetricsOverrideParams::builder()
            .width(self.config.viewport_width)
            .height(self.config.viewport_height)
            .device_scale_factor(self.config.device_scale_factor)
            .mobile(false)
            .build()
            .expect("valid viewport params");

        if let Err(e) = page.execute(viewport_cmd).await {
            warn!(session_id, error = %e, "failed to set page viewport");
        }

        info!(
            session_id,
            viewport_width = self.config.viewport_width,
            viewport_height = self.config.viewport_height,
            device_scale_factor = self.config.device_scale_factor,
            "created new page with viewport"
        );

        inst.pages.insert("main".to_string(), page.clone());
        Ok(page)
    }

    /// Close a specific browser session.
    pub async fn close_session(&self, session_id: &str) -> Result<(), BrowserError> {
        let instance = {
            let mut instances = self.instances.write().await;
            instances.remove(session_id)
        };

        if let Some(instance) = instance {
            let inst = instance.lock().await;
            // Pages are closed when browser is dropped
            drop(inst);

            #[cfg(feature = "metrics")]
            {
                self.active_count
                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                moltis_metrics::gauge!(moltis_metrics::browser::INSTANCES_ACTIVE)
                    .set(self.active_count.load(std::sync::atomic::Ordering::Relaxed) as f64);
                moltis_metrics::counter!(moltis_metrics::browser::INSTANCES_DESTROYED_TOTAL)
                    .increment(1);
            }

            info!(session_id, "closed browser session");
        }

        Ok(())
    }

    /// Clean up idle browser instances.
    pub async fn cleanup_idle(&self) {
        let idle_timeout = Duration::from_secs(self.config.idle_timeout_secs);
        let now = Instant::now();

        let mut to_remove = Vec::new();

        {
            let instances = self.instances.read().await;
            for (sid, instance) in instances.iter() {
                if let Ok(inst) = instance.try_lock()
                    && now.duration_since(inst.last_used) > idle_timeout
                {
                    to_remove.push(sid.clone());
                }
            }
        }

        for sid in to_remove {
            if let Err(e) = self.close_session(&sid).await {
                warn!(session_id = sid, error = %e, "failed to close idle session");
            }
        }
    }

    /// Shut down all browser instances.
    pub async fn shutdown(&self) {
        let sessions: Vec<String> = {
            let instances = self.instances.read().await;
            instances.keys().cloned().collect()
        };

        for sid in sessions {
            let _ = self.close_session(&sid).await;
        }

        info!("browser pool shut down");
    }

    /// Get the number of active instances.
    pub async fn active_count(&self) -> usize {
        self.instances.read().await.len()
    }

    /// Launch a new browser instance.
    async fn launch_browser(
        &self,
        session_id: &str,
        sandbox: bool,
    ) -> Result<BrowserInstance, BrowserError> {
        if sandbox {
            self.launch_sandboxed_browser(session_id).await
        } else {
            self.launch_host_browser(session_id).await
        }
    }

    /// Launch a browser inside a container (sandboxed mode).
    async fn launch_sandboxed_browser(
        &self,
        session_id: &str,
    ) -> Result<BrowserInstance, BrowserError> {
        use crate::container;

        // Check container runtime availability (Docker or Apple Container)
        if !container::is_container_available() {
            return Err(BrowserError::LaunchFailed(
                "No container runtime available for sandboxed browser. \
                 Please install Docker or Apple Container."
                    .to_string(),
            ));
        }

        // Ensure the container image is available
        container::ensure_image(&self.config.sandbox_image).map_err(|e| {
            BrowserError::LaunchFailed(format!("failed to ensure browser image: {e}"))
        })?;

        // Start the container
        let container = BrowserContainer::start(
            &self.config.sandbox_image,
            self.config.viewport_width,
            self.config.viewport_height,
        )
        .map_err(|e| {
            BrowserError::LaunchFailed(format!("failed to start browser container: {e}"))
        })?;

        let ws_url = container.websocket_url();
        info!(
            session_id,
            container_id = container.id(),
            ws_url,
            "connecting to sandboxed browser"
        );

        // Connect to the containerized browser with custom timeout
        let handler_config = HandlerConfig {
            request_timeout: Duration::from_millis(self.config.navigation_timeout_ms),
            viewport: Some(chromiumoxide::handler::viewport::Viewport {
                width: self.config.viewport_width,
                height: self.config.viewport_height,
                device_scale_factor: Some(self.config.device_scale_factor),
                emulating_mobile: false,
                is_landscape: true,
                has_touch: false,
            }),
            ..Default::default()
        };

        let (browser, mut handler) = Browser::connect_with_config(&ws_url, handler_config)
            .await
            .map_err(|e| {
                BrowserError::LaunchFailed(format!(
                    "failed to connect to containerized browser at {}: {}",
                    ws_url, e
                ))
            })?;

        // Spawn handler to process browser events
        let session_id_clone = session_id.to_string();
        tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                debug!(session_id = session_id_clone, ?event, "browser event");
            }
            // Handler exits when connection closes - this is normal for idle sessions
            debug!(
                session_id = session_id_clone,
                "sandboxed browser event handler exited (connection closed)"
            );
        });

        info!(session_id, "sandboxed browser connected successfully");

        Ok(BrowserInstance {
            browser,
            pages: HashMap::new(),
            last_used: Instant::now(),
            sandboxed: true,
            container: Some(container),
        })
    }

    /// Launch a browser on the host (non-sandboxed mode).
    async fn launch_host_browser(&self, session_id: &str) -> Result<BrowserInstance, BrowserError> {
        // Check if Chrome/Chromium is available before attempting to launch
        let detection = crate::detect::detect_browser(self.config.chrome_path.as_deref());
        if !detection.found {
            return Err(BrowserError::LaunchFailed(format!(
                "Chrome/Chromium not found. {}",
                detection.install_hint
            )));
        }

        let mut builder = CdpBrowserConfig::builder();

        // with_head() shows the browser window (non-headless mode)
        // By default chromiumoxide runs headless, so we only call with_head() when NOT headless
        if !self.config.headless {
            builder = builder.with_head();
        }

        info!(
            session_id,
            viewport_width = self.config.viewport_width,
            viewport_height = self.config.viewport_height,
            device_scale_factor = self.config.device_scale_factor,
            headless = self.config.headless,
            "configuring browser viewport"
        );

        builder = builder
            .viewport(chromiumoxide::handler::viewport::Viewport {
                width: self.config.viewport_width,
                height: self.config.viewport_height,
                device_scale_factor: Some(self.config.device_scale_factor),
                emulating_mobile: false,
                is_landscape: true,
                has_touch: false,
            })
            .request_timeout(Duration::from_millis(self.config.navigation_timeout_ms));

        // User agent can be set via Chrome arg instead of builder method
        if let Some(ref ua) = self.config.user_agent {
            builder = builder.arg(format!("--user-agent={ua}"));
        }

        if let Some(ref path) = self.config.chrome_path {
            builder = builder.chrome_executable(path);
        }

        for arg in &self.config.chrome_args {
            builder = builder.arg(arg);
        }

        // Additional security/sandbox args for headless
        builder = builder
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-software-rasterizer")
            .arg("--no-sandbox")
            .arg("--disable-setuid-sandbox");

        let config = builder.build().map_err(|e| {
            BrowserError::LaunchFailed(format!("failed to build browser config: {e}"))
        })?;

        let (browser, mut handler) = Browser::launch(config).await.map_err(|e| {
            // Include install instructions in launch failure messages
            let install_hint = crate::detect::install_instructions();
            BrowserError::LaunchFailed(format!("browser launch failed: {e}\n\n{install_hint}"))
        })?;

        // Spawn handler to process browser events
        let session_id_clone = session_id.to_string();
        tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                debug!(session_id = session_id_clone, ?event, "browser event");
            }
        });

        Ok(BrowserInstance {
            browser,
            pages: HashMap::new(),
            last_used: Instant::now(),
            sandboxed: false,
            container: None,
        })
    }
}

/// Generate a random session ID.
fn generate_session_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let id: u64 = rng.random();
    format!("browser-{:016x}", id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_session_id() {
        let id1 = generate_session_id();
        let id2 = generate_session_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("browser-"));
    }
}
