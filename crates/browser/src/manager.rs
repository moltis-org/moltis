//! Browser manager providing high-level browser automation actions.

use std::{sync::Arc, time::Instant};

use {
    base64::{Engine, engine::general_purpose::STANDARD as BASE64},
    chromiumoxide::{
        Page,
        cdp::browser_protocol::{
            emulation::SetDeviceMetricsOverrideParams,
            input::{
                DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams,
                DispatchMouseEventType, MouseButton,
            },
            page::CaptureScreenshotFormat,
        },
    },
    tokio::time::{Duration, timeout},
    tracing::{debug, info, warn},
};

use crate::{
    error::Error,
    pool::BrowserPool,
    screencast::ScreencastRegistry,
    snapshot::{
        extract_snapshot, find_element_by_ref, focus_element_by_ref, scroll_element_into_view,
    },
    types::{
        BrowserAction, BrowserConfig, BrowserPreference, BrowserRequest, BrowserResponse,
        ExportedCookie, KeyInputType, MouseInputButton, MouseInputType,
    },
};

/// Extract session_id or return an error for actions that require an existing session.
fn require_session(session_id: Option<&str>, action: &str) -> Result<String, Error> {
    session_id
        .map(String::from)
        .ok_or_else(|| Error::InvalidAction(format!("{action} requires a session_id")))
}

/// Callback invoked after each browser action for logging/audit.
pub type ActionHook = Arc<dyn Fn(&str, &str, Option<&str>, bool, Option<&str>, u64) + Send + Sync>;

/// Manage Chrome/Chromium instances with CDP.
pub struct BrowserManager {
    pool: Arc<BrowserPool>,
    config: BrowserConfig,
    screencasts: Arc<ScreencastRegistry>,
    /// Optional hook called after each action: (session_id, action, url, success, error, duration_ms)
    action_hook: tokio::sync::RwLock<Option<ActionHook>>,
}

impl Default for BrowserManager {
    fn default() -> Self {
        Self::new(BrowserConfig::default())
    }
}

impl BrowserManager {
    /// Create a new browser manager with the given configuration.
    pub fn new(config: BrowserConfig) -> Self {
        match crate::container::cleanup_stale_browser_containers(&config.container_prefix) {
            Ok(removed) if removed > 0 => {
                info!(
                    removed,
                    "removed stale browser containers from previous runs"
                );
            },
            Ok(_) => {},
            Err(e) => {
                warn!(error = %e, "failed to clean stale browser containers at startup");
            },
        }

        info!(
            sandbox_image = %config.sandbox_image,
            "browser manager initialized (sandbox mode controlled per-session)"
        );

        Self {
            pool: Arc::new(BrowserPool::new(config.clone())),
            config,
            screencasts: Arc::new(ScreencastRegistry::new()),
            action_hook: tokio::sync::RwLock::new(None),
        }
    }

    /// Register a hook called after each browser action for logging/audit.
    /// Arguments: (session_id, action, url, success, error, duration_ms).
    pub async fn set_action_hook(&self, hook: ActionHook) {
        *self.action_hook.write().await = Some(hook);
    }

    /// Check if browser support is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Handle a browser request.
    pub async fn handle_request(&self, request: BrowserRequest) -> BrowserResponse {
        if !self.config.enabled {
            return BrowserResponse::error(
                request.session_id.unwrap_or_default(),
                "browser support is disabled",
                0,
            );
        }

        // Determine sandbox mode from request (defaults to false/host)
        let sandbox = request.sandbox.unwrap_or(false);
        let profile_id = request
            .profile_id
            .as_deref()
            .unwrap_or("default")
            .to_string();

        // Log the action with execution mode for visibility
        let mode = if sandbox {
            "sandbox"
        } else {
            "host"
        };
        info!(
            action = %request.action,
            session_id = request.session_id.as_deref().unwrap_or("(new)"),
            browser = ?request.browser,
            execution_mode = mode,
            sandbox_image = %self.config.sandbox_image,
            "executing browser action"
        );

        let start = Instant::now();
        let timeout_duration = Duration::from_millis(request.timeout_ms);
        let action_for_hook = format!("{}", request.action);

        let resp = match timeout(
            timeout_duration,
            self.execute_action(
                request.session_id.as_deref(),
                request.action,
                sandbox,
                request.browser,
                &profile_id,
            ),
        )
        .await
        {
            Ok(result) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                match result {
                    Ok((session_id, response)) => {
                        let mut resp = response;
                        resp.duration_ms = duration_ms;
                        resp.session_id = session_id;
                        resp
                    },
                    Err(e) => {
                        #[cfg(feature = "metrics")]
                        moltis_metrics::counter!(
                            moltis_metrics::browser::ERRORS_TOTAL,
                            "type" => e.to_string()
                        )
                        .increment(1);

                        BrowserResponse::error(
                            request.session_id.unwrap_or_default(),
                            e.to_string(),
                            duration_ms,
                        )
                    },
                }
            },
            Err(_) => {
                #[cfg(feature = "metrics")]
                moltis_metrics::counter!(
                    moltis_metrics::browser::ERRORS_TOTAL,
                    "type" => "timeout"
                )
                .increment(1);

                BrowserResponse::error(
                    request.session_id.unwrap_or_default(),
                    format!("operation timed out after {}ms", request.timeout_ms),
                    request.timeout_ms,
                )
            },
        };

        // Call action hook for logging/audit (skip high-frequency events)
        if !action_for_hook.starts_with("mouse_input")
            && !action_for_hook.starts_with("keyboard_input")
            && !action_for_hook.starts_with("evaluate")
            && !action_for_hook.starts_with("screenshot")
            && !matches!(
                action_for_hook.as_str(),
                "start_screencast" | "stop_screencast" | "get_url" | "get_title"
            )
        {
            if let Some(hook) = self.action_hook.read().await.as_ref() {
                hook(
                    &resp.session_id,
                    &action_for_hook,
                    resp.url.as_deref(),
                    resp.success,
                    resp.error.as_deref(),
                    resp.duration_ms,
                );
            }
        }

        resp
    }

    /// Clean up a session whose CDP connection has died and return an
    /// actionable error the agent can act on.
    ///
    /// Only logs and closes the session if it still exists in the pool
    /// (avoids spam when multiple queued events all hit a dead session).
    async fn cleanup_stale_session(&self, session_id: &str, action: &str) -> Error {
        if self.pool.has_session(session_id).await {
            warn!(
                session_id,
                action, "browser connection dead, closing stale session"
            );
            let _ = self.pool.close_session(session_id).await;
            // Notify the action hook so the session is marked closed in history
            if let Some(hook) = self.action_hook.read().await.as_ref() {
                hook(session_id, "close", None, false, Some("connection lost"), 0);
            }
        }
        Error::ConnectionClosed(format!(
            "Browser session {session_id} lost its connection during {action}. \
             Please navigate to the page again to get a fresh session."
        ))
    }

    /// Execute a browser action.
    async fn execute_action(
        &self,
        session_id: Option<&str>,
        action: BrowserAction,
        sandbox: bool,
        browser: Option<BrowserPreference>,
        profile_id: &str,
    ) -> Result<(String, BrowserResponse), Error> {
        // Navigate has its own retry-with-fresh-session logic, so handle it
        // separately to avoid double-cleanup.
        if let BrowserAction::Navigate { ref url } = action {
            return self
                .navigate(session_id, url, sandbox, browser, profile_id)
                .await;
        }

        let action_name = action.to_string();

        let result = match action {
            BrowserAction::Navigate { .. } => unreachable!(),
            BrowserAction::Screenshot {
                full_page,
                highlight_ref,
            } => {
                self.screenshot(
                    session_id,
                    full_page,
                    highlight_ref,
                    sandbox,
                    browser,
                    profile_id,
                )
                .await
            },
            BrowserAction::Snapshot => {
                self.snapshot(session_id, sandbox, browser, profile_id)
                    .await
            },
            BrowserAction::Click { ref_ } => self.click(session_id, ref_, sandbox).await,
            BrowserAction::Type { ref_, text } => {
                self.type_text(session_id, ref_, &text, sandbox).await
            },
            BrowserAction::Scroll { ref_, x, y } => {
                self.scroll(session_id, ref_, x, y, sandbox).await
            },
            BrowserAction::Evaluate { code } => self.evaluate(session_id, &code, sandbox).await,
            BrowserAction::Wait {
                selector,
                ref_,
                timeout_ms,
            } => {
                self.wait(session_id, selector, ref_, timeout_ms, sandbox)
                    .await
            },
            BrowserAction::GetUrl => self.get_url(session_id, sandbox).await,
            BrowserAction::GetTitle => self.get_title(session_id, sandbox).await,
            BrowserAction::Back => self.go_back(session_id, sandbox).await,
            BrowserAction::Forward => self.go_forward(session_id, sandbox).await,
            BrowserAction::Refresh => self.refresh(session_id, sandbox).await,
            BrowserAction::Close => self.close(session_id, sandbox).await,
            BrowserAction::StartScreencast {
                quality,
                max_width,
                max_height,
            } => {
                self.start_screencast(
                    session_id, sandbox, browser, quality, max_width, max_height, profile_id,
                )
                .await
            },
            BrowserAction::StopScreencast => self.stop_screencast(session_id, sandbox).await,
            BrowserAction::MouseInput {
                x,
                y,
                event_type,
                button,
                click_count,
                delta_x,
                delta_y,
            } => {
                self.mouse_input(
                    session_id,
                    x,
                    y,
                    event_type,
                    button,
                    click_count,
                    delta_x,
                    delta_y,
                    sandbox,
                )
                .await
            },
            BrowserAction::KeyboardInput {
                event_type,
                key,
                text,
                code,
                modifiers,
            } => {
                self.keyboard_input(session_id, event_type, key, text, code, modifiers, sandbox)
                    .await
            },
            BrowserAction::ExportCookies { domain } => {
                self.export_cookies(session_id, domain, sandbox).await
            },
            BrowserAction::ImportCookies { cookies } => {
                self.import_cookies(session_id, cookies, sandbox).await
            },
        };

        // Never kill sessions from execute_action errors. The navigate
        // method has its own retry-with-fresh-session logic for connection
        // errors. All other actions just return the error — Chrome may
        // recover on the next request. Sessions are only cleaned up by
        // explicit close, idle timeout, or navigate retry failure.
        result
    }

    /// Navigate to a URL.
    async fn navigate(
        &self,
        session_id: Option<&str>,
        url: &str,
        sandbox: bool,
        browser: Option<BrowserPreference>,
        profile_id: &str,
    ) -> Result<(String, BrowserResponse), Error> {
        // Validate URL before navigation
        validate_url(url)?;

        // Check if the domain is allowed
        if !crate::types::is_domain_allowed(url, &self.config.allowed_domains) {
            return Err(Error::NavigationFailed(format!(
                "domain not in allowed list. Allowed domains: {:?}",
                self.config.allowed_domains
            )));
        }

        let sid = self
            .pool
            .get_or_create(session_id, sandbox, browser, profile_id)
            .await?;
        let page = self.pool.get_page(&sid).await?;

        #[cfg(feature = "metrics")]
        let nav_start = Instant::now();

        // Try navigation, retry with fresh session if connection is dead
        if let Err(e) = page.goto(url).await {
            let nav_err = Error::NavigationFailed(e.to_string());
            if nav_err.is_connection_error() {
                warn!(
                    session_id = sid,
                    "browser connection dead, closing session and retrying"
                );
                let _ = self.pool.close_session(&sid).await;
                // Retry with a fresh session (use same sandbox mode)
                let new_sid = self
                    .pool
                    .get_or_create(None, sandbox, browser, profile_id)
                    .await?;
                let new_page = self.pool.get_page(&new_sid).await?;
                new_page
                    .goto(url)
                    .await
                    .map_err(|e| Error::NavigationFailed(e.to_string()))?;
                // Continue with the new session
                let _ = new_page.wait_for_navigation().await;
                let current_url = new_page.url().await.ok().flatten().unwrap_or_default();
                info!(
                    session_id = new_sid,
                    url = current_url,
                    "navigated to URL (after retry)"
                );
                return Ok((
                    new_sid.clone(),
                    BrowserResponse::success(new_sid, 0, sandbox).with_url(current_url),
                ));
            }
            return Err(nav_err);
        }

        // Wait for network idle
        let _ = page.wait_for_navigation().await;

        #[cfg(feature = "metrics")]
        {
            moltis_metrics::histogram!(moltis_metrics::browser::NAVIGATION_DURATION_SECONDS)
                .record(nav_start.elapsed().as_secs_f64());
        }

        let current_url = page.url().await.ok().flatten().unwrap_or_default();

        info!(session_id = sid, url = current_url, "navigated to URL");

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox).with_url(current_url),
        ))
    }

    /// Take a screenshot of the page.
    async fn screenshot(
        &self,
        session_id: Option<&str>,
        full_page: bool,
        highlight_ref: Option<u32>,
        sandbox: bool,
        browser: Option<BrowserPreference>,
        profile_id: &str,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = self
            .pool
            .get_or_create(session_id, sandbox, browser, profile_id)
            .await?;
        let page = self.pool.get_page(&sid).await?;

        // Optionally highlight an element before screenshot
        if let Some(ref_) = highlight_ref {
            let _ = self.highlight_element(&page, ref_).await;
        }

        let screenshot = page
            .screenshot(
                chromiumoxide::page::ScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Png)
                    .full_page(full_page)
                    .build(),
            )
            .await
            .map_err(|e| Error::ScreenshotFailed(e.to_string()))?;

        // Remove highlight after screenshot
        if highlight_ref.is_some() {
            let _ = self.remove_highlights(&page).await;
        }

        // Use data URI format so the sanitizer can strip it for LLM context
        // while the UI can still display it as an image
        let data_uri = format!("data:image/png;base64,{}", BASE64.encode(&screenshot));

        #[cfg(feature = "metrics")]
        moltis_metrics::counter!(moltis_metrics::browser::SCREENSHOTS_TOTAL).increment(1);

        // Calculate approximate dimensions from PNG data (width/height are in bytes 16-23)
        let (width, height) = if screenshot.len() > 24 {
            let w = u32::from_be_bytes([
                screenshot[16],
                screenshot[17],
                screenshot[18],
                screenshot[19],
            ]);
            let h = u32::from_be_bytes([
                screenshot[20],
                screenshot[21],
                screenshot[22],
                screenshot[23],
            ]);
            (w, h)
        } else {
            (0, 0)
        };

        info!(
            session_id = sid,
            bytes = screenshot.len(),
            width,
            height,
            full_page,
            "took screenshot"
        );

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox)
                .with_screenshot(data_uri, self.config.device_scale_factor),
        ))
    }

    /// Get a DOM snapshot with element references.
    ///
    /// Stale-connection errors are detected centrally in `execute_action()`.
    async fn snapshot(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
        browser: Option<BrowserPreference>,
        profile_id: &str,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = self
            .pool
            .get_or_create(session_id, sandbox, browser, profile_id)
            .await?;
        let page = self.pool.get_page(&sid).await?;

        let snapshot = extract_snapshot(&page).await?;

        debug!(
            session_id = sid,
            elements = snapshot.elements.len(),
            "extracted snapshot"
        );

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox).with_snapshot(snapshot),
        ))
    }

    /// Click an element by reference.
    async fn click(
        &self,
        session_id: Option<&str>,
        ref_: u32,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "click")?;

        let page = self.pool.get_page(&sid).await?;

        // Scroll element into view first
        scroll_element_into_view(&page, ref_).await?;

        // Small delay for scroll to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Find element center
        let (x, y) = find_element_by_ref(&page, ref_).await?;

        // Dispatch mouse events
        let press_cmd = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MousePressed)
            .x(x)
            .y(y)
            .button(MouseButton::Left)
            .click_count(1)
            .build()
            .map_err(|e| Error::Cdp(e.to_string()))?;
        page.execute(press_cmd)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        let release_cmd = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseReleased)
            .x(x)
            .y(y)
            .button(MouseButton::Left)
            .click_count(1)
            .build()
            .map_err(|e| Error::Cdp(e.to_string()))?;
        page.execute(release_cmd)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        debug!(
            session_id = sid,
            ref_ = ref_,
            x = x,
            y = y,
            "clicked element"
        );

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Type text into an element.
    async fn type_text(
        &self,
        session_id: Option<&str>,
        ref_: u32,
        text: &str,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "type")?;

        let page = self.pool.get_page(&sid).await?;

        // Focus the element
        focus_element_by_ref(&page, ref_).await?;

        // Type each character
        for c in text.chars() {
            let key_down = DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyDown)
                .text(c.to_string())
                .build()
                .map_err(|e| Error::Cdp(e.to_string()))?;
            page.execute(key_down)
                .await
                .map_err(|e| Error::Cdp(e.to_string()))?;

            let key_up = DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .text(c.to_string())
                .build()
                .map_err(|e| Error::Cdp(e.to_string()))?;
            page.execute(key_up)
                .await
                .map_err(|e| Error::Cdp(e.to_string()))?;
        }

        debug!(
            session_id = sid,
            ref_ = ref_,
            chars = text.len(),
            "typed text"
        );

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Scroll the page or an element.
    async fn scroll(
        &self,
        session_id: Option<&str>,
        ref_: Option<u32>,
        x: i32,
        y: i32,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "scroll")?;

        let page = self.pool.get_page(&sid).await?;

        let js = if let Some(ref_) = ref_ {
            format!(
                r#"(() => {{
                    const el = document.querySelector(`[data-moltis-ref="{ref_}"]`);
                    if (el) el.scrollBy({x}, {y});
                    return !!el;
                }})()"#
            )
        } else {
            format!("window.scrollBy({x}, {y}); true")
        };

        page.evaluate(js.as_str())
            .await
            .map_err(|e| Error::JsEvalFailed(e.to_string()))?;

        debug!(session_id = sid, ref_ = ?ref_, x = x, y = y, "scrolled");

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Execute JavaScript in the page context.
    async fn evaluate(
        &self,
        session_id: Option<&str>,
        code: &str,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "evaluate")?;

        let page = self.pool.get_page(&sid).await?;

        let result: serde_json::Value = page
            .evaluate(code)
            .await
            .map_err(|e| Error::JsEvalFailed(e.to_string()))?
            .into_value()
            .map_err(|e| Error::JsEvalFailed(format!("{e:?}")))?;

        debug!(session_id = sid, "evaluated JavaScript");

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox).with_result(result),
        ))
    }

    /// Wait for an element to appear.
    async fn wait(
        &self,
        session_id: Option<&str>,
        selector: Option<String>,
        ref_: Option<u32>,
        timeout_ms: u64,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "wait")?;

        let page = self.pool.get_page(&sid).await?;

        let check_js = if let Some(ref selector) = selector {
            format!(
                r#"document.querySelector({}) !== null"#,
                serde_json::to_string(selector).map_err(|e| Error::Cdp(e.to_string()))?
            )
        } else if let Some(ref_) = ref_ {
            format!(r#"document.querySelector('[data-moltis-ref="{ref_}"]') !== null"#)
        } else {
            return Err(Error::InvalidAction("wait requires selector or ref".into()));
        };

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let interval = Duration::from_millis(100);

        while Instant::now() < deadline {
            let found: bool = page
                .evaluate(check_js.as_str())
                .await
                .map_err(|e| Error::JsEvalFailed(e.to_string()))?
                .into_value()
                .unwrap_or(false);

            if found {
                debug!(session_id = sid, "element found");
                return Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)));
            }

            tokio::time::sleep(interval).await;
        }

        Err(Error::Timeout(format!(
            "element not found after {}ms",
            timeout_ms
        )))
    }

    /// Get the current page URL.
    async fn get_url(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "get_url")?;

        let page = self.pool.get_page(&sid).await?;
        let url = page.url().await.ok().flatten().unwrap_or_default();

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox).with_url(url),
        ))
    }

    /// Get the page title.
    async fn get_title(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "get_title")?;

        let page = self.pool.get_page(&sid).await?;
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox).with_title(title),
        ))
    }

    /// Go back in history.
    async fn go_back(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "back")?;

        let page = self.pool.get_page(&sid).await?;

        page.evaluate("history.back()")
            .await
            .map_err(|e| Error::JsEvalFailed(e.to_string()))?;

        // Wait for navigation
        let _ = page.wait_for_navigation().await;

        let url = page.url().await.ok().flatten().unwrap_or_default();

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox).with_url(url),
        ))
    }

    /// Go forward in history.
    async fn go_forward(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "forward")?;

        let page = self.pool.get_page(&sid).await?;

        page.evaluate("history.forward()")
            .await
            .map_err(|e| Error::JsEvalFailed(e.to_string()))?;

        // Wait for navigation
        let _ = page.wait_for_navigation().await;

        let url = page.url().await.ok().flatten().unwrap_or_default();

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox).with_url(url),
        ))
    }

    /// Refresh the page.
    async fn refresh(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "refresh")?;

        let page = self.pool.get_page(&sid).await?;

        page.reload().await.map_err(|e| Error::Cdp(e.to_string()))?;

        // Wait for navigation
        let _ = page.wait_for_navigation().await;

        let url = page.url().await.ok().flatten().unwrap_or_default();

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox).with_url(url),
        ))
    }

    /// Start a CDP screencast.
    async fn start_screencast(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
        browser: Option<BrowserPreference>,
        quality: u8,
        max_width: u32,
        max_height: u32,
        profile_id: &str,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = self
            .pool
            .get_or_create(session_id, sandbox, browser, profile_id)
            .await?;
        let page = self.pool.get_page(&sid).await?;

        // Resize the viewport to match the screencast area on first
        // screencast only. Repeated resizes on session switch can
        // destabilize the Chrome process inside containers.
        if !self.screencasts.is_active(&sid).await {
            let resize = SetDeviceMetricsOverrideParams::builder()
                .width(max_width)
                .height(max_height)
                .device_scale_factor(1.0)
                .mobile(false)
                .build()
                .map_err(|e| Error::Cdp(e.to_string()))?;
            if let Err(e) = page.execute(resize).await {
                debug!(session_id = sid, error = %e, "viewport resize failed (non-fatal)");
            }
        }

        let _rx = self
            .screencasts
            .start(&sid, &page, quality, max_width, max_height)
            .await?;

        info!(
            session_id = sid,
            quality, max_width, max_height, "screencast started (viewport resized)"
        );

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Stop a CDP screencast.
    async fn stop_screencast(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "stop_screencast")?;
        let page = self.pool.get_page(&sid).await.ok();
        self.screencasts.stop(&sid, page.as_ref()).await?;

        info!(session_id = sid, "screencast stopped");

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Send a mouse input event to the page.
    #[allow(clippy::too_many_arguments)]
    async fn mouse_input(
        &self,
        session_id: Option<&str>,
        x: f64,
        y: f64,
        event_type: MouseInputType,
        button: MouseInputButton,
        click_count: u32,
        delta_x: f64,
        delta_y: f64,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "mouse_input")?;
        let page = self.pool.get_page(&sid).await?;

        let cdp_type = match event_type {
            MouseInputType::Pressed => DispatchMouseEventType::MousePressed,
            MouseInputType::Released => DispatchMouseEventType::MouseReleased,
            MouseInputType::Moved => DispatchMouseEventType::MouseMoved,
            MouseInputType::Wheel => DispatchMouseEventType::MouseWheel,
        };

        let cdp_button = match button {
            MouseInputButton::Left => MouseButton::Left,
            MouseInputButton::Right => MouseButton::Right,
            MouseInputButton::Middle => MouseButton::Middle,
        };

        let mut builder = DispatchMouseEventParams::builder()
            .r#type(cdp_type)
            .x(x)
            .y(y)
            .button(cdp_button)
            .click_count(click_count as i64);

        // CDP requires both deltaX and deltaY for mouseWheel events
        if matches!(event_type, MouseInputType::Wheel) {
            builder = builder.delta_x(delta_x).delta_y(delta_y);
        }

        let cmd = builder.build().map_err(|e| Error::Cdp(e.to_string()))?;
        page.execute(cmd)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        debug!(session_id = sid, x, y, "mouse input dispatched");

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Send a keyboard input event to the page.
    async fn keyboard_input(
        &self,
        session_id: Option<&str>,
        event_type: KeyInputType,
        key: Option<String>,
        text: Option<String>,
        code: Option<String>,
        modifiers: Option<i64>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "keyboard_input")?;
        let page = self.pool.get_page(&sid).await?;

        let cdp_type = match event_type {
            KeyInputType::KeyDown => DispatchKeyEventType::KeyDown,
            KeyInputType::KeyUp => DispatchKeyEventType::KeyUp,
            KeyInputType::Char => DispatchKeyEventType::Char,
        };

        let mut builder = DispatchKeyEventParams::builder().r#type(cdp_type);

        if let Some(ref text) = text {
            builder = builder.text(text.clone());
        }
        if let Some(ref key) = key {
            builder = builder.key(key.clone());
            // CDP needs windowsVirtualKeyCode for special keys (Backspace,
            // Enter, Tab, arrows, Delete, Escape, etc.) — without it,
            // Chrome silently ignores the key event.
            if let Some(vk) = key_to_windows_virtual_keycode(key) {
                builder = builder.windows_virtual_key_code(vk);
            }
        }
        if let Some(ref code) = code {
            builder = builder.code(code.clone());
        }
        if let Some(modifiers) = modifiers {
            builder = builder.modifiers(modifiers);
        }

        let cmd = builder.build().map_err(|e| Error::Cdp(e.to_string()))?;
        page.execute(cmd)
            .await
            .map_err(|e| Error::Cdp(e.to_string()))?;

        debug!(session_id = sid, ?key, "keyboard input dispatched");

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Export cookies from the browser session.
    async fn export_cookies(
        &self,
        session_id: Option<&str>,
        domain: Option<String>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        use chromiumoxide::cdp::browser_protocol::network::GetCookiesParams;

        let sid = require_session(session_id, "export_cookies")?;
        let page = self.pool.get_page(&sid).await?;

        let params = {
            // Get the current URL to build a cookie URL filter
            let url = page.url().await.ok().flatten().unwrap_or_default();
            if url.is_empty() {
                GetCookiesParams::default()
            } else {
                GetCookiesParams {
                    urls: Some(vec![url]),
                }
            }
        };

        let result = page
            .execute(params)
            .await
            .map_err(|e| Error::Cdp(format!("failed to get cookies: {e}")))?;

        let cookies: Vec<ExportedCookie> = result
            .cookies
            .clone()
            .into_iter()
            .filter(|c| {
                domain
                    .as_ref()
                    .is_none_or(|d| c.domain.contains(d.as_str()))
            })
            .map(|c| ExportedCookie {
                name: c.name,
                value: c.value,
                domain: c.domain,
                path: c.path,
                secure: c.secure,
                http_only: c.http_only,
                expires: c.expires,
                same_site: c.same_site.map(|s| format!("{s:?}")).unwrap_or_default(),
                size: c.size as u32,
            })
            .collect();

        info!(session_id = sid, count = cookies.len(), "exported cookies");

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox).with_cookies(cookies),
        ))
    }

    /// Import cookies into the browser session.
    async fn import_cookies(
        &self,
        session_id: Option<&str>,
        cookies: Vec<crate::types::CookieParam>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        use chromiumoxide::cdp::browser_protocol::network::{CookieSameSite, SetCookieParams};

        let sid = require_session(session_id, "import_cookies")?;
        let page = self.pool.get_page(&sid).await?;

        for cookie in &cookies {
            let mut params = SetCookieParams::builder()
                .name(&cookie.name)
                .value(&cookie.value);

            if let Some(ref domain) = cookie.domain {
                params = params.domain(domain);
            }
            if let Some(ref path) = cookie.path {
                params = params.path(path);
            }
            if cookie.secure {
                params = params.secure(true);
            }
            if cookie.http_only {
                params = params.http_only(true);
            }
            if let Some(expires) = cookie.expires {
                use chromiumoxide::cdp::browser_protocol::network::TimeSinceEpoch;
                params = params.expires(TimeSinceEpoch::new(expires));
            }
            if let Some(ref ss) = cookie.same_site {
                let same_site = match ss.to_lowercase().as_str() {
                    "strict" => CookieSameSite::Strict,
                    "lax" => CookieSameSite::Lax,
                    _ => CookieSameSite::None,
                };
                params = params.same_site(same_site);
            }

            let cmd = params.build().map_err(|e| Error::Cdp(e.to_string()))?;
            page.execute(cmd)
                .await
                .map_err(|e| Error::Cdp(format!("failed to set cookie '{}': {e}", cookie.name)))?;
        }

        info!(session_id = sid, count = cookies.len(), "imported cookies");

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Close the browser session.
    async fn close(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), Error> {
        let sid = require_session(session_id, "close")?;

        self.pool.close_session(&sid).await?;

        info!(session_id = sid, "closed browser session");

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Highlight an element (for screenshots).
    async fn highlight_element(&self, page: &Page, ref_: u32) -> Result<(), Error> {
        let js = format!(
            r#"(() => {{
                const el = document.querySelector(`[data-moltis-ref="{ref_}"]`);
                if (el) {{
                    el.style.outline = '3px solid #ff0000';
                    el.style.outlineOffset = '2px';
                }}
            }})()"#
        );

        page.evaluate(js.as_str())
            .await
            .map_err(|e| Error::JsEvalFailed(e.to_string()))?;

        Ok(())
    }

    /// Remove all element highlights.
    async fn remove_highlights(&self, page: &Page) -> Result<(), Error> {
        let js = r#"
            document.querySelectorAll('[data-moltis-ref]').forEach(el => {
                el.style.outline = '';
                el.style.outlineOffset = '';
            });
        "#;

        page.evaluate(js)
            .await
            .map_err(|e| Error::JsEvalFailed(e.to_string()))?;

        Ok(())
    }

    /// Close a specific browser session by ID.
    pub async fn close_session(&self, session_id: &str) {
        if let Err(e) = self.pool.close_session(session_id).await {
            warn!(session_id, error = %e, "failed to close browser session");
        }
    }

    /// Clean up idle browser instances.
    pub async fn cleanup_idle(&self) {
        self.pool.cleanup_idle().await;
    }

    /// Get the screencast registry (for subscribing to frame events).
    pub fn screencasts(&self) -> &Arc<ScreencastRegistry> {
        &self.screencasts
    }

    /// Get a page by session ID (for direct CDP access from API handlers).
    pub async fn get_page(&self, session_id: &str) -> Result<Page, Error> {
        self.pool.get_page(session_id).await
    }

    /// Shut down all browser instances.
    pub async fn shutdown(&self) {
        self.screencasts.stop_all().await;
        self.pool.shutdown().await;
    }

    /// Get the number of active browser instances.
    pub async fn active_count(&self) -> usize {
        self.pool.active_count().await
    }

    /// List all active browser sessions with metadata.
    pub async fn list_sessions(&self) -> Vec<crate::pool::BrowserSessionInfo> {
        let mut sessions = self.pool.list_sessions().await;
        // Annotate which sessions have active screencasts.
        let screencast_sessions = self.screencasts.active_sessions().await;
        for session in &mut sessions {
            // The BrowserSessionInfo doesn't have a screencast field yet,
            // but clients can check the screencast endpoint separately.
            let _ = screencast_sessions.contains(&session.session_id);
        }
        sessions
    }
}

/// Map DOM `KeyboardEvent.key` to Windows virtual key code for CDP.
///
/// CDP's `Input.dispatchKeyEvent` requires `windowsVirtualKeyCode` for
/// non-printable keys (Backspace, Enter, arrows, etc.) to work correctly.
fn key_to_windows_virtual_keycode(key: &str) -> Option<i64> {
    match key {
        "Backspace" => Some(8),
        "Tab" => Some(9),
        "Enter" => Some(13),
        "Shift" => Some(16),
        "Control" => Some(17),
        "Alt" => Some(18),
        "Escape" => Some(27),
        " " => Some(32),
        "PageUp" => Some(33),
        "PageDown" => Some(34),
        "End" => Some(35),
        "Home" => Some(36),
        "ArrowLeft" => Some(37),
        "ArrowUp" => Some(38),
        "ArrowRight" => Some(39),
        "ArrowDown" => Some(40),
        "Insert" => Some(45),
        "Delete" => Some(46),
        _ => None,
    }
}

/// Validate a URL before attempting navigation.
///
/// Checks for:
/// - Valid URL structure (can be parsed)
/// - Allowed schemes (http, https)
/// - Not obviously malformed (LLM garbage in path)
fn validate_url(url: &str) -> Result<(), Error> {
    // Check if URL is empty
    if url.is_empty() {
        return Err(Error::InvalidAction("URL cannot be empty".to_string()));
    }

    // Parse the URL
    let parsed = url::Url::parse(url)
        .map_err(|e| Error::InvalidAction(format!("invalid URL '{}': {}", truncate_url(url), e)))?;

    // Check scheme — allow about:blank for new empty sessions
    match parsed.scheme() {
        "http" | "https" => {},
        "about" if url == "about:blank" => return Ok(()),
        scheme => {
            return Err(Error::InvalidAction(format!(
                "unsupported URL scheme '{}', only http/https allowed",
                scheme
            )));
        },
    }

    // Check for obviously malformed URLs (LLM garbage)
    // Check the original URL string (before normalization) to catch garbage
    let suspicious_patterns = [
        "}}}",           // JSON garbage
        "]}",            // JSON array closing
        "}<",            // Mixed JSON/XML
        "assistant to=", // LLM prompt leakage
        "functions.",    // LLM function call leakage (e.g., "functions.browser")
    ];

    for pattern in suspicious_patterns {
        if url.contains(pattern) {
            warn!(
                url = %truncate_url(url),
                pattern = pattern,
                "rejecting URL with suspicious pattern (likely LLM garbage)"
            );
            return Err(Error::InvalidAction(format!(
                "URL contains invalid characters or LLM garbage: '{}'",
                truncate_url(url)
            )));
        }
    }

    Ok(())
}

/// Truncate a URL for error messages (to avoid huge garbage URLs in logs).
fn truncate_url(url: &str) -> String {
    if url.len() > 100 {
        format!("{}...", &url[..url.floor_char_boundary(100)])
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BrowserConfig::default();
        assert!(config.enabled);
        assert!(config.headless);
        assert_eq!(config.max_instances, 0); // 0 = unlimited, limited by memory
        assert_eq!(config.memory_limit_percent, 90);
    }

    #[test]
    fn test_browser_manager_enabled_by_default() {
        let manager = BrowserManager::default();
        assert!(manager.is_enabled());
    }

    #[test]
    fn test_validate_url_valid() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://localhost:8080/path").is_ok());
        assert!(validate_url("https://www.lemonde.fr/").is_ok());
    }

    #[test]
    fn test_validate_url_empty() {
        assert!(validate_url("").is_err());
    }

    #[test]
    fn test_validate_url_invalid_scheme() {
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn test_validate_url_llm_garbage() {
        // The actual garbage URL from the bug report (contains "assistant to=")
        let garbage = "https://www.lemonde.fr/path>assistant to=functions.browser";
        assert!(validate_url(garbage).is_err());

        // LLM function leakage
        assert!(validate_url("https://example.com/path/functions.browser").is_err());

        // Test with the closing brace pattern from JSON garbage
        // Note: `}}<` would match the `}<` pattern
        assert!(validate_url("https://example.com/path}}<tag").is_err());
    }

    #[test]
    fn test_validate_url_malformed() {
        assert!(validate_url("not a url").is_err());
        assert!(validate_url("://missing.scheme").is_err());
    }

    #[test]
    fn test_truncate_url_handles_multibyte_boundary() {
        let url = format!("https://{}л{}", "a".repeat(91), "tail");
        let truncated = truncate_url(&url);
        let prefix = truncated.strip_suffix("...").unwrap_or("");
        assert_eq!(prefix.len(), 99);
        assert!(!prefix.contains('л'));
        assert!(prefix.ends_with('a'));
    }

    #[tokio::test]
    async fn manager_close_session_nonexistent_is_noop() {
        let manager = BrowserManager::default();
        // Should not panic — logs a warning and returns.
        manager.close_session("nonexistent").await;
    }

    #[tokio::test]
    async fn manager_cleanup_idle_empty() {
        let manager = BrowserManager::default();
        manager.cleanup_idle().await;
        assert_eq!(manager.active_count().await, 0);
    }

    #[tokio::test]
    async fn manager_shutdown_empty() {
        let manager = BrowserManager::default();
        manager.shutdown().await;
        assert_eq!(manager.active_count().await, 0);
    }

    #[tokio::test]
    async fn cleanup_stale_session_returns_connection_closed() {
        let manager = BrowserManager::default();
        let err = manager.cleanup_stale_session("sess-42", "screenshot").await;
        assert!(
            err.is_connection_error(),
            "cleanup_stale_session must return a connection error"
        );
        let msg = err.to_string();
        assert!(msg.contains("sess-42"), "error should mention session id");
        assert!(
            msg.contains("screenshot"),
            "error should mention the action"
        );
    }

    #[test]
    fn key_to_vk_backspace() {
        assert_eq!(key_to_windows_virtual_keycode("Backspace"), Some(8));
    }

    #[test]
    fn key_to_vk_enter() {
        assert_eq!(key_to_windows_virtual_keycode("Enter"), Some(13));
    }

    #[test]
    fn key_to_vk_arrows() {
        assert_eq!(key_to_windows_virtual_keycode("ArrowLeft"), Some(37));
        assert_eq!(key_to_windows_virtual_keycode("ArrowUp"), Some(38));
        assert_eq!(key_to_windows_virtual_keycode("ArrowRight"), Some(39));
        assert_eq!(key_to_windows_virtual_keycode("ArrowDown"), Some(40));
    }

    #[test]
    fn key_to_vk_printable_returns_none() {
        assert_eq!(key_to_windows_virtual_keycode("a"), None);
    }

    #[test]
    fn key_to_vk_delete_escape_tab() {
        assert_eq!(key_to_windows_virtual_keycode("Delete"), Some(46));
        assert_eq!(key_to_windows_virtual_keycode("Escape"), Some(27));
        assert_eq!(key_to_windows_virtual_keycode("Tab"), Some(9));
    }

    /// Shared screencast integration test logic.
    ///
    /// Navigates to a page, starts screencast, subscribes, and verifies at
    /// least one frame arrives through the broadcast channel.
    async fn assert_screencast_frames_arrive(sandbox: bool) {
        use tokio::time::{Duration, timeout};

        let manager = BrowserManager::default();

        // Navigate to create a session
        let request = BrowserRequest {
            session_id: None,
            action: BrowserAction::Navigate {
                url: "https://example.com".to_string(),
            },
            timeout_ms: 30000,
            sandbox: Some(sandbox),
            browser: None,
            profile_id: None,
        };
        let resp = manager.handle_request(request).await;
        assert!(resp.success, "navigate failed: {:?}", resp.error);
        let session_id = resp.session_id.clone();
        assert!(!session_id.is_empty());

        // Start screencast
        let request = BrowserRequest {
            session_id: Some(session_id.clone()),
            action: BrowserAction::StartScreencast {
                quality: 60,
                max_width: 800,
                max_height: 600,
            },
            timeout_ms: 10000,
            sandbox: Some(sandbox),
            browser: None,
            profile_id: None,
        };
        let resp = manager.handle_request(request).await;
        assert!(resp.success, "start_screencast failed: {:?}", resp.error);

        // Subscribe to the screencast frames
        let mut rx = manager
            .screencasts()
            .subscribe(&session_id)
            .await
            .unwrap_or_else(|| panic!("subscribe should succeed after start_screencast"));

        // Wait for at least one frame (with timeout)
        let frame = timeout(Duration::from_secs(15), rx.recv()).await;
        assert!(frame.is_ok(), "timed out waiting for screencast frame");
        let frame = frame
            .unwrap_or_else(|_| panic!("timeout"))
            .unwrap_or_else(|e| panic!("recv error: {e}"));
        assert_eq!(frame.session_id, session_id);
        assert!(!frame.data.is_empty(), "frame data should not be empty");
        assert!(frame.sequence >= 1, "sequence should be at least 1");

        // Cleanup
        manager.close_session(&session_id).await;
    }

    /// Host mode: screencast frames arrive without sandboxing.
    ///
    ///   cargo test -p moltis-browser screencast_host -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires a real browser installed"]
    async fn screencast_host() {
        assert_screencast_frames_arrive(false).await;
    }

    /// Sandbox mode: screencast frames arrive from a containerized browser.
    /// Skips if no container runtime is available.
    ///
    ///   cargo test -p moltis-browser screencast_sandbox -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires a container runtime (Docker/Podman/Apple Container)"]
    async fn screencast_sandbox() {
        if !crate::container::is_container_available() {
            eprintln!("SKIP: no container runtime available");
            return;
        }
        assert_screencast_frames_arrive(true).await;
    }

    /// Helper: create a session with ephemeral profile, navigate, return
    /// (manager, session_id). Ephemeral profiles avoid SingletonLock
    /// conflicts when multiple tests run in parallel.
    async fn setup_session(url: &str) -> (BrowserManager, String) {
        let config = BrowserConfig {
            persist_profile: false,
            ..BrowserConfig::default()
        };
        let manager = BrowserManager::new(config);
        let resp = manager
            .handle_request(BrowserRequest {
                session_id: None,
                action: BrowserAction::Navigate {
                    url: url.to_string(),
                },
                timeout_ms: 30000,
                sandbox: Some(false),
                browser: None,
                profile_id: None,
            })
            .await;
        assert!(resp.success, "navigate failed: {:?}", resp.error);
        (manager, resp.session_id)
    }

    /// Verify mouse click events are dispatched without errors.
    ///
    ///   cargo test -p moltis-browser click_dispatches -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires a real browser installed"]
    async fn click_dispatches() {
        let (manager, sid) = setup_session("https://example.com").await;

        // Dispatch mousePressed + mouseReleased (a click) at viewport center
        for event_type in [
            BrowserAction::MouseInput {
                x: 400.0,
                y: 300.0,
                event_type: MouseInputType::Pressed,
                button: MouseInputButton::Left,
                click_count: 1,
                delta_x: 0.0,
                delta_y: 0.0,
            },
            BrowserAction::MouseInput {
                x: 400.0,
                y: 300.0,
                event_type: MouseInputType::Released,
                button: MouseInputButton::Left,
                click_count: 1,
                delta_x: 0.0,
                delta_y: 0.0,
            },
        ] {
            let resp = manager
                .handle_request(BrowserRequest {
                    session_id: Some(sid.clone()),
                    action: event_type,
                    timeout_ms: 5000,
                    sandbox: Some(false),
                    browser: None,
                    profile_id: None,
                })
                .await;
            assert!(resp.success, "mouse event failed: {:?}", resp.error);
        }

        manager.close_session(&sid).await;
    }

    /// Verify mouseWheel scroll events are dispatched without errors.
    ///
    ///   cargo test -p moltis-browser scroll_dispatches -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires a real browser installed"]
    async fn scroll_dispatches() {
        let (manager, sid) = setup_session("https://example.com").await;

        let resp = manager
            .handle_request(BrowserRequest {
                session_id: Some(sid.clone()),
                action: BrowserAction::MouseInput {
                    x: 400.0,
                    y: 300.0,
                    event_type: MouseInputType::Wheel,
                    button: MouseInputButton::Left,
                    click_count: 0,
                    delta_x: 0.0,
                    delta_y: 300.0,
                },
                timeout_ms: 5000,
                sandbox: Some(false),
                browser: None,
                profile_id: None,
            })
            .await;
        assert!(resp.success, "scroll failed: {:?}", resp.error);

        manager.close_session(&sid).await;
    }

    /// Verify screencast frame metadata contains valid viewport dimensions
    /// that can be used for coordinate mapping.
    ///
    ///   cargo test -p moltis-browser screencast_metadata_valid -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires a real browser installed"]
    async fn screencast_metadata_valid() {
        use tokio::time::{Duration, timeout};

        let (manager, sid) = setup_session("https://example.com").await;

        let resp = manager
            .handle_request(BrowserRequest {
                session_id: Some(sid.clone()),
                action: BrowserAction::StartScreencast {
                    quality: 60,
                    max_width: 1280,
                    max_height: 800,
                },
                timeout_ms: 10000,
                sandbox: Some(false),
                browser: None,
                profile_id: None,
            })
            .await;
        assert!(resp.success, "start_screencast failed: {:?}", resp.error);

        let mut rx = manager
            .screencasts()
            .subscribe(&sid)
            .await
            .unwrap_or_else(|| panic!("subscribe failed"));

        let frame = timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("timeout"))
            .unwrap_or_else(|e| panic!("recv error: {e}"));

        // Verify metadata has positive viewport dimensions
        assert!(
            frame.metadata.device_width > 0.0,
            "device_width should be positive, got {}",
            frame.metadata.device_width
        );
        assert!(
            frame.metadata.device_height > 0.0,
            "device_height should be positive, got {}",
            frame.metadata.device_height
        );
        // offset_top should be non-negative
        assert!(
            frame.metadata.offset_top >= 0.0,
            "offset_top should be >= 0, got {}",
            frame.metadata.offset_top
        );

        eprintln!(
            "Frame metadata: device={}x{}, offset_top={}, page_scale={}",
            frame.metadata.device_width,
            frame.metadata.device_height,
            frame.metadata.offset_top,
            frame.metadata.page_scale_factor
        );

        manager.close_session(&sid).await;
    }
}
