//! AgentTool implementation for Home Assistant operations.

use std::{collections::HashMap, sync::Arc};

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
};

use crate::{
    client::HomeAssistantClient,
    config::HomeAssistantConfig,
    types::{EntityState, Target},
};

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
    async fn resolve_client(&self, instance: Option<&str>) -> crate::error::Result<SharedClient> {
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
    ///
    /// Supports domain, area_id, and entity_id prefix filters.
    /// Multiple filters are AND-combined.
    fn filter_entities(
        states: &[EntityState],
        domain: Option<&str>,
        area_id: Option<&str>,
        entity_id_prefix: Option<&str>,
    ) -> Vec<Value> {
        states
            .iter()
            .filter(|s| {
                if let Some(d) = domain
                    && s.domain() != d
                {
                    return false;
                }
                if let Some(a) = area_id
                    && s.area_id() != Some(a)
                {
                    return false;
                }
                if let Some(prefix) = entity_id_prefix
                    && !s.entity_id.starts_with(prefix)
                {
                    return false;
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
         - call_service: Call any HA service. Params: service_domain, service (required), \
           data (optional JSON object), area_id (optional).\n\
         - get_config: Get HA instance info (version, location, components).\n\
         - get_services: List all available services.\n\
         - get_history: Get state history for an entity. Params: entity_id (required), \
           start_time (required ISO 8601), end_time (optional ISO 8601).\n\
         - fire_event: Fire a custom event. Params: event_type (required), \
           data (optional JSON object).\n\
         - health_check: Verify HA instance is reachable and token is valid.\n\n\
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
                        "get_services",
                        "get_history",
                        "fire_event",
                        "health_check",
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
                "entity_id_prefix": {
                    "type": "string",
                    "description": "Filter entities by entity_id prefix (e.g. 'light.' for all lights)"
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
                    "description": "JSON object data (for call_service or fire_event)"
                },
                "event_type": {
                    "type": "string",
                    "description": "Event type for fire_event (e.g. 'my_custom_event')"
                },
                "start_time": {
                    "type": "string",
                    "description": "ISO 8601 start time for get_history"
                },
                "end_time": {
                    "type": "string",
                    "description": "ISO 8601 end time for get_history"
                },
            }
        })
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip_all, level = "debug", fields(operation))
    )]
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
                let entity_id_prefix = params.get("entity_id_prefix").and_then(|v| v.as_str());

                let states = client
                    .get_states()
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                let filtered = Self::filter_entities(&states, domain, area_id, entity_id_prefix);
                Ok(json!({
                    "count": filtered.len(),
                    "entities": filtered,
                }))
            },

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
            },

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
            },

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
            },

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
            },

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
                let entity_id = params.get("entity_id");
                let mut target = Target::default();
                if let Some(a) = area_id {
                    target.area_id.push(a.to_owned());
                }
                if let Some(e) = entity_id {
                    if let Some(s) = e.as_str() {
                        // comma-separated: "light.bedroom,light.kitchen"
                        for id in s.split(',') {
                            let id = id.trim();
                            if !id.is_empty() {
                                target.entity_id.push(id.to_owned());
                            }
                        }
                    } else if let Some(arr) = e.as_array() {
                        // array form: ["light.bedroom", "light.kitchen"]
                        for id in arr {
                            if let Some(s) = id.as_str() {
                                target.entity_id.push(s.to_owned());
                            }
                        }
                    }
                }
                let target =
                    (!target.entity_id.is_empty() || !target.area_id.is_empty()).then_some(target);

                let result = client
                    .call_service(
                        domain,
                        service,
                        target.as_ref(),
                        params.get("data").cloned(),
                        false,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                Ok(result)
            },

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
                }))
            },

            "get_services" => {
                let services = client
                    .get_services()
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                // Summarise: domain → {service_name: description}
                let mut summary = serde_json::Map::new();
                for svc in &services {
                    let mut domain_services = serde_json::Map::new();
                    if let Some(obj) = svc.services.as_object() {
                        for (name, info) in obj {
                            let desc = info.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            domain_services.insert(name.clone(), json!(desc));
                        }
                    }
                    summary.insert(svc.domain.clone(), Value::Object(domain_services));
                }

                Ok(json!({
                    "count": services.len(),
                    "domains": summary,
                }))
            },

            "get_history" => {
                let entity_id = params
                    .get("entity_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'entity_id' parameter"))?;

                let start_time = params
                    .get("start_time")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'start_time' parameter"))?;

                let end_time = params.get("end_time").and_then(|v| v.as_str());

                let history = client
                    .get_history(entity_id, start_time, end_time)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                // HA returns Vec<Vec<StateChange>> — one inner array per entity.
                let record_count: usize = history
                    .iter()
                    .map(|arr| arr.as_array().map_or(0, Vec::len))
                    .sum();

                Ok(json!({
                    "entity_id": entity_id,
                    "entities": history.len(),
                    "records": record_count,
                    "history": history,
                }))
            },

            "fire_event" => {
                let event_type = params
                    .get("event_type")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'event_type' parameter"))?;

                client
                    .fire_event(event_type, params.get("data").cloned())
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                Ok(json!({
                    "event_type": event_type,
                    "status": "fired",
                }))
            },

            "health_check" => {
                client
                    .health_check()
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;

                Ok(json!({ "status": "ok" }))
            },

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
            .filter_map(|(name, account)| match HomeAssistantClient::new(account) {
                Ok(client) => {
                    #[cfg(feature = "tracing")]
                    tracing::info!(instance = %name, "HA client pre-connected");
                    Some((name.clone(), Arc::new(client)))
                },
                Err(e) => {
                    #[cfg(feature = "tracing")]
                    tracing::warn!(instance = %name, error = %e, "HA client warmup failed");
                    None
                },
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
    use {
        super::*,
        crate::types::EntityState,
        moltis_config::HomeAssistantAccountConfig,
        serde_json::json,
        wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        },
    };

    fn make_tool(server: &MockServer) -> HomeAssistantTool {
        let mut config = HomeAssistantConfig {
            enabled: true,
            ..Default::default()
        };
        config
            .instances
            .insert("home".to_owned(), HomeAssistantAccountConfig {
                url: Some(server.uri()),
                token: Some(secrecy::Secret::new("test-token".to_owned())),
                timeout_seconds: 10,
            });
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
        let config = HomeAssistantConfig {
            enabled: true,
            ..Default::default()
        };
        assert!(HomeAssistantTool::from_config(&config).is_none());
    }

    // --- filter_entities ---

    #[test]
    fn filter_entities_no_filter() {
        let states: Vec<EntityState> = serde_json::from_value(state_list_json()).unwrap();
        let result = HomeAssistantTool::filter_entities(&states, None, None, None);
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn filter_entities_by_domain() {
        let states: Vec<EntityState> = serde_json::from_value(state_list_json()).unwrap();
        let result = HomeAssistantTool::filter_entities(&states, Some("light"), None, None);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_entities_by_area() {
        let states: Vec<EntityState> = serde_json::from_value(state_list_json()).unwrap();
        let result = HomeAssistantTool::filter_entities(&states, None, Some("living"), None);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn filter_entities_by_domain_and_area() {
        let states: Vec<EntityState> = serde_json::from_value(state_list_json()).unwrap();
        let result =
            HomeAssistantTool::filter_entities(&states, Some("light"), Some("bedroom"), None);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn filter_entities_no_match() {
        let states: Vec<EntityState> = serde_json::from_value(state_list_json()).unwrap();
        let result = HomeAssistantTool::filter_entities(&states, Some("climate"), None, None);
        assert!(result.is_empty());
    }

    #[test]
    fn filter_entities_by_entity_id_prefix() {
        let states: Vec<EntityState> = serde_json::from_value(state_list_json()).unwrap();
        let result = HomeAssistantTool::filter_entities(&states, None, None, Some("light."));
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_entities_by_prefix_and_domain() {
        let states: Vec<EntityState> = serde_json::from_value(state_list_json()).unwrap();
        // prefix "sensor." + domain "sensor" — should match 1
        let result =
            HomeAssistantTool::filter_entities(&states, Some("sensor"), None, Some("sensor."));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["entity_id"], "sensor.temperature");
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
        assert!(desc.contains("get_services"));
        assert!(desc.contains("get_history"));
        assert!(desc.contains("fire_event"));
        assert!(desc.contains("health_check"));
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
        assert!(ops.contains(&json!("get_services")));
        assert!(ops.contains(&json!("get_history")));
        assert!(ops.contains(&json!("fire_event")));
        assert!(ops.contains(&json!("health_check")));
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

    // --- execute: get_services ---

    #[tokio::test]
    async fn execute_get_services() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {
                    "domain": "light",
                    "services": {
                        "turn_on": {"name": "Turn On"},
                        "turn_off": {"name": "Turn Off"}
                    }
                },
                {
                    "domain": "climate",
                    "services": {
                        "set_temperature": {"name": "Set Temperature"}
                    }
                }
            ])))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({"operation": "get_services"}))
            .await
            .unwrap();
        assert_eq!(result["count"], 2);
        // Now returns domain → {service_name: description}
        let light_services = result["domains"]["light"].as_object().unwrap();
        assert_eq!(light_services.len(), 2);
        assert_eq!(light_services["turn_on"], "Turn On");
        assert_eq!(light_services["turn_off"], "Turn Off");
        let climate_services = result["domains"]["climate"].as_object().unwrap();
        assert_eq!(climate_services["set_temperature"], "Set Temperature");
    }

    // --- execute: get_history ---

    #[tokio::test]
    async fn execute_get_history() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/api/history/period/2026-04-20T00%3A00%3A00%2B00%3A00",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                [{
                    "entity_id": "sensor.temperature",
                    "state": "20.0",
                    "last_changed": "2026-04-20T00:00:00+00:00"
                }, {
                    "entity_id": "sensor.temperature",
                    "state": "21.5",
                    "last_changed": "2026-04-20T12:00:00+00:00"
                }]
            ])))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({
                "operation": "get_history",
                "entity_id": "sensor.temperature",
                "start_time": "2026-04-20T00:00:00+00:00"
            }))
            .await
            .unwrap();
        assert_eq!(result["entities"], 1);
        assert_eq!(result["records"], 2);
        assert_eq!(result["entity_id"], "sensor.temperature");
    }

    #[tokio::test]
    async fn execute_get_history_missing_entity_id() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let err = tool
            .execute(json!({
                "operation": "get_history",
                "start_time": "2026-04-20T00:00:00+00:00"
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'entity_id'"));
    }

    #[tokio::test]
    async fn execute_get_history_missing_start_time() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let err = tool
            .execute(json!({
                "operation": "get_history",
                "entity_id": "sensor.temperature"
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'start_time'"));
    }

    // --- execute: fire_event ---

    #[tokio::test]
    async fn execute_fire_event() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/events/custom_event"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({
                "operation": "fire_event",
                "event_type": "custom_event",
                "data": {"key": "value"}
            }))
            .await
            .unwrap();
        assert_eq!(result["status"], "fired");
        assert_eq!(result["event_type"], "custom_event");
    }

    #[tokio::test]
    async fn execute_fire_event_missing_event_type() {
        let server = MockServer::start().await;
        let tool = make_tool(&server);
        let err = tool
            .execute(json!({"operation": "fire_event"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'event_type'"));
    }

    // --- execute: health_check ---

    #[tokio::test]
    async fn execute_health_check() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/config"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let result = tool
            .execute(json!({"operation": "health_check"}))
            .await
            .unwrap();
        assert_eq!(result["status"], "ok");
    }

    #[tokio::test]
    async fn execute_health_check_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/config"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let tool = make_tool(&server);
        let err = tool
            .execute(json!({"operation": "health_check"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("500"));
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
