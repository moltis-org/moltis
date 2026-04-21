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
        self.entity_id
            .split('.')
            .next()
            .unwrap_or("unknown")
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

/// Area registry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Area {
    pub area_id: String,
    pub name: String,
    #[serde(default)]
    pub picture: Option<String>,
}

/// Device registry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub area_id: Option<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
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
    Trigger {
        variables: serde_json::Value,
    },
    /// Unstructured message from the server.
    Raw(serde_json::Value),
    /// WebSocket disconnected.
    Disconnected,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
