//! Browser instance pool management.

use std::{
    collections::HashMap,
    path::PathBuf,
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

use crate::{
    container::{BrowserContainer, browserless_session_timeout_ms},
    error::Error,
    types::{BrowserConfig, BrowserPreference},
};

/// Containers now host multiple tabs with shared cookies — keep them
/// alive for a full day so logins persist and tabs aren't disrupted.
pub(crate) const MAX_BROWSER_INSTANCE_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// Information about an active browser session.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BrowserSessionInfo {
    pub session_id: String,
    pub sandboxed: bool,
    pub url: String,
    pub age_secs: u64,
    pub idle_secs: u64,
    pub profile_id: String,
}

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

/// Returns memory-saving Chrome flags when `total_mb` is below `threshold_mb`.
///
/// Returns an empty slice when the threshold is 0 (disabled) or when the system
/// has enough memory.
#[must_use]
pub(crate) fn low_memory_chrome_args(total_mb: u64, threshold_mb: u64) -> &'static [&'static str] {
    if threshold_mb == 0 || total_mb >= threshold_mb {
        return &[];
    }
    &[
        "--single-process",
        "--renderer-process-limit=1",
        "--js-flags=--max-old-space-size=128",
    ]
}

/// A pooled browser instance with one or more pages.
struct BrowserInstance {
    browser: Arc<Browser>,
    pages: HashMap<String, Page>,
    last_used: Instant,
    /// When this instance was first created. Used to enforce a hard TTL that
    /// prevents Chromium memory leaks from accumulating in long-lived instances.
    created_at: Instant,
    /// Whether this instance is running in sandbox mode.
    #[allow(dead_code)]
    sandboxed: bool,
    /// Container for sandboxed instances (None for host browser).
    #[allow(dead_code)]
    container: Option<BrowserContainer>,
    /// Browser profile identifier for cookie isolation.
    profile_id: String,
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
        browser: Option<BrowserPreference>,
        profile_id: &str,
    ) -> Result<String, Error> {
        // Treat empty string as None (generate new session ID)
        let session_id = session_id.filter(|s| !s.is_empty());

        // Check if we have an existing instance with this session ID
        if let Some(sid) = session_id {
            let instances = self.instances.read().await;
            if instances.contains_key(sid) {
                debug!(session_id = sid, "reusing existing browser instance");
                return Ok(sid.to_string());
            }
        }

        // If another session with the same profile is already running,
        // create a new tab in that browser instead of launching a new
        // container. This shares cookies, local storage, and avoids
        // Chrome's SingletonLock conflicts.
        if sandbox {
            let instances = self.instances.read().await;
            let existing = instances.iter().find(|(_, inst_arc)| {
                // Can't await lock here — try_lock to avoid blocking
                if let Ok(inst) = inst_arc.try_lock() {
                    inst.profile_id == profile_id && inst.sandboxed
                } else {
                    false
                }
            });
            if let Some((_, existing_inst)) = existing {
                let sid = session_id
                    .map(String::from)
                    .unwrap_or_else(generate_session_id);
                // Clone the browser handle and create a new page (tab)
                let inst = existing_inst.lock().await;
                let new_inst = BrowserInstance {
                    browser: Arc::clone(&inst.browser),
                    pages: HashMap::new(),
                    last_used: Instant::now(),
                    created_at: Instant::now(),
                    sandboxed: true,
                    container: None, // shares parent's container
                    profile_id: profile_id.to_string(),
                };
                drop(inst);
                drop(instances);

                let new_inst = Arc::new(Mutex::new(new_inst));
                self.instances.write().await.insert(sid.clone(), new_inst);

                info!(
                    session_id = sid,
                    profile_id, "created new tab in existing browser (shared cookies)"
                );
                return Ok(sid);
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
                        return Err(Error::PoolExhausted);
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
                    return Err(Error::PoolExhausted);
                }
            }
        }

        // Create new instance
        let sid = session_id
            .map(String::from)
            .unwrap_or_else(generate_session_id);

        let instance = self
            .launch_browser(&sid, sandbox, browser, profile_id)
            .await?;
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
    pub async fn get_page(&self, session_id: &str) -> Result<Page, Error> {
        let instances = self.instances.read().await;
        let instance = instances.get(session_id).ok_or(Error::ElementNotFound(0))?;

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
            .map_err(|e| Error::LaunchFailed(e.to_string()))?;

        // Explicitly set viewport on page to ensure it matches config
        // (browser-level viewport may not always be applied to new pages)
        let viewport_cmd = SetDeviceMetricsOverrideParams::builder()
            .width(self.config.viewport_width)
            .height(self.config.viewport_height)
            .device_scale_factor(self.config.device_scale_factor)
            .mobile(false)
            .build()
            .map_err(|e| Error::Cdp(format!("invalid viewport params: {e}")))?;

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

    /// Check if a session exists in the pool.
    pub async fn has_session(&self, session_id: &str) -> bool {
        self.instances.read().await.contains_key(session_id)
    }

    /// Close a specific browser session.
    pub async fn close_session(&self, session_id: &str) -> Result<(), Error> {
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

    /// Clean up idle browser instances and instances that have exceeded the
    /// hard TTL ([`MAX_BROWSER_INSTANCE_LIFETIME`]). The TTL prevents Chromium
    /// memory leaks from accumulating in long-lived browser instances.
    pub async fn cleanup_idle(&self) {
        let idle_timeout = Duration::from_secs(self.config.idle_timeout_secs);
        let now = Instant::now();

        let mut to_remove = Vec::new();

        {
            let instances = self.instances.read().await;
            for (sid, instance) in instances.iter() {
                if let Ok(inst) = instance.try_lock() {
                    let idle = now.duration_since(inst.last_used) > idle_timeout;
                    let expired =
                        now.duration_since(inst.created_at) > MAX_BROWSER_INSTANCE_LIFETIME;
                    if idle || expired {
                        if expired {
                            info!(
                                session_id = sid,
                                age_secs = inst.created_at.elapsed().as_secs(),
                                "browser instance exceeded max lifetime"
                            );
                        }
                        to_remove.push(sid.clone());
                    }
                }
            }
        }

        if to_remove.is_empty() {
            return;
        }

        info!(
            count = to_remove.len(),
            sessions = ?to_remove,
            "cleaning up browser sessions"
        );

        for sid in to_remove {
            if let Err(e) = self.close_session(&sid).await {
                warn!(session_id = sid, error = %e, "failed to close session");
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

    /// List all active session IDs with their metadata.
    pub async fn list_sessions(&self) -> Vec<BrowserSessionInfo> {
        let instances = self.instances.read().await;
        let mut sessions = Vec::with_capacity(instances.len());
        for (sid, instance) in instances.iter() {
            let inst = instance.lock().await;
            let url = if let Some(page) = inst.pages.values().next() {
                page.url().await.ok().flatten().unwrap_or_default()
            } else {
                String::new()
            };
            sessions.push(BrowserSessionInfo {
                session_id: sid.clone(),
                sandboxed: inst.sandboxed,
                url,
                age_secs: inst.created_at.elapsed().as_secs(),
                idle_secs: inst.last_used.elapsed().as_secs(),
                profile_id: inst.profile_id.clone(),
            });
        }
        sessions
    }

    /// Launch a new browser instance.
    async fn launch_browser(
        &self,
        session_id: &str,
        sandbox: bool,
        browser: Option<BrowserPreference>,
        profile_id: &str,
    ) -> Result<BrowserInstance, Error> {
        if sandbox {
            self.launch_sandboxed_browser(session_id, profile_id).await
        } else {
            self.launch_host_browser(session_id, browser).await
        }
    }

    /// Launch a browser inside a container (sandboxed mode).
    async fn launch_sandboxed_browser(
        &self,
        session_id: &str,
        profile_id: &str,
    ) -> Result<BrowserInstance, Error> {
        use crate::container;

        // All container operations (CLI checks, image pulls, container start +
        // readiness polling) use synchronous `std::process::Command` and
        // `std::thread::sleep`.  Run them on the blocking thread-pool so they
        // don't stall the tokio event loop.
        let image = self.config.sandbox_image.clone();
        let prefix = self.config.container_prefix.clone();
        let vw = self.config.viewport_width;
        let vh = self.config.viewport_height;
        let low_mem = self.config.low_memory_threshold_mb;
        let session_timeout_ms = browserless_session_timeout_ms(
            self.config.idle_timeout_secs,
            self.config.navigation_timeout_ms,
            MAX_BROWSER_INSTANCE_LIFETIME.as_secs(),
        );
        let profile_dir = sandbox_profile_dir(self.config.resolved_profile_dir(), profile_id);
        let container_host = self.config.container_host.clone();

        let container = tokio::task::spawn_blocking(move || {
            // Check container runtime availability (Docker or Apple Container)
            if !container::is_container_available() {
                return Err(Error::LaunchFailed(
                    "No container runtime available for sandboxed browser. \
                     Please install Docker or Apple Container."
                        .to_string(),
                ));
            }

            // Ensure the container image is available
            let t_image = Instant::now();
            container::ensure_image(&image)
                .map_err(|e| Error::LaunchFailed(format!("failed to ensure browser image: {e}")))?;
            info!(
                elapsed_ms = t_image.elapsed().as_millis() as u64,
                "browser container image ready"
            );

            // Create profile directory on host if needed.
            // The directory must be world-writable (0o777) because the browserless/chrome
            // container runs Chrome as uid 999 (`chrome` user), which differs from the
            // host uid that owns this directory. Without open permissions the bind-mounted
            // volume is not writable and Chrome crashes on startup.
            // This is safe: the directory is Moltis-managed (~/.moltis/browser/profile/)
            // and contains only ephemeral Chrome profile data (cache, cookies, local
            // storage). It is not a system directory and has no security-sensitive content.
            if let Some(ref dir) = profile_dir {
                match std::fs::create_dir_all(dir) {
                    Err(e) => warn!(
                        path = %dir.display(),
                        error = %e,
                        "failed to create browser profile directory for container"
                    ),
                    Ok(()) => {
                        set_container_dir_permissions(dir);
                        // Remove stale Chrome singleton files from a previous
                        // session. These are symlinks that dangle after the
                        // container dies. Use symlink_metadata (not exists())
                        // because exists() follows symlinks and returns false
                        // for dangling ones.
                        for name in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
                            let path = dir.join(name);
                            if std::fs::symlink_metadata(&path).is_ok() {
                                let _ = std::fs::remove_file(&path);
                            }
                        }
                    },
                }
            }

            // Start the container (includes readiness polling)
            BrowserContainer::start(
                &image,
                &prefix,
                vw,
                vh,
                low_mem,
                session_timeout_ms,
                profile_dir.as_deref(),
                &container_host,
            )
            .map_err(|e| Error::LaunchFailed(format!("failed to start browser container: {e}")))
        })
        .await
        .map_err(|e| Error::LaunchFailed(format!("container launch task panicked: {e}")))??;

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
                Error::LaunchFailed(format!(
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
            browser: Arc::new(browser),
            pages: HashMap::new(),
            last_used: Instant::now(),
            created_at: Instant::now(),
            sandboxed: true,
            container: Some(container),
            profile_id: profile_id.to_string(),
        })
    }

    /// Launch a browser on the host (non-sandboxed mode).
    async fn launch_host_browser(
        &self,
        session_id: &str,
        browser: Option<BrowserPreference>,
    ) -> Result<BrowserInstance, Error> {
        let requested_browser = browser.unwrap_or_default();

        // Detect all installed browser candidates.
        let mut detection = crate::detect::detect_browser(self.config.chrome_path.as_deref());
        let mut install_attempt: Option<crate::detect::AutoInstallResult> = None;

        // Auto-install is always on: if none are installed, try to install one.
        if detection.browsers.is_empty() {
            let result = crate::detect::auto_install_browser(requested_browser).await;
            if result.attempted && result.installed {
                info!(details = %result.details, "auto-installed browser on host");
            } else if result.attempted {
                warn!(details = %result.details, "browser auto-install failed");
            } else {
                warn!(
                    details = %result.details,
                    "browser auto-install skipped (installer unavailable)"
                );
            }
            install_attempt = Some(result);
            detection = crate::detect::detect_browser(self.config.chrome_path.as_deref());
        }

        if detection.browsers.is_empty() {
            let mut message = format!("No compatible browser found. {}", detection.install_hint);
            if let Some(attempt) = install_attempt
                && attempt.attempted
            {
                message.push_str("\n\nAuto-install attempt:\n");
                message.push_str(&attempt.details);
            }
            return Err(Error::LaunchFailed(message));
        }

        let selected =
            match crate::detect::pick_browser(&detection.browsers, Some(requested_browser)) {
                Some(browser) => browser,
                None => {
                    let installed = crate::detect::installed_browser_labels(&detection.browsers);
                    let installed_list = if installed.is_empty() {
                        "none".to_string()
                    } else {
                        installed.join(", ")
                    };
                    return Err(Error::LaunchFailed(format!(
                        "requested browser '{}' is not installed. Installed browsers: {}",
                        requested_browser, installed_list
                    )));
                },
            };

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
        builder = builder.chrome_executable(selected.path.clone());

        for arg in &self.config.chrome_args {
            builder = builder.arg(arg);
        }

        // ── Stealth flags to reduce headless browser detection ──
        // Sites like LinkedIn detect headless Chrome via navigator.webdriver,
        // user agent strings, missing plugins, etc.
        builder = builder
            // Remove navigator.webdriver = true
            .arg("--disable-blink-features=AutomationControlled")
            // Use new headless mode (more like real Chrome)
            .arg("--headless=new")
            // Disable automation info bar
            .arg("--disable-infobars")
            // Realistic window size
            .arg("--window-size=1440,900")
            // Disable "Chrome is being controlled" notification
            .arg("--disable-automation-extension");

        // Set a realistic user agent if not explicitly configured
        if self.config.user_agent.is_none() {
            builder = builder.arg(
                "--user-agent=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
            );
        }

        // Restore session cookies across browser restarts so logins persist.
        // Without this, cookies marked as "session" (no expiry) are lost
        // when Chrome exits, even with a persistent profile directory.
        builder = builder.arg("--restore-session-cookies");

        // Set persistent profile directory if configured
        if let Some(ref profile_path) = self.config.resolved_profile_dir() {
            if let Err(e) = std::fs::create_dir_all(profile_path) {
                warn!(
                    path = %profile_path.display(),
                    error = %e,
                    "failed to create browser profile directory, falling back to ephemeral"
                );
            } else {
                // Clean up stale Chrome singleton files from previous sessions
                for name in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
                    let p = profile_path.join(name);
                    if std::fs::symlink_metadata(&p).is_ok() {
                        let _ = std::fs::remove_file(&p);
                    }
                }
                info!(
                    path = %profile_path.display(),
                    "using persistent browser profile"
                );
                builder = builder.user_data_dir(profile_path);
            }
        }

        // Additional security/sandbox args for headless
        builder = builder
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-software-rasterizer")
            .arg("--no-sandbox")
            .arg("--disable-setuid-sandbox");

        // Auto-inject low-memory flags on constrained systems
        if self.config.low_memory_threshold_mb > 0 {
            let mut sys = System::new();
            sys.refresh_memory();
            let total_mb = sys.total_memory() / (1024 * 1024);
            let extra = low_memory_chrome_args(total_mb, self.config.low_memory_threshold_mb);
            if !extra.is_empty() {
                info!(
                    total_mb,
                    threshold = self.config.low_memory_threshold_mb,
                    "low memory detected, adding constrained Chrome flags"
                );
                for arg in extra {
                    builder = builder.arg(*arg);
                }
            }
        }

        let config = builder
            .build()
            .map_err(|e| Error::LaunchFailed(format!("failed to build browser config: {e}")))?;

        let (browser, mut handler) = Browser::launch(config).await.map_err(|e| {
            // Include install instructions in launch failure messages
            let install_hint = crate::detect::install_instructions();
            Error::LaunchFailed(format!("browser launch failed: {e}\n\n{install_hint}"))
        })?;

        info!(
            session_id,
            browser = %selected.kind,
            path = %selected.path.display(),
            "launched host browser executable"
        );

        // Spawn handler to process browser events
        let session_id_clone = session_id.to_string();
        tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                debug!(session_id = session_id_clone, ?event, "browser event");
            }
        });

        Ok(BrowserInstance {
            browser: Arc::new(browser),
            pages: HashMap::new(),
            last_used: Instant::now(),
            created_at: Instant::now(),
            sandboxed: false,
            container: None,
            profile_id: "default".to_string(),
        })
    }
}

impl Drop for BrowserPool {
    fn drop(&mut self) {
        let instances = self.instances.get_mut();
        let count = instances.len();
        if count > 0 {
            info!(
                count,
                "browser pool dropping, stopping remaining containers"
            );
        }
    }
}

/// Generate a random session ID.
fn generate_session_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let id: u64 = rng.random();
    format!("browser-{:016x}", id)
}

/// Sanitize a session identifier to a filesystem-safe single path segment.
fn sanitize_session_component(session_id: &str) -> String {
    let sanitized: String = session_id
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '_',
        })
        .collect();

    if sanitized.is_empty() {
        return "session".to_string();
    }

    sanitized
}

/// Derive a per-session sandbox profile directory from a configured profile root.
fn sandbox_profile_dir(profile_root: Option<PathBuf>, profile_id: &str) -> Option<PathBuf> {
    // Each profile_id gets its own directory so agents with different
    // profiles have isolated cookies/local storage, while sessions within
    // the same profile share state and persist across restarts.
    // Chrome's SingletonLock is cleaned up before launch to avoid conflicts.
    profile_root.map(|root| {
        root.join("sandbox")
            .join(sanitize_session_component(profile_id))
    })
}

/// Make a directory world-accessible so container processes (which run as a
/// different uid, e.g. uid 999 in browserless/chrome) can read and write it.
///
/// On Unix this sets mode `0o777`. On non-Unix platforms this is a no-op
/// because container runtimes there (e.g. Apple Container) handle permission
/// mapping differently.
#[cfg(unix)]
fn set_container_dir_permissions(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o777)) {
        warn!(
            path = %dir.display(),
            error = %e,
            "failed to set browser profile directory permissions"
        );
    }
}

#[cfg(not(unix))]
fn set_container_dir_permissions(_dir: &std::path::Path) {}

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

    #[test]
    fn sanitize_session_component_replaces_unsafe_chars() {
        let sanitized = sanitize_session_component("discord:moltis:1476434288646815864");
        assert_eq!(sanitized, "discord_moltis_1476434288646815864");
    }

    #[test]
    fn sandbox_profile_dir_uses_profile_id() {
        let base = PathBuf::from("/tmp/moltis-profile");
        let path = sandbox_profile_dir(Some(base.clone()), "default");
        assert_eq!(
            path,
            Some(PathBuf::from("/tmp/moltis-profile/sandbox/default"))
        );

        let path = sandbox_profile_dir(Some(base), "session:main");
        assert_eq!(
            path,
            Some(PathBuf::from("/tmp/moltis-profile/sandbox/session_main"))
        );
    }

    #[test]
    fn sandbox_profile_dir_none_when_profile_disabled() {
        assert!(sandbox_profile_dir(None, "browser-abc123").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn set_container_dir_permissions_makes_world_writable() -> Result<(), Box<dyn std::error::Error>>
    {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("browser-profile");
        std::fs::create_dir_all(&dir)?;

        set_container_dir_permissions(&dir);

        let mode = std::fs::metadata(&dir)?.permissions().mode();
        assert_eq!(mode & 0o777, 0o777, "directory should be world-writable");
        Ok(())
    }

    fn test_config() -> BrowserConfig {
        BrowserConfig {
            idle_timeout_secs: 60,
            ..BrowserConfig::default()
        }
    }

    #[tokio::test]
    async fn cleanup_idle_empty_pool_returns_early() {
        let pool = BrowserPool::new(test_config());
        // Should not panic — hits the early-return guard.
        pool.cleanup_idle().await;
        assert_eq!(pool.active_count().await, 0);
    }

    #[tokio::test]
    async fn shutdown_empty_pool_is_noop() {
        let pool = BrowserPool::new(test_config());
        pool.shutdown().await;
        assert_eq!(pool.active_count().await, 0);
    }

    #[tokio::test]
    async fn active_count_starts_at_zero() {
        let pool = BrowserPool::new(test_config());
        assert_eq!(pool.active_count().await, 0);
    }

    #[tokio::test]
    async fn close_session_missing_is_ok() {
        let pool = BrowserPool::new(test_config());
        // Closing a non-existent session should succeed (no-op).
        let result = pool.close_session("nonexistent").await;
        assert!(result.is_ok());
    }

    #[test]
    fn drop_empty_pool_does_not_panic() {
        let pool = BrowserPool::new(test_config());
        drop(pool);
    }

    #[cfg(unix)]
    #[test]
    fn singleton_lock_dangling_symlink_detected() {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir failed: {e}"));
        let link = tmp.path().join("dangling-link");
        // Create a symlink to a non-existent target
        std::os::unix::fs::symlink("/nonexistent/target", &link)
            .unwrap_or_else(|e| panic!("symlink failed: {e}"));

        // symlink_metadata succeeds (the symlink inode exists)
        assert!(
            link.symlink_metadata().is_ok(),
            "symlink_metadata should succeed for dangling symlink"
        );
        // exists() returns false because the target doesn't exist
        assert!(
            !link.exists(),
            "exists() should return false for dangling symlink"
        );
        // remove_file works on dangling symlinks
        assert!(
            std::fs::remove_file(&link).is_ok(),
            "remove_file should succeed on dangling symlink"
        );
    }

    #[test]
    fn sandbox_profile_same_id_same_path() {
        let root = PathBuf::from("/tmp/profiles");
        let a = sandbox_profile_dir(Some(root.clone()), "default");
        let b = sandbox_profile_dir(Some(root), "default");
        assert_eq!(a, b, "same profile_id should produce the same path");
    }

    #[test]
    fn sandbox_profile_different_ids_different_paths() {
        let root = PathBuf::from("/tmp/profiles");
        let a = sandbox_profile_dir(Some(root.clone()), "alice");
        let b = sandbox_profile_dir(Some(root), "bob");
        assert_ne!(
            a, b,
            "different profile_ids should produce different paths"
        );
    }

    #[test]
    fn low_memory_args_injected_below_threshold() {
        let args = low_memory_chrome_args(1024, 2048);
        assert_eq!(args.len(), 3);
        assert!(args.contains(&"--single-process"));
        assert!(args.contains(&"--renderer-process-limit=1"));
        assert!(args.contains(&"--js-flags=--max-old-space-size=128"));
    }

    #[test]
    fn low_memory_args_empty_at_or_above_threshold() {
        assert!(low_memory_chrome_args(2048, 2048).is_empty());
        assert!(low_memory_chrome_args(4096, 2048).is_empty());
    }

    #[test]
    fn low_memory_args_disabled_when_threshold_zero() {
        assert!(low_memory_chrome_args(512, 0).is_empty());
    }
}
