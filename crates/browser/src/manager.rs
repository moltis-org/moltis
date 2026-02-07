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
        // Log sandbox mode status
        if config.sandbox {
            info!(
                sandbox_image = %config.sandbox_image,
                "browser sandbox mode enabled"
            );
        } else {
            info!("browser running on host (sandbox disabled)");
        }

        Self {
            pool: Arc::new(BrowserPool::new(config.clone())),
            config,
        }
    }

    /// Check if browser is running in sandbox mode.
    pub fn is_sandboxed(&self) -> bool {
        self.config.sandbox
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

        // Log the action with execution mode for visibility
        let mode = if self.config.sandbox {
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
            self.execute_action(request.session_id.as_deref(), request.action),
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
    ) -> Result<(String, BrowserResponse), BrowserError> {
        match action {
            BrowserAction::Navigate { url } => self.navigate(session_id, &url).await,
            BrowserAction::Screenshot {
                full_page,
                highlight_ref,
            } => self.screenshot(session_id, full_page, highlight_ref).await,
            BrowserAction::Snapshot => self.snapshot(session_id).await,
            BrowserAction::Click { ref_ } => self.click(session_id, ref_).await,
            BrowserAction::Type { ref_, text } => self.type_text(session_id, ref_, &text).await,
            BrowserAction::Scroll { ref_, x, y } => self.scroll(session_id, ref_, x, y).await,
            BrowserAction::Evaluate { code } => self.evaluate(session_id, &code).await,
            BrowserAction::Wait {
                selector,
                ref_,
                timeout_ms,
            } => self.wait(session_id, selector, ref_, timeout_ms).await,
            BrowserAction::GetUrl => self.get_url(session_id).await,
            BrowserAction::GetTitle => self.get_title(session_id).await,
            BrowserAction::Back => self.go_back(session_id).await,
            BrowserAction::Forward => self.go_forward(session_id).await,
            BrowserAction::Refresh => self.refresh(session_id).await,
            BrowserAction::Close => self.close(session_id).await,
        }
    }

    /// Navigate to a URL.
    async fn navigate(
        &self,
        session_id: Option<&str>,
        url: &str,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        // Check if the domain is allowed
        if !crate::types::is_domain_allowed(url, &self.config.allowed_domains) {
            return Err(BrowserError::NavigationFailed(format!(
                "domain not in allowed list. Allowed domains: {:?}",
                self.config.allowed_domains
            )));
        }

        let sid = self.pool.get_or_create(session_id).await?;
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
                // Retry with a fresh session
                let new_sid = self.pool.get_or_create(None).await?;
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
                    BrowserResponse::success(new_sid, 0, self.config.sandbox).with_url(current_url),
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
            BrowserResponse::success(sid, 0, self.config.sandbox).with_url(current_url),
        ))
    }

    /// Take a screenshot of the page.
    async fn screenshot(
        &self,
        session_id: Option<&str>,
        full_page: bool,
        highlight_ref: Option<u32>,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = self.pool.get_or_create(session_id).await?;
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

        debug!(
            session_id = sid,
            bytes = screenshot.len(),
            "took screenshot"
        );

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, self.config.sandbox).with_screenshot(data_uri),
        ))
    }

    /// Get a DOM snapshot with element references.
    async fn snapshot(
        &self,
        session_id: Option<&str>,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = self.pool.get_or_create(session_id).await?;
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
            BrowserResponse::success(sid, 0, self.config.sandbox).with_snapshot(snapshot),
        ))
    }

    /// Click an element by reference.
    async fn click(
        &self,
        session_id: Option<&str>,
        ref_: u32,
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
        page.execute(
            DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MousePressed)
                .x(x)
                .y(y)
                .button(MouseButton::Left)
                .click_count(1)
                .build()
                .unwrap(),
        )
        .await
        .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        page.execute(
            DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MouseReleased)
                .x(x)
                .y(y)
                .button(MouseButton::Left)
                .click_count(1)
                .build()
                .unwrap(),
        )
        .await
        .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        debug!(
            session_id = sid,
            ref_ = ref_,
            x = x,
            y = y,
            "clicked element"
        );

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, self.config.sandbox),
        ))
    }

    /// Type text into an element.
    async fn type_text(
        &self,
        session_id: Option<&str>,
        ref_: u32,
        text: &str,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "type")?;

        let page = self.pool.get_page(&sid).await?;

        // Focus the element
        focus_element_by_ref(&page, ref_).await?;

        // Type each character
        for c in text.chars() {
            page.execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyDown)
                    .text(c.to_string())
                    .build()
                    .unwrap(),
            )
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

            page.execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyUp)
                    .text(c.to_string())
                    .build()
                    .unwrap(),
            )
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        }

        debug!(
            session_id = sid,
            ref_ = ref_,
            chars = text.len(),
            "typed text"
        );

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, self.config.sandbox),
        ))
    }

    /// Scroll the page or an element.
    async fn scroll(
        &self,
        session_id: Option<&str>,
        ref_: Option<u32>,
        x: i32,
        y: i32,
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

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, self.config.sandbox),
        ))
    }

    /// Execute JavaScript in the page context.
    async fn evaluate(
        &self,
        session_id: Option<&str>,
        code: &str,
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
            BrowserResponse::success(sid, 0, self.config.sandbox).with_result(result),
        ))
    }

    /// Wait for an element to appear.
    async fn wait(
        &self,
        session_id: Option<&str>,
        selector: Option<String>,
        ref_: Option<u32>,
        timeout_ms: u64,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "wait")?;

        let page = self.pool.get_page(&sid).await?;

        let check_js = if let Some(ref selector) = selector {
            format!(
                r#"document.querySelector({}) !== null"#,
                serde_json::to_string(selector).unwrap()
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
                return Ok((
                    sid.clone(),
                    BrowserResponse::success(sid, 0, self.config.sandbox),
                ));
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
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "get_url")?;

        let page = self.pool.get_page(&sid).await?;
        let url = page.url().await.ok().flatten().unwrap_or_default();

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, self.config.sandbox).with_url(url),
        ))
    }

    /// Get the page title.
    async fn get_title(
        &self,
        session_id: Option<&str>,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "get_title")?;

        let page = self.pool.get_page(&sid).await?;
        let title = page.get_title().await.ok().flatten().unwrap_or_default();

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, self.config.sandbox).with_title(title),
        ))
    }

    /// Go back in history.
    async fn go_back(
        &self,
        session_id: Option<&str>,
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
            BrowserResponse::success(sid, 0, self.config.sandbox).with_url(url),
        ))
    }

    /// Go forward in history.
    async fn go_forward(
        &self,
        session_id: Option<&str>,
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
            BrowserResponse::success(sid, 0, self.config.sandbox).with_url(url),
        ))
    }

    /// Refresh the page.
    async fn refresh(
        &self,
        session_id: Option<&str>,
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
            BrowserResponse::success(sid, 0, self.config.sandbox).with_url(url),
        ))
    }

    /// Close the browser session.
    async fn close(
        &self,
        session_id: Option<&str>,
    ) -> Result<(String, BrowserResponse), BrowserError> {
        let sid = require_session(session_id, "close")?;

        self.pool.close_session(&sid).await?;

        info!(session_id = sid, "closed browser session");

        Ok((
            sid.clone(),
            BrowserResponse::success(sid, 0, self.config.sandbox),
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BrowserConfig::default();
        assert!(config.enabled);
        assert!(config.headless);
        assert_eq!(config.max_instances, 3);
    }

    #[test]
    fn test_browser_manager_enabled_by_default() {
        let manager = BrowserManager::default();
        assert!(manager.is_enabled());
    }
}
