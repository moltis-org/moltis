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
    tokio::sync::RwLock,
    tracing::debug,
};

/// Browser automation tool for interacting with web pages.
///
/// Unlike `web_fetch` which just retrieves page content, this tool allows
/// full browser interaction: clicking buttons, filling forms, taking
/// screenshots, and executing JavaScript.
///
/// This tool automatically tracks and reuses browser session IDs. When
/// the LLM doesn't provide a session_id (or provides empty string), the
/// tool will use the most recently created session. This prevents pool
/// exhaustion from creating new browser instances on every call.
pub struct BrowserTool {
    manager: Arc<BrowserManager>,
    /// Track the most recent session ID for automatic reuse.
    /// This prevents pool exhaustion when the LLM forgets to pass session_id.
    last_session_id: RwLock<Option<String>>,
}

impl BrowserTool {
    /// Create a new browser tool wrapping a browser manager.
    pub fn new(manager: Arc<BrowserManager>) -> Self {
        Self {
            manager,
            last_session_id: RwLock::new(None),
        }
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

    /// Clear the tracked session ID (e.g., after explicit close).
    async fn clear_session(&self) {
        let mut guard = self.last_session_id.write().await;
        *guard = None;
    }

    /// Save the session ID for future reuse.
    async fn save_session(&self, session_id: &str) {
        if !session_id.is_empty() {
            let mut guard = self.last_session_id.write().await;
            *guard = Some(session_id.to_string());
        }
    }

    /// Get the tracked session ID if available.
    async fn get_saved_session(&self) -> Option<String> {
        let guard = self.last_session_id.read().await;
        guard.clone()
    }
}

#[async_trait]
impl AgentTool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Control a real browser to interact with web pages.\n\n\
         USE THIS TOOL when the user says 'browse', 'browser', 'open in browser', \
         or needs interaction (clicking, forms, screenshots, JavaScript-heavy pages).\n\n\
         REQUIRED: You MUST specify an 'action' parameter. Example:\n\
         {\"action\": \"navigate\", \"url\": \"https://example.com\"}\n\n\
         Actions: navigate, screenshot, snapshot, click, type, scroll, evaluate, wait, close\n\n\
         SESSION: The browser session is automatically tracked. After 'navigate', \
         subsequent actions will reuse the same browser. No need to pass session_id.\n\n\
         WORKFLOW:\n\
         1. {\"action\": \"navigate\", \"url\": \"...\"} - opens URL in browser\n\
         2. {\"action\": \"snapshot\"} - get interactive elements with ref numbers\n\
         3. {\"action\": \"click\", \"ref_\": N} - click element by ref number\n\
         4. {\"action\": \"screenshot\"} - capture the current view\n\
         5. {\"action\": \"close\"} - close the browser when done"
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
        let mut params = params;

        // Extract sandbox mode from context (injected by gateway based on session sandbox mode).
        // The browser should be sandboxed when the chat session is sandboxed.
        let sandbox_mode = params
            .get("_sandbox")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Inject saved session_id if LLM didn't provide one (or provided empty string)
        if let Some(obj) = params.as_object_mut() {
            let needs_session = match obj.get("session_id") {
                None => true,
                Some(serde_json::Value::String(s)) if s.is_empty() => true,
                Some(serde_json::Value::Null) => true,
                _ => false,
            };

            if needs_session && let Some(saved_sid) = self.get_saved_session().await {
                debug!(
                    session_id = %saved_sid,
                    "injecting saved session_id (LLM didn't provide one)"
                );
                obj.insert("session_id".to_string(), serde_json::json!(saved_sid));
            }

            // Inject sandbox mode from session context
            obj.insert("sandbox".to_string(), serde_json::json!(sandbox_mode));
        }

        // Check if this is a "close" action - we'll clear saved session after
        let is_close = params
            .get("action")
            .and_then(|a| a.as_str())
            .is_some_and(|a| a == "close");

        // Try to parse the request, defaulting to "navigate" if action is missing
        let request: BrowserRequest = match serde_json::from_value(params.clone()) {
            Ok(req) => req,
            Err(e) if e.to_string().contains("missing field `action`") => {
                // Default to navigate action if action is missing but url is present
                if let Some(obj) = params.as_object_mut() {
                    if obj.contains_key("url") {
                        obj.insert("action".to_string(), serde_json::json!("navigate"));
                        serde_json::from_value(params)?
                    } else {
                        // No URL either - return helpful error
                        anyhow::bail!(
                            "Missing required 'action' field. Use: \
                             {{\"action\": \"navigate\", \"url\": \"https://...\"}} to open a page"
                        );
                    }
                } else {
                    return Err(e.into());
                }
            },
            Err(e) => return Err(e.into()),
        };

        let response = self.manager.handle_request(request).await;

        // Track the session ID for future reuse
        if response.success {
            if is_close {
                self.clear_session().await;
            } else {
                self.save_session(&response.session_id).await;
            }
        }

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

    #[test]
    fn test_parameters_schema_has_required_action() {
        let config = moltis_config::schema::BrowserConfig {
            enabled: true,
            ..Default::default()
        };
        let tool = BrowserTool::from_config(&config).unwrap();
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(
            required.iter().any(|v| v == "action"),
            "action should be in required fields"
        );
    }
}
