//! Browser manager providing high-level browser automation actions.

use std::{sync::Arc, time::Instant};

use {
    base64::{Engine, engine::general_purpose::STANDARD as BASE64},
    chromiumoxide::{
        Page,
        cdp::browser_protocol::{
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
    error::BrowserError,
    pool::BrowserPool,
    snapshot::{
        extract_snapshot, find_element_by_ref, focus_element_by_ref, scroll_element_into_view,
    },
    types::{BrowserAction, BrowserConfig, BrowserRequest, BrowserResponse},
};

/// Extract session_id or return an error for actions that require an existing session.
fn require_session(session_id: Option<&str>, action: &str) -> Result<String, BrowserError> {
    session_id
        .map(String::from)
        .ok_or_else(|| BrowserError::InvalidAction(format!("{action} requires a session_id")))
}

/// Manage Chrome/Chromium instances with CDP.
pub struct BrowserManager {
    pool: Arc<BrowserPool>,
    config: BrowserConfig,
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
        }
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

        // Log the action with execution mode for visibility
        let mode = if sandbox {
            "sandbox"
        } else {
            "host"
        };
        info!(
            action = %request.action,
            session_id = request.session_id.as_deref().unwrap_or("(new)"),
            execution_mode = mode,
            sandbox_image = %self.config.sandbox_image,
            "executing browser action"
        );

        let start = Instant::now();
        let timeout_duration = Duration::from_millis(request.timeout_ms);

        match timeout(
            timeout_duration,
            self.execute_action(request.session_id.as_deref(), request.action, sandbox),
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
        }
    }

    /// Execute a browser action.
    async fn execute_action(
        &self,
        session_id: Option<&str>,
        action: BrowserAction,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        match action {
            BrowserAction::Navigate { url } => self.navigate(session_id, &url, sandbox).await,
            BrowserAction::Screenshot {
                full_page,
                highlight_ref,
            } => {
                self.screenshot(session_id, full_page, highlight_ref, sandbox)
                    .await
            },
            BrowserAction::Snapshot => self.snapshot(session_id, sandbox).await,
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
        }
    }

    /// Navigate to a URL.
    async fn navigate(
        &self,
        session_id: Option<&str>,
        url: &str,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        // Validate URL before navigation
        validate_url(url)?;

        // Check if the domain is allowed
        if !crate::types::is_domain_allowed(url, &self.config.allowed_domains) {
            return Err(BrowserError::NavigationFailed(format!(
                "domain not in allowed list. Allowed domains: {:?}",
                self.config.allowed_domains
            )));
        }

        let sid = self.pool.get_or_create(session_id, sandbox).await?;
        let page = self.pool.get_page(&sid).await?;

        #[cfg(feature = "metrics")]
        let nav_start = Instant::now();

        // Try navigation, retry with fresh session if connection is dead
        if let Err(e) = page.goto(url).await {
            let err_str = e.to_string();
            if err_str.contains("AlreadyClosed") || err_str.contains("ConnectionClosed") {
                warn!(
                    session_id = sid,
                    "browser connection dead, closing session and retrying"
                );
                let _ = self.pool.close_session(&sid).await;
                // Retry with a fresh session (use same sandbox mode)
                let new_sid = self.pool.get_or_create(None, sandbox).await?;
                let new_page = self.pool.get_page(&new_sid).await?;
                new_page
                    .goto(url)
                    .await
                    .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;
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
            return Err(BrowserError::NavigationFailed(err_str));
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
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = self.pool.get_or_create(session_id, sandbox).await?;
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
            .map_err(|e| BrowserError::ScreenshotFailed(e.to_string()))?;

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
    async fn snapshot(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = self.pool.get_or_create(session_id, sandbox).await?;
        let page = self.pool.get_page(&sid).await?;

        // Try snapshot, retry with fresh session if connection is dead
        let snapshot = match extract_snapshot(&page).await {
            Ok(s) => s,
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("AlreadyClosed") || err_str.contains("ConnectionClosed") {
                    warn!(
                        session_id = sid,
                        "browser connection dead, closing session and retrying"
                    );
                    let _ = self.pool.close_session(&sid).await;
                    // For snapshot we need an existing page, so return error
                    return Err(BrowserError::ConnectionClosed(
                        "Browser connection closed. Please navigate to a page first.".to_string(),
                    ));
                }
                return Err(e);
            },
        };

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
    ) -> Result<(String, BrowserResponse), BrowserError> {
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
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        page.execute(press_cmd)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        let release_cmd = DispatchMouseEventParams::builder()
            .r#type(DispatchMouseEventType::MouseReleased)
            .x(x)
            .y(y)
            .button(MouseButton::Left)
            .click_count(1)
            .build()
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        page.execute(release_cmd)
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

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
    ) -> Result<(String, BrowserResponse), BrowserError> {
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
                .map_err(|e| BrowserError::Cdp(e.to_string()))?;
            page.execute(key_down)
                .await
                .map_err(|e| BrowserError::Cdp(e.to_string()))?;

            let key_up = DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .text(c.to_string())
                .build()
                .map_err(|e| BrowserError::Cdp(e.to_string()))?;
            page.execute(key_up)
                .await
                .map_err(|e| BrowserError::Cdp(e.to_string()))?;
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
    ) -> Result<(String, BrowserResponse), BrowserError> {
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
            .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

        debug!(session_id = sid, ref_ = ?ref_, x = x, y = y, "scrolled");

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Execute JavaScript in the page context.
    async fn evaluate(
        &self,
        session_id: Option<&str>,
        code: &str,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "evaluate")?;

        let page = self.pool.get_page(&sid).await?;

        let result: serde_json::Value = page
            .evaluate(code)
            .await
            .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
            .into_value()
            .map_err(|e| BrowserError::JsEvalFailed(format!("{e:?}")))?;

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
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "wait")?;

        let page = self.pool.get_page(&sid).await?;

        let check_js = if let Some(ref selector) = selector {
            format!(
                r#"document.querySelector({}) !== null"#,
                serde_json::to_string(selector).map_err(|e| BrowserError::Cdp(e.to_string()))?
            )
        } else if let Some(ref_) = ref_ {
            format!(r#"document.querySelector('[data-moltis-ref="{ref_}"]') !== null"#)
        } else {
            return Err(BrowserError::InvalidAction(
                "wait requires selector or ref".into(),
            ));
        };

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let interval = Duration::from_millis(100);

        while Instant::now() < deadline {
            let found: bool = page
                .evaluate(check_js.as_str())
                .await
                .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?
                .into_value()
                .unwrap_or(false);

            if found {
                debug!(session_id = sid, "element found");
                return Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)));
            }

            tokio::time::sleep(interval).await;
        }

        Err(BrowserError::Timeout(format!(
            "element not found after {}ms",
            timeout_ms
        )))
    }

    /// Get the current page URL.
    async fn get_url(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), BrowserError> {
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
    ) -> Result<(String, BrowserResponse), BrowserError> {
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
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "back")?;

        let page = self.pool.get_page(&sid).await?;

        page.evaluate("history.back()")
            .await
            .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

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
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "forward")?;

        let page = self.pool.get_page(&sid).await?;

        page.evaluate("history.forward()")
            .await
            .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

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
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "refresh")?;

        let page = self.pool.get_page(&sid).await?;

        page.reload()
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        // Wait for navigation
        let _ = page.wait_for_navigation().await;

        let url = page.url().await.ok().flatten().unwrap_or_default();

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, sandbox).with_url(url),
        ))
    }

    /// Close the browser session.
    async fn close(
        &self,
        session_id: Option<&str>,
        sandbox: bool,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "close")?;

        self.pool.close_session(&sid).await?;

        info!(session_id = sid, "closed browser session");

        Ok((sid.clone(), BrowserResponse::success(sid, 0, sandbox)))
    }

    /// Highlight an element (for screenshots).
    async fn highlight_element(&self, page: &Page, ref_: u32) -> Result<(), BrowserError> {
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
            .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

        Ok(())
    }

    /// Remove all element highlights.
    async fn remove_highlights(&self, page: &Page) -> Result<(), BrowserError> {
        let js = r#"
            document.querySelectorAll('[data-moltis-ref]').forEach(el => {
                el.style.outline = '';
                el.style.outlineOffset = '';
            });
        "#;

        page.evaluate(js)
            .await
            .map_err(|e| BrowserError::JsEvalFailed(e.to_string()))?;

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

    /// Shut down all browser instances.
    pub async fn shutdown(&self) {
        self.pool.shutdown().await;
    }

    /// Get the number of active browser instances.
    pub async fn active_count(&self) -> usize {
        self.pool.active_count().await
    }
}

/// Validate a URL before attempting navigation.
///
/// Checks for:
/// - Valid URL structure (can be parsed)
/// - Allowed schemes (http, https)
/// - Not obviously malformed (LLM garbage in path)
fn validate_url(url: &str) -> Result<(), BrowserError> {
    // Check if URL is empty
    if url.is_empty() {
        return Err(BrowserError::InvalidAction(
            "URL cannot be empty".to_string(),
        ));
    }

    // Parse the URL
    let parsed = url::Url::parse(url).map_err(|e| {
        BrowserError::InvalidAction(format!("invalid URL '{}': {}", truncate_url(url), e))
    })?;

    // Check scheme
    match parsed.scheme() {
        "http" | "https" => {},
        scheme => {
            return Err(BrowserError::InvalidAction(format!(
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
            return Err(BrowserError::InvalidAction(format!(
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
        format!("{}...", &url[..100])
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
}
