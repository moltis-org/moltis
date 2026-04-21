//! AgentTool implementation for Home Assistant operations.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use moltis_agents::tool_registry::AgentTool;
use serde_json::{Value, json};

use crate::client::HomeAssistantClient;
use crate::config::HomeAssistantConfig;
use crate::types::{EntityState, Target};

/// Shared client handle behind an Arc.
type SharedClient = Arc<HomeAssistantClient>;

/// Home Assistant agent tool providing entity control and state queries.
///
/// Connections to HA instances are lazily initialised on first use.
pub struct HomeAssistantTool {
    config: HomeAssistantConfig,
    clients: tokio::sync::RwLock<HashMap<String, SharedClient>>,
}

impl HomeAssistantTool {
    /// Create the tool from config, returning `None` if HA is disabled
    /// or no instances are configured.
    #[must_use]
    pub fn from_config(config: &HomeAssistantConfig) -> Option<Self> {
        if !config.enabled || config.instances.is_empty() {
            return None;
        }
        Some(Self {
            config: config.clone(),
            clients: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Resolve which instance to use and return its client.
    async fn resolve_client(
        &self,
        instance: Option<&str>,
    ) -> crate::error::Result<SharedClient> {
        let (name, account_config) = crate::config::resolve_instance(&self.config, instance)?;

        // Check cache
        {
            let clients = self.clients.read().await;
            if let Some(client) = clients.get(name) {
                return Ok(Arc::clone(client));
            }
        }

        // Build new client
        let client = HomeAssistantClient::new(account_config)?;
        let shared: SharedClient = Arc::new(client);

        let mut clients = self.clients.write().await;
        clients.insert(name.to_owned(), Arc::clone(&shared));

        Ok(shared)
    }

    /// Extract entities matching a filter from a state list.
    fn filter_entities(
        states: &[EntityState],
        domain: Option<&str>,
        area_id: Option<&str>,
    ) -> Vec<Value> {
        states
            .iter()
            .filter(|s| {
                if let Some(d) = domain {
                    if s.domain() != d {
                        return false;
                    }
                }
                if let Some(a) = area_id {
                    if s.area_id() != Some(a) {
                        return false;
                    }
                }
                true
            })
            .map(|s| {
                json!({
                    "entity_id": s.entity_id,
                    "state": s.state,
                    "friendly_name": s.friendly_name(),
                    "domain": s.domain(),
                    "area_id": s.area_id(),
                    "last_changed": s.last_changed,
                })
            })
            .collect()
    }
}

#[async_trait]
impl AgentTool for HomeAssistantTool {
    fn name(&self) -> &str {
        "home_assistant"
    }

    fn description(&self) -> &str {
        "Control Home Assistant entities and query their state. \
         Supports multiple named instances.\n\n\
         Operations:\n\
         - list_entities: List entities. Optional params: domain, area_id.\n\
         - get_state: Get state of a specific entity. Params: entity_id (required).\n\
         - turn_on: Turn an entity on. Params: entity_id (required).\n\
         - turn_off: Turn an entity off. Params: entity_id (required).\n\
         - toggle: Toggle an entity. Params: entity_id (required).\n\
         - call_service: Call any HA service. Params: domain, service (required), \
           data (optional JSON object), area_id (optional).\n\
         - get_config: Get HA instance info (version, location, components).\n\n\
         Pass 'instance' to select a specific HA instance if multiple are configured."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["operation"],
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "list_entities",
                        "get_state",
                        "turn_on",
                        "turn_off",
                        "toggle",
                        "call_service",
                        "get_config",
                    ],
                    "description": "The operation to perform"
                },
                "instance": {
                    "type": "string",
                    "description": "HA instance name (optional if only one configured)"
                },
                "entity_id": {
                    "type": "string",
                    "description": "Entity ID (e.g. 'light.living_room')"
                },
                "domain": {
                    "type": "string",
                    "description": "Filter entities by domain (e.g. 'light', 'switch', 'sensor')"
                },
                "area_id": {
                    "type": "string",
                    "description": "Filter entities by area ID"
                },
                "service_domain": {
                    "type": "string",
                    "description": "Service domain for call_service (e.g. 'light')"
                },
                "service": {
                    "type": "string",
                    "description": "Service name for call_service (e.g. 'turn_on')"
                },
                "data": {
                    "type": "object",
                    "description": "Service data for call_service"
                },
            }
        })
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(operation)))]
    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let operation = params
            .get("operation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'operation' parameter"))?;

        let instance = params.get("instance").and_then(|v| v.as_str());
        let client = self
            .resolve_client(instance)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        match operation {
            "list_entities" => {
                let domain = params.get("domain").and_then(|v| v.as_str());
                let area_id = params.get("area_id").and_then(|v| v.as_str());

                // TODO: use POST /api/states with filter body for large instances
                let states = client
                    .get_states()
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                let filtered = Self::filter_entities(&states, domain, area_id);
                Ok(json!({
                    "count": filtered.len(),
                    "entities": filtered,
                }))
            }

            "get_state" => {
                let entity_id = params
                    .get("entity_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'entity_id' parameter"))?;

                match client.get_state(entity_id).await {
                    Ok(Some(state)) => Ok(json!({
                        "entity_id": state.entity_id,
                        "state": state.state,
                        "attributes": state.attributes,
                        "friendly_name": state.friendly_name(),
                        "last_changed": state.last_changed,
                    })),
                    Ok(None) => Ok(json!({
                        "entity_id": entity_id,
                        "found": false,
                    })),
                    Err(e) => Err(anyhow::anyhow!("{e}")),
                }
            }

            "turn_on" => {
                let entity_id = params
                    .get("entity_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'entity_id' parameter"))?;

                client
                    .turn_on(entity_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                Ok(json!({ "entity_id": entity_id, "action": "turn_on", "status": "ok" }))
            }

            "turn_off" => {
                let entity_id = params
                    .get("entity_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'entity_id' parameter"))?;

                client
                    .turn_off(entity_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                Ok(json!({ "entity_id": entity_id, "action": "turn_off", "status": "ok" }))
            }

            "toggle" => {
                let entity_id = params
                    .get("entity_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'entity_id' parameter"))?;

                client
                    .toggle(entity_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                Ok(json!({ "entity_id": entity_id, "action": "toggle", "status": "ok" }))
            }

            "call_service" => {
                let domain = params
                    .get("service_domain")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'service_domain' parameter"))?;

                let service = params
                    .get("service")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'service' parameter"))?;

                let area_id = params.get("area_id").and_then(|v| v.as_str());
                let target = area_id.map(|a| Target::area(a));

                let result = client
                    .call_service(domain, service, target.as_ref(), params.get("data").cloned())
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                Ok(result)
            }

            "get_config" => {
                let config = client
                    .get_config()
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                Ok(json!({
                    "version": config.version,
                    "location_name": config.location_name,
                    "latitude": config.latitude,
                    "longitude": config.longitude,
                    "elevation": config.elevation,
                    "time_zone": config.time_zone,
                    "components": config.components,
                    // config_dir omitted — server filesystem path, not useful to LLM
                }))
            }

            other => Err(anyhow::anyhow!("unknown operation: '{other}'")),
        }
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        // Pre-build clients for all configured instances.
        // Construct outside the lock to avoid holding it during I/O.
        let built: Vec<(String, SharedClient)> = self
            .config
            .instances
            .iter()
            .filter(|(_, a)| a.url.is_some() && a.token.is_some())
            .filter_map(|(name, account)| {
                match HomeAssistantClient::new(account) {
                    Ok(client) => {
                        #[cfg(feature = "tracing")]
                        tracing::info!(instance = %name, "HA client pre-connected");
                        Some((name.clone(), Arc::new(client)))
                    }
                    Err(e) => {
                        #[cfg(feature = "tracing")]
                        tracing::warn!(instance = %name, error = %e, "HA client warmup failed");
                        None
                    }
                }
            })
            .collect();

        let mut clients = self.clients.write().await;
        for (name, client) in built {
            clients.insert(name, client);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::EntityState;
    use moltis_config::HomeAssistantAccountConfig;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    fn make_tool(server: &MockServer) -> HomeAssistantTool {
        let mut config = HomeAssistantConfig::default();
        config.enabled = true;
        config.instances.insert(
            "home".to_owned(),
            HomeAssistantAccountConfig {
                url: Some(server.uri()),
                token: Some(secrecy::Secret::new("test-token".to_owned())),
                timeout_seconds: 10,
            },
        );
        HomeAssistantTool::from_config(&config).unwrap()
    }

    fn state_list_json() -> Value {
        json!([
            {
                "entity_id": "light.living_room",
                "state": "on",
                "attributes": {"friendly_name": "Living Room", "area_id": "living"},
                "last_changed": "2026-01-01T00:00:00+00:00",
                "last_updated": "2026-01-01T00:00:00+00:00",
                "context": {"id": "a", "parent_id": null, "user_id": null}
            },
            {
                "entity_id": "light.bedroom",
                "state": "off",
                "attributes": {"friendly_name": "Bedroom", "area_id": "bedroom"},
                "last_changed": "2026-01-01T00:00:00+00:00",
                "last_updated": "2026-01-01T00:00:00+00:00",
                "context": {"id": "b", "parent_id": null, "user_id": null}
            },
            {
                "entity_id": "switch.kitchen",
                "state": "on",
                "attributes": {"friendly_name": "Kitchen Fan", "area_id": "kitchen"},
                "last_changed": "2026-01-01T00:00:00+00:00",
                "last_updated": "2026-01-01T00:00:00+00:00",
                "context": {"id": "c", "parent_id": null, "user_id": null}
            },
            {
                "entity_id": "sensor.temperature",
                "state": "22.5",
                "attributes": {"friendly_name": "Temp", "unit_of_measurement": "°C"},
                "last_changed": "2026-01-01T00:00:00+00:00",
                "last_updated": "2026-01-01T00:00:00+00:00",
                "context": {"id": "d", "parent_id": null, "user_id": null}
            }
        ])
    }

    fn config_json() -> Value {
        json!({
            "version": "2025.1.0",
            "unit_system": "metric",
            "location_name": "Home",
            "latitude": 45.0,
            "longitude": -63.0,
            "elevation": 30.0,
            "time_zone": "America/Halifax",
            "components": ["light", "switch", "sensor"],
            "config_dir": "/config"
        })
    }

    // --- from_config ---

    #[test]
    fn from_config_returns_none_when_disabled() {
        let config = HomeAssistantConfig::default();
        assert!(HomeAssistantTool::from_config(&config).is_none());
    }

    #[test]
    fn from_config_returns_none_when_empty_instances() {
        let mut config = HomeAssistantConfig::default();
        config.enabled = true;
        assert!(HomeAssistantTool::from_config(&config).is_none());
    }

    // --- filter_entities ---

    #[test]
    fn filter_entities_no_filter() {
        let states: Vec<EntityState> =
            serde_json::from_value(state_list_json()).unwrap();
        let result = HomeAssistantTool::filter_entities(&states, None, None);
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn filter_entities_by_domain() {
        let states: Vec<EntityState> =
            serde_json::from_value(state_list_json()).unwrap();
        let result = HomeAssistantTool::filter_entities(&states, Some("light"), None);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_entities_by_area() {
        let states: Vec<EntityState> =
            serde_json::from_value(state_list_json()).unwrap();
        let result = HomeAssistantTool::filter_entities(&states, None, Some("living"));
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn filter_entities_by_domain_and_area() {
        let states: Vec<EntityState> =
            serde_json::from_value(state_list_json()).unwrap();
        let result = HomeAssistantTool::filter_entities(&states, Some("light"), Some("bedroom"));
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn filter_entities_no_match() {
        let states: Vec<EntityState> =
            serde_json::from_value(state_list_json()).unwrap();
        let result = HomeAssistantTool::filter_entities(&states, Some("climate"), None);
        assert!(result.is_empty());
    }

    // --- tool metadata ---

    #[tokio::test]
    async fn tool_name() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        assert_eq!(tool.name(), "home_assistant");
    }

    #[tokio::test]
    async fn tool_description_contains_operations() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let desc = tool.description();
        assert!(desc.contains("list_entities"));
        assert!(desc.contains("turn_on"));
        assert!(desc.contains("call_service"));
    }

    #[tokio::test]
    async fn tool_schema_has_required_operation() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let schema = tool.parameters_schema();
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.contains(&json!("operation")));
    }

    #[tokio::test]
    async fn tool_schema_has_all_operations() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let schema = tool.parameters_schema();
        let ops = schema
            .pointer("/properties/operation/enum")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(ops.contains(&json!("list_entities")));
        assert!(ops.contains(&json!("get_state")));
        assert!(ops.contains(&json!("turn_on")));
        assert!(ops.contains(&json!("turn_off")));
        assert!(ops.contains(&json!("toggle")));
        assert!(ops.contains(&json!("call_service")));
        assert!(ops.contains(&json!("get_config")));
    }

    // --- execute: list_entities ---

    #[tokio::test]
    async fn execute_list_entities() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states"))
            .respond_with(ResponseTemplate::new(200).set_body_json(state_list_json()))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({"operation": "list_entities"}))
            .await
            .unwrap();
        assert_eq!(result["count"], 4);
        assert_eq!(result["entities"].as_array().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn execute_list_entities_filtered_by_domain() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states"))
            .respond_with(ResponseTemplate::new(200).set_body_json(state_list_json()))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({"operation": "list_entities", "domain": "light"}))
            .await
            .unwrap();
        assert_eq!(result["count"], 2);
    }

    // --- execute: get_state ---

    #[tokio::test]
    async fn execute_get_state_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states/light.living_room"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entity_id": "light.living_room",
                "state": "on",
                "attributes": {"friendly_name": "Living Room"},
                "last_changed": "2026-01-01T00:00:00+00:00",
                "last_updated": "2026-01-01T00:00:00+00:00",
                "context": {"id": "a", "parent_id": null, "user_id": null}
            })))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({"operation": "get_state", "entity_id": "light.living_room"}))
            .await
            .unwrap();
        assert_eq!(result["entity_id"], "light.living_room");
        assert_eq!(result["state"], "on");
    }

    #[tokio::test]
    async fn execute_get_state_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states/light.nonexistent"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({"operation": "get_state", "entity_id": "light.nonexistent"}))
            .await
            .unwrap();
        assert_eq!(result["found"], false);
    }

    // --- execute: turn_on / turn_off / toggle ---

    #[tokio::test]
    async fn execute_turn_on() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/light/turn_on"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({"operation": "turn_on", "entity_id": "light.living_room"}))
            .await
            .unwrap();
        assert_eq!(result["status"], "ok");
        assert_eq!(result["entity_id"], "light.living_room");
    }

    #[tokio::test]
    async fn execute_turn_off() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/light/turn_off"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({"operation": "turn_off", "entity_id": "light.bedroom"}))
            .await
            .unwrap();
        assert_eq!(result["status"], "ok");
    }

    #[tokio::test]
    async fn execute_toggle() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/switch/toggle"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({"operation": "toggle", "entity_id": "switch.kitchen"}))
            .await
            .unwrap();
        assert_eq!(result["action"], "toggle");
        assert_eq!(result["status"], "ok");
    }

    // --- execute: call_service ---

    #[tokio::test]
    async fn execute_call_service() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/homeassistant/turn_off"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({
                "operation": "call_service",
                "service_domain": "homeassistant",
                "service": "turn_off"
            }))
            .await
            .unwrap();
        assert_eq!(result, json!([]));
    }

    #[tokio::test]
    async fn execute_call_service_with_area_target() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/light/turn_on"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        tool.execute(json!({
            "operation": "call_service",
            "service_domain": "light",
            "service": "turn_on",
            "area_id": "living"
        }))
        .await
        .unwrap();
    }

    // --- execute: get_config ---

    #[tokio::test]
    async fn execute_get_config() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/config"))
            .respond_with(ResponseTemplate::new(200).set_body_json(config_json()))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({"operation": "get_config"}))
            .await
            .unwrap();
        assert_eq!(result["version"], "2025.1.0");
        assert_eq!(result["location_name"], "Home");
        // config_dir must be redacted
        assert!(result.get("config_dir").is_none());
    }

    // --- execute: error cases ---

    #[tokio::test]
    async fn execute_unknown_operation() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let err = tool
            .execute(json!({"operation": "destroy_house"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown operation"));
    }

    #[tokio::test]
    async fn execute_missing_operation() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("missing 'operation'"));
    }

    #[tokio::test]
    async fn execute_turn_on_missing_entity_id() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let err = tool
            .execute(json!({"operation": "turn_on"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'entity_id'"));
    }

    #[tokio::test]
    async fn execute_get_state_missing_entity_id() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let err = tool
            .execute(json!({"operation": "get_state"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'entity_id'"));
    }

    #[tokio::test]
    async fn execute_call_service_missing_domain() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let err = tool
            .execute(json!({"operation": "call_service", "service": "turn_on"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'service_domain'"));
    }

    #[tokio::test]
    async fn execute_call_service_missing_service() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let err = tool
            .execute(json!({"operation": "call_service", "service_domain": "light"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'service'"));
    }

    // --- warmup ---

    #[tokio::test]
    async fn warmup_preconnects_clients() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        tool.warmup().await.unwrap();
        // After warmup, the client should be cached (no HTTP call needed)
        // The next call should work without hitting the mock server again
        // (we just verify warmup itself doesn't fail)
    }

    // --- instance resolution ---

    #[tokio::test]
    async fn execute_with_unknown_instance_errors() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let err = tool
            .execute(json!({"operation": "get_config", "instance": "nonexistent"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no HA instance"));
    }
}
