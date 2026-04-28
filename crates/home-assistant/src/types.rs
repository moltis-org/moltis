//! Home Assistant entity and API types.

use serde::{Deserialize, Serialize};

/// An HA entity state as returned by `GET /api/states`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityState {
    pub entity_id: String,
    pub state: String,
    pub attributes: serde_json::Value,
    pub last_changed: String,
    pub last_updated: String,
    pub context: serde_json::Value,
}

impl EntityState {
    /// Extract the `friendly_name` attribute if present.
    #[must_use]
    pub fn friendly_name(&self) -> Option<&str> {
        self.attributes
            .get("friendly_name")
            .and_then(|v| v.as_str())
    }

    /// Extract the `area_id` attribute if present.
    #[must_use]
    pub fn area_id(&self) -> Option<&str> {
        self.attributes.get("area_id").and_then(|v| v.as_str())
    }

    /// Extract the `device_id` attribute if present.
    #[must_use]
    pub fn device_id(&self) -> Option<&str> {
        self.attributes.get("device_id").and_then(|v| v.as_str())
    }

    /// Get the entity domain (part before the first `.`).
    #[must_use]
    pub fn domain(&self) -> &str {
        self.entity_id.split('.').next().unwrap_or("unknown")
    }
}

/// HA instance configuration as returned by `GET /api/config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaConfigResponse {
    pub version: String,
    pub unit_system: serde_json::Value,
    pub location_name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub elevation: f64,
    pub time_zone: String,
    pub components: Vec<String>,
    pub config_dir: String,
}

/// Service description as returned by `GET /api/services`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDescription {
    pub domain: String,
    pub services: serde_json::Value,
}

/// Service call target for HA's `target` parameter.
///
/// Used to address entities by area, device, or label instead of
/// listing individual entity IDs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Target {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub entity_id: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub device_id: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub area_id: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub label_id: Vec<String>,
}

impl Target {
    /// Create a target for a single entity.
    #[must_use]
    pub fn entity(entity_id: &str) -> Self {
        Self {
            entity_id: vec![entity_id.to_owned()],
            ..Self::default()
        }
    }

    /// Create a target for a single area.
    #[must_use]
    pub fn area(area_id: &str) -> Self {
        Self {
            area_id: vec![area_id.to_owned()],
            ..Self::default()
        }
    }

    /// Create a target for a single device.
    #[must_use]
    pub fn device(device_id: &str) -> Self {
        Self {
            device_id: vec![device_id.to_owned()],
            ..Self::default()
        }
    }
}

/// Incoming HA WebSocket event.
#[derive(Debug)]
pub enum HaEvent {
    /// An entity state changed.
    StateChanged {
        entity_id: String,
        old_state: Option<serde_json::Value>,
        new_state: Option<serde_json::Value>,
    },
    /// A trigger matched (automation-style).
    Trigger { variables: serde_json::Value },
    /// Unstructured message from the server.
    Raw(serde_json::Value),
    /// WebSocket disconnected.
    Disconnected,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {super::*, serde_json::json};

    #[test]
    fn entity_domain_extraction() {
        let state = EntityState {
            entity_id: "light.living_room".to_owned(),
            state: "on".to_owned(),
            attributes: serde_json::json!({
                "friendly_name": "Living Room",
                "area_id": "kitchen"
            }),
            last_changed: String::new(),
            last_updated: String::new(),
            context: serde_json::Value::Null,
        };

        assert_eq!(state.domain(), "light");
        assert_eq!(state.friendly_name(), Some("Living Room"));
        assert_eq!(state.area_id(), Some("kitchen"));
        assert!(state.device_id().is_none());
    }

    #[test]
    fn target_entity_builder() {
        let t = Target::entity("light.desk");
        assert_eq!(t.entity_id, vec!["light.desk"]);
        assert!(t.area_id.is_empty());
    }

    #[test]
    fn target_serialization_skips_empty() {
        let t = Target::entity("switch.kitchen");
        let json = serde_json::to_value(&t).unwrap();
        assert!(json.get("entity_id").is_some());
        assert!(json.get("area_id").is_none());
        assert!(json.get("device_id").is_none());
    }

    #[test]
    fn entity_state_deserialization() {
        let raw = r#"{
            "entity_id": "sensor.temperature",
            "state": "22.5",
            "attributes": {"friendly_name": "Temp", "unit_of_measurement": "°C"},
            "last_changed": "2026-01-01T00:00:00+00:00",
            "last_updated": "2026-01-01T00:00:00+00:00",
            "context": {"id": "abc", "parent_id": null, "user_id": null}
        }"#;
        let state: EntityState = serde_json::from_str(raw).unwrap();
        assert_eq!(state.entity_id, "sensor.temperature");
        assert_eq!(state.state, "22.5");
    }

    #[test]
    fn entity_state_deserialization_minimal() {
        // HA may return entities with minimal fields
        let raw = r#"{
            "entity_id": "binary_sensor.door",
            "state": "off",
            "attributes": {},
            "last_changed": "2026-01-01T00:00:00+00:00",
            "last_updated": "2026-01-01T00:00:00+00:00",
            "context": {"id": "x", "parent_id": null, "user_id": null}
        }"#;
        let state: EntityState = serde_json::from_str(raw).unwrap();
        assert_eq!(state.entity_id, "binary_sensor.door");
        assert!(state.friendly_name().is_none());
        assert!(state.area_id().is_none());
    }

    #[test]
    fn entity_state_with_unavailable() {
        let raw = r#"{
            "entity_id": "sensor.broken",
            "state": "unavailable",
            "attributes": {"friendly_name": "Broken Sensor"},
            "last_changed": "2026-01-01T00:00:00+00:00",
            "last_updated": "2026-01-01T00:00:00+00:00",
            "context": {}
        }"#;
        let state: EntityState = serde_json::from_str(raw).unwrap();
        assert_eq!(state.state, "unavailable");
    }

    #[test]
    fn target_area_builder() {
        let t = Target::area("kitchen");
        assert_eq!(t.area_id, vec!["kitchen"]);
        assert!(t.entity_id.is_empty());
        assert!(t.device_id.is_empty());
    }

    #[test]
    fn target_device_builder() {
        let t = Target::device("abc123");
        assert_eq!(t.device_id, vec!["abc123"]);
        assert!(t.entity_id.is_empty());
    }

    #[test]
    fn target_multi_entity() {
        let t = Target {
            entity_id: vec!["light.1".into(), "light.2".into()],
            ..Target::default()
        };
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["entity_id"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn target_default_is_empty() {
        let t = Target::default();
        let json = serde_json::to_value(&t).unwrap();
        // All empty vecs should be skipped
        assert_eq!(json.as_object().unwrap().len(), 0);
    }

    #[test]
    fn entity_state_serialization_roundtrip() {
        let state = EntityState {
            entity_id: "switch.test".to_owned(),
            state: "on".to_owned(),
            attributes: json!({"friendly_name": "Test"}),
            last_changed: "2026-01-01T00:00:00+00:00".to_owned(),
            last_updated: "2026-01-01T00:00:00+00:00".to_owned(),
            context: json!({"id": "abc"}),
        };
        let serialized = serde_json::to_value(&state).unwrap();
        let deserialized: EntityState = serde_json::from_value(serialized).unwrap();
        assert_eq!(deserialized.entity_id, "switch.test");
        assert_eq!(deserialized.state, "on");
    }

    #[test]
    fn ha_config_response_deserialization() {
        let raw = r#"{
            "version": "2025.2.0",
            "unit_system": {"length": "km", "mass": "kg"},
            "location_name": "Cottage",
            "latitude": 44.5,
            "longitude": -64.0,
            "elevation": 10.0,
            "time_zone": "America/Halifax",
            "components": ["light", "sensor"],
            "config_dir": "/config"
        }"#;
        let config: HaConfigResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(config.version, "2025.2.0");
        assert_eq!(config.location_name, "Cottage");
        assert_eq!(config.components, vec!["light", "sensor"]);
    }

    #[test]
    fn service_description_deserialization() {
        let raw = r#"{
            "domain": "climate",
            "services": {
                "set_temperature": {
                    "name": "Set temperature",
                    "target": {"entity_id": {}},
                    "fields": {"temperature": {"name": "Temperature"}}
                }
            }
        }"#;
        let svc: ServiceDescription = serde_json::from_str(raw).unwrap();
        assert_eq!(svc.domain, "climate");
        assert!(svc.services["set_temperature"]["name"].is_string());
    }
}
