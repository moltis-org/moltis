//! AgentTool implementation for CalDAV calendar operations.

use std::{collections::HashMap, sync::Arc};

use {
    anyhow::{Result, anyhow},
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    moltis_config::CalDavConfig,
    serde_json::{Value, json},
};

use crate::{
    client::{LibDavCalDavClient, SharedCalDavClient},
    discovery,
    types::{NewEvent, TimeRange, UpdateEvent},
};

/// CalDAV agent tool providing calendar CRUD operations.
///
/// Connections to CalDAV servers are lazily initialised on first use.
pub struct CalDavTool {
    config: CalDavConfig,
    clients: tokio::sync::RwLock<HashMap<String, SharedCalDavClient>>,
}

impl CalDavTool {
    /// Create the tool from config, returning `None` if CalDAV is disabled
    /// or no accounts are configured.
    #[must_use]
    pub fn from_config(config: &CalDavConfig) -> Option<Self> {
        if !config.enabled || config.accounts.is_empty() {
            return None;
        }
        Some(Self {
            config: config.clone(),
            clients: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Resolve which account to use and return its client.
    async fn resolve_client(&self, account: Option<&str>) -> Result<SharedCalDavClient> {
        let account_name = account
            .or(self.config.default_account.as_deref())
            .or_else(|| {
                // If there's exactly one account, use it implicitly
                if self.config.accounts.len() == 1 {
                    self.config.accounts.keys().next().map(String::as_str)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                let names: Vec<&str> = self.config.accounts.keys().map(String::as_str).collect();
                anyhow!(
                    "multiple CalDAV accounts configured ({}), \
                     specify 'account' parameter or set caldav.default_account",
                    names.join(", ")
                )
            })?;

        // Check if already connected
        {
            let clients = self.clients.read().await;
            if let Some(client) = clients.get(account_name) {
                return Ok(Arc::clone(client));
            }
        }

        // Need to connect — get config and build client
        let account_config = self
            .config
            .accounts
            .get(account_name)
            .ok_or_else(|| anyhow!("unknown CalDAV account '{account_name}'"))?;

        let base_url = discovery::resolve_base_url(
            account_config.provider.as_deref(),
            account_config.url.as_deref(),
        )
        .ok_or_else(|| {
            anyhow!(
                "CalDAV account '{account_name}' has no URL and provider '{}' requires one",
                account_config.provider.as_deref().unwrap_or("generic")
            )
        })?;

        let username = account_config
            .username
            .as_deref()
            .ok_or_else(|| anyhow!("CalDAV account '{account_name}' has no username"))?;

        let password = account_config
            .password
            .as_ref()
            .ok_or_else(|| anyhow!("CalDAV account '{account_name}' has no password"))?;

        #[cfg(feature = "tracing")]
        tracing::info!(
            account = account_name,
            url = %base_url,
            "connecting to CalDAV server"
        );

        let client: SharedCalDavClient =
            Arc::new(LibDavCalDavClient::connect(&base_url, username, password).await?);

        let mut clients = self.clients.write().await;
        clients.insert(account_name.to_string(), Arc::clone(&client));

        Ok(client)
    }
}

#[async_trait]
impl AgentTool for CalDavTool {
    fn name(&self) -> &str {
        "caldav"
    }

    fn description(&self) -> &str {
        "Manage calendar events via CalDAV. Supports multiple accounts (Fastmail, iCloud, generic).\n\n\
         Operations:\n\
         - list_calendars: List available calendars. Returns href, display_name, color, description.\n\
         - list_events: List events in a calendar. Params: calendar (href, required), start/end (ISO 8601, optional).\n\
         - create_event: Create a new event. Params: calendar (href), summary, start (ISO 8601), end (optional), \
           all_day (bool), location, description.\n\
         - update_event: Update an existing event. Params: event_href, etag (required for concurrency), \
           plus any fields to change: summary, start, end, all_day, location, description.\n\
         - delete_event: Delete an event. Params: event_href, etag (required).\n\n\
         ETags are required for update/delete to prevent conflicts. Get them from list_events first.\n\
         If multiple accounts are configured, pass 'account' to select one."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["operation"],
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["list_calendars", "list_events", "create_event", "update_event", "delete_event"],
                    "description": "The calendar operation to perform"
                },
                "account": {
                    "type": "string",
                    "description": "CalDAV account name (optional if only one account or default is set)"
                },
                "calendar": {
                    "type": "string",
                    "description": "Calendar href path (required for list_events, create_event)"
                },
                "event_href": {
                    "type": "string",
                    "description": "Event resource href (required for update_event, delete_event)"
                },
                "etag": {
                    "type": "string",
                    "description": "Event ETag for concurrency control (required for update_event, delete_event)"
                },
                "summary": {
                    "type": "string",
                    "description": "Event title"
                },
                "start": {
                    "type": "string",
                    "description": "Start date/time in ISO 8601 format (e.g. 2025-06-15T10:00:00 or 2025-06-15 for all-day)"
                },
                "end": {
                    "type": "string",
                    "description": "End date/time in ISO 8601 format"
                },
                "all_day": {
                    "type": "boolean",
                    "description": "Whether this is an all-day event"
                },
                "location": {
                    "type": "string",
                    "description": "Event location"
                },
                "description": {
                    "type": "string",
                    "description": "Event description/notes"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let operation = params
            .get("operation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing 'operation' parameter"))?;

        let account = params.get("account").and_then(|v| v.as_str());

        match operation {
            "list_calendars" => {
                let client = self.resolve_client(account).await?;
                let calendars = client.list_calendars().await?;
                Ok(json!({ "calendars": calendars }))
            },

            "list_events" => {
                let calendar = params
                    .get("calendar")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("'list_events' requires 'calendar' parameter"))?;

                let range = match (
                    params.get("start").and_then(|v| v.as_str()),
                    params.get("end").and_then(|v| v.as_str()),
                ) {
                    (Some(start), Some(end)) => Some(TimeRange {
                        start: start.to_string(),
                        end: end.to_string(),
                    }),
                    _ => None,
                };

                let client = self.resolve_client(account).await?;
                let events = client.list_events(calendar, range).await?;
                Ok(json!({ "events": events }))
            },

            "create_event" => {
                let calendar = params
                    .get("calendar")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("'create_event' requires 'calendar' parameter"))?;

                let summary = params
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("'create_event' requires 'summary' parameter"))?;

                let start = params
                    .get("start")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("'create_event' requires 'start' parameter"))?;

                let event = NewEvent {
                    summary: summary.to_string(),
                    start: start.to_string(),
                    end: params.get("end").and_then(|v| v.as_str()).map(String::from),
                    all_day: params
                        .get("all_day")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    location: params
                        .get("location")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    description: params
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                };

                let client = self.resolve_client(account).await?;
                let created = client.create_event(calendar, event).await?;
                Ok(json!(created))
            },

            "update_event" => {
                let event_href = params
                    .get("event_href")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("'update_event' requires 'event_href' parameter"))?;

                let etag = params
                    .get("etag")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("'update_event' requires 'etag' parameter"))?;

                let updates = UpdateEvent {
                    summary: params
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    start: params
                        .get("start")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    end: params.get("end").and_then(|v| v.as_str()).map(String::from),
                    all_day: params.get("all_day").and_then(|v| v.as_bool()),
                    location: params
                        .get("location")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    description: params
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                };

                let client = self.resolve_client(account).await?;
                let updated = client.update_event(event_href, etag, updates).await?;
                Ok(json!(updated))
            },

            "delete_event" => {
                let event_href = params
                    .get("event_href")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("'delete_event' requires 'event_href' parameter"))?;

                let etag = params
                    .get("etag")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("'delete_event' requires 'etag' parameter"))?;

                let client = self.resolve_client(account).await?;
                client.delete_event(event_href, etag).await?;
                Ok(json!({ "ok": true }))
            },

            _ => anyhow::bail!("unknown operation: {operation}"),
        }
    }
}

// ── Mock client for testing ─────────────────────────────────────────────────

/// Mock CalDAV client for unit tests.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) struct MockCalDavClient {
    pub calendars: Vec<crate::types::CalendarInfo>,
    pub events: std::sync::Mutex<Vec<crate::types::EventSummary>>,
}

#[cfg(test)]
#[async_trait]
impl crate::client::CalDavClient for MockCalDavClient {
    async fn list_calendars(&self) -> Result<Vec<crate::types::CalendarInfo>> {
        Ok(self.calendars.clone())
    }

    async fn list_events(
        &self,
        _calendar_href: &str,
        _range: Option<TimeRange>,
    ) -> Result<Vec<crate::types::EventSummary>> {
        let events = self
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        Ok(events)
    }

    async fn create_event(
        &self,
        _calendar_href: &str,
        _event: NewEvent,
    ) -> Result<crate::types::CreatedEvent> {
        let uid = format!("mock-{}@moltis", uuid::Uuid::new_v4());
        Ok(crate::types::CreatedEvent {
            href: format!("/cal/{uid}.ics"),
            etag: Some("\"mock-etag\"".to_string()),
            uid,
        })
    }

    async fn update_event(
        &self,
        href: &str,
        _etag: &str,
        _updates: UpdateEvent,
    ) -> Result<crate::types::UpdatedEvent> {
        Ok(crate::types::UpdatedEvent {
            href: href.to_string(),
            etag: Some("\"mock-etag-updated\"".to_string()),
        })
    }

    async fn delete_event(&self, _href: &str, _etag: &str) -> Result<()> {
        Ok(())
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        moltis_config::{CalDavAccountConfig, CalDavConfig},
        secrecy::Secret,
    };

    fn test_config() -> CalDavConfig {
        let mut accounts = HashMap::new();
        accounts.insert("test".to_string(), CalDavAccountConfig {
            url: Some("https://caldav.example.com".to_string()),
            username: Some("user".to_string()),
            password: Some(Secret::new("pass".to_string())),
            provider: Some("generic".to_string()),
            ..Default::default()
        });
        CalDavConfig {
            enabled: true,
            default_account: Some("test".to_string()),
            accounts,
        }
    }

    #[test]
    fn from_config_returns_none_when_disabled() {
        let config = CalDavConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(CalDavTool::from_config(&config).is_none());
    }

    #[test]
    fn from_config_returns_none_when_no_accounts() {
        let config = CalDavConfig {
            enabled: true,
            accounts: HashMap::new(),
            ..Default::default()
        };
        assert!(CalDavTool::from_config(&config).is_none());
    }

    #[test]
    fn from_config_returns_some_when_valid() {
        let config = test_config();
        assert!(CalDavTool::from_config(&config).is_some());
    }

    #[test]
    fn tool_name_is_caldav() {
        let config = test_config();
        let tool = CalDavTool::from_config(&config).unwrap();
        assert_eq!(tool.name(), "caldav");
    }

    #[test]
    fn parameters_schema_has_operation() {
        let config = test_config();
        let tool = CalDavTool::from_config(&config).unwrap();
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "operation"));
    }

    #[tokio::test]
    async fn execute_missing_operation_errors() {
        let config = test_config();
        let tool = CalDavTool::from_config(&config).unwrap();
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("operation"));
    }

    #[tokio::test]
    async fn execute_unknown_operation_errors() {
        let config = test_config();
        let tool = CalDavTool::from_config(&config).unwrap();
        let result = tool.execute(json!({"operation": "nope"})).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unknown operation")
        );
    }

    #[tokio::test]
    async fn create_event_missing_params_errors() {
        let config = test_config();
        let tool = CalDavTool::from_config(&config).unwrap();

        // Missing calendar
        let result = tool
            .execute(json!({
                "operation": "create_event",
                "summary": "Test",
                "start": "2025-01-01T10:00:00"
            }))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("calendar"));
    }

    #[tokio::test]
    async fn delete_event_missing_etag_errors() {
        let config = test_config();
        let tool = CalDavTool::from_config(&config).unwrap();

        let result = tool
            .execute(json!({
                "operation": "delete_event",
                "event_href": "/cal/test.ics"
            }))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("etag"));
    }
}
