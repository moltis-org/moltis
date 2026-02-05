//! Browser action types and request/response structures.

use serde::{Deserialize, Serialize};

/// Browser action to perform.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserAction {
    /// Navigate to a URL.
    Navigate { url: String },

    /// Take a screenshot of the current page.
    Screenshot {
        #[serde(default)]
        full_page: bool,
        /// Optional: highlight element by ref before screenshot.
        #[serde(default)]
        highlight_ref: Option<u32>,
    },

    /// Get a DOM snapshot with numbered element references.
    Snapshot,

    /// Click an element by its reference number.
    Click { ref_: u32 },

    /// Type text into an element.
    Type { ref_: u32, text: String },

    /// Scroll the page or an element.
    Scroll {
        /// Element ref to scroll (None = viewport).
        #[serde(default)]
        ref_: Option<u32>,
        /// Horizontal scroll delta.
        #[serde(default)]
        x: i32,
        /// Vertical scroll delta.
        #[serde(default)]
        y: i32,
    },

    /// Execute JavaScript in the page context.
    Evaluate { code: String },

    /// Wait for an element to appear (by CSS selector or ref).
    Wait {
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        ref_: Option<u32>,
        #[serde(default = "default_wait_timeout_ms")]
        timeout_ms: u64,
    },

    /// Get the current page URL.
    GetUrl,

    /// Get the page title.
    GetTitle,

    /// Go back in history.
    Back,

    /// Go forward in history.
    Forward,

    /// Refresh the page.
    Refresh,

    /// Close the browser session.
    Close,
}

fn default_wait_timeout_ms() -> u64 {
    30000
}

/// Request to the browser service.
#[derive(Debug, Clone, Deserialize)]
pub struct BrowserRequest {
    /// Browser session ID (optional - creates new if missing).
    #[serde(default)]
    pub session_id: Option<String>,

    /// The action to perform.
    #[serde(flatten)]
    pub action: BrowserAction,

    /// Global timeout in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    60000
}

/// Element reference in a DOM snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct ElementRef {
    /// Unique reference number for this element.
    pub ref_: u32,
    /// Tag name (e.g., "button", "input", "a").
    pub tag: String,
    /// Element's role attribute or inferred role.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Visible text content (truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Link href (for anchor elements).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    /// Input placeholder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Input value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// aria-label attribute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aria_label: Option<String>,
    /// Whether the element is visible in the viewport.
    pub visible: bool,
    /// Whether the element is interactive (clickable/editable).
    pub interactive: bool,
    /// Bounding box in viewport coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<ElementBounds>,
}

/// Bounding box for an element.
#[derive(Debug, Clone, Serialize)]
pub struct ElementBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// DOM snapshot with element references.
#[derive(Debug, Clone, Serialize)]
pub struct DomSnapshot {
    /// Current page URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Interactive elements with reference numbers.
    pub elements: Vec<ElementRef>,
    /// Viewport dimensions.
    pub viewport: ViewportSize,
    /// Total page scroll dimensions.
    pub scroll: ScrollDimensions,
}

/// Viewport size.
#[derive(Debug, Clone, Serialize)]
pub struct ViewportSize {
    pub width: u32,
    pub height: u32,
}

/// Scroll dimensions.
#[derive(Debug, Clone, Serialize)]
pub struct ScrollDimensions {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Response from a browser action.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserResponse {
    /// Whether the action succeeded.
    pub success: bool,

    /// Session ID for this browser instance.
    pub session_id: String,

    /// Error message if action failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Screenshot as base64 PNG (for screenshot action).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,

    /// DOM snapshot (for snapshot action).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<DomSnapshot>,

    /// JavaScript evaluation result (for evaluate action).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,

    /// Current URL (for navigate, get_url, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Page title (for get_title, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Duration of the action in milliseconds.
    pub duration_ms: u64,
}

impl BrowserResponse {
    pub fn success(session_id: String, duration_ms: u64) -> Self {
        Self {
            success: true,
            session_id,
            error: None,
            screenshot: None,
            snapshot: None,
            result: None,
            url: None,
            title: None,
            duration_ms,
        }
    }

    pub fn error(session_id: String, error: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            success: false,
            session_id,
            error: Some(error.into()),
            screenshot: None,
            snapshot: None,
            result: None,
            url: None,
            title: None,
            duration_ms,
        }
    }

    pub fn with_screenshot(mut self, screenshot: String) -> Self {
        self.screenshot = Some(screenshot);
        self
    }

    pub fn with_snapshot(mut self, snapshot: DomSnapshot) -> Self {
        self.snapshot = Some(snapshot);
        self
    }

    pub fn with_result(mut self, result: serde_json::Value) -> Self {
        self.result = Some(result);
        self
    }

    pub fn with_url(mut self, url: String) -> Self {
        self.url = Some(url);
        self
    }

    pub fn with_title(mut self, title: String) -> Self {
        self.title = Some(title);
        self
    }
}

/// Browser configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    /// Whether browser support is enabled.
    pub enabled: bool,
    /// Path to Chrome/Chromium binary (auto-detected if not set).
    pub chrome_path: Option<String>,
    /// Whether to run in headless mode.
    pub headless: bool,
    /// Default viewport width.
    pub viewport_width: u32,
    /// Default viewport height.
    pub viewport_height: u32,
    /// Maximum concurrent browser instances.
    pub max_instances: usize,
    /// Instance idle timeout in seconds before closing.
    pub idle_timeout_secs: u64,
    /// Default navigation timeout in milliseconds.
    pub navigation_timeout_ms: u64,
    /// User agent string (uses default if not set).
    pub user_agent: Option<String>,
    /// Additional Chrome arguments.
    #[serde(default)]
    pub chrome_args: Vec<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            chrome_path: None,
            headless: true,
            viewport_width: 1280,
            viewport_height: 720,
            max_instances: 3,
            idle_timeout_secs: 300,
            navigation_timeout_ms: 30000,
            user_agent: None,
            chrome_args: Vec::new(),
        }
    }
}

impl From<&moltis_config::schema::BrowserConfig> for BrowserConfig {
    fn from(cfg: &moltis_config::schema::BrowserConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            chrome_path: cfg.chrome_path.clone(),
            headless: cfg.headless,
            viewport_width: cfg.viewport_width,
            viewport_height: cfg.viewport_height,
            max_instances: cfg.max_instances,
            idle_timeout_secs: cfg.idle_timeout_secs,
            navigation_timeout_ms: cfg.navigation_timeout_ms,
            user_agent: cfg.user_agent.clone(),
            chrome_args: cfg.chrome_args.clone(),
        }
    }
}
