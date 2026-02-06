//! Browser automation tool for LLM agents.
//!
//! This tool provides full browser automation capabilities including:
//! - Navigation with JavaScript execution
//! - Screenshots of pages
//! - DOM snapshots with numbered element references
//! - Clicking, typing, scrolling on elements
//! - JavaScript evaluation

use {
    anyhow::Result,
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    moltis_browser::{BrowserManager, BrowserRequest},
    std::sync::Arc,
};

/// Browser automation tool for interacting with web pages.
///
/// Unlike `web_fetch` which just retrieves page content, this tool allows
/// full browser interaction: clicking buttons, filling forms, taking
/// screenshots, and executing JavaScript.
pub struct BrowserTool {
    manager: Arc<BrowserManager>,
}

impl BrowserTool {
    /// Create a new browser tool wrapping a browser manager.
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self { manager }
    }

    /// Create from config; returns `None` if browser is disabled.
    pub fn from_config(config: &moltis_config::schema::BrowserConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        let browser_config = moltis_browser::BrowserConfig::from(config);
        let manager = Arc::new(BrowserManager::new(browser_config));
        Some(Self::new(manager))
    }
}

#[async_trait]
impl AgentTool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Control a real browser to interact with web pages.\n\n\
         REQUIRED: You MUST specify an 'action' parameter. Example:\n\
         {\"action\": \"navigate\", \"url\": \"https://example.com\"}\n\n\
         Actions: navigate, screenshot, snapshot, click, type, scroll, evaluate, wait, close\n\n\
         WORKFLOW:\n\
         1. {\"action\": \"navigate\", \"url\": \"...\"} - go to URL, returns session_id\n\
         2. {\"action\": \"snapshot\", \"session_id\": \"...\"} - get interactive elements\n\
         3. {\"action\": \"click\", \"session_id\": \"...\", \"ref_\": N} - click element\n\
         4. {\"action\": \"screenshot\", \"session_id\": \"...\"} - capture the view\n\n\
         For simple page content retrieval, prefer `web_fetch` as it's faster."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "screenshot", "snapshot", "click", "type", "scroll", "evaluate", "wait", "get_url", "get_title", "back", "forward", "refresh", "close"],
                    "description": "REQUIRED. The browser action to perform. Use 'navigate' with 'url' to open a page, 'snapshot' to see elements, 'screenshot' to capture."
                },
                "session_id": {
                    "type": "string",
                    "description": "Browser session ID (omit to create new session, or reuse existing)"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to (for 'navigate' action)"
                },
                "ref_": {
                    "type": "integer",
                    "description": "Element reference number from snapshot (for click/type/scroll)"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type (for 'type' action)"
                },
                "code": {
                    "type": "string",
                    "description": "JavaScript code to execute (for 'evaluate' action)"
                },
                "x": {
                    "type": "integer",
                    "description": "Horizontal scroll pixels (for 'scroll' action)"
                },
                "y": {
                    "type": "integer",
                    "description": "Vertical scroll pixels (for 'scroll' action)"
                },
                "full_page": {
                    "type": "boolean",
                    "description": "Capture full page screenshot vs viewport only"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector to wait for (for 'wait' action)"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default: 60000)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let request: BrowserRequest = serde_json::from_value(params)?;
        let response = self.manager.handle_request(request).await;
        Ok(serde_json::to_value(&response)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name() {
        let config = moltis_config::schema::BrowserConfig {
            enabled: true,
            ..Default::default()
        };
        let tool = BrowserTool::from_config(&config).unwrap();
        assert_eq!(tool.name(), "browser");
    }

    #[test]
    fn test_disabled_returns_none() {
        let config = moltis_config::schema::BrowserConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(BrowserTool::from_config(&config).is_none());
    }
}
