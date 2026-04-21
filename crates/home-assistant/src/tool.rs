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
