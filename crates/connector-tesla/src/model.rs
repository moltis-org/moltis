use {
    moltis_connectors::{ConnectorItemInput, SourceObservation},
    serde::{Deserialize, Serialize},
    serde_json::{Map, Value},
};

use crate::TeslaVehicleEndpoint;

/// Every Fleet API payload is wrapped in a `response` envelope.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TeslaResponse<T> {
    pub response: T,
}

/// Connectivity state reported by the vehicle list. Fleet API best practice is
/// to read this before requesting vehicle data, because requesting data from a
/// sleeping car wakes it and drains the battery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeslaVehicleOnlineState {
    Online,
    Asleep,
    Offline,
    Waking,
    #[default]
    #[serde(other)]
    Unknown,
}

impl TeslaVehicleOnlineState {
    /// Only an online vehicle can answer a data request without being woken.
    #[must_use]
    pub const fn is_reachable(self) -> bool {
        matches!(self, Self::Online)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Asleep => "asleep",
            Self::Offline => "offline",
            Self::Waking => "waking",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeslaVehicleSummary {
    pub vin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub state: TeslaVehicleOnlineState,
}

/// Raw vehicle list entry. Tesla's field names are snake_case and the numeric
/// `id` is not stable across regions, so only the VIN is carried forward.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TeslaApiVehicle {
    pub vin: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub state: TeslaVehicleOnlineState,
}

impl From<TeslaApiVehicle> for TeslaVehicleSummary {
    fn from(vehicle: TeslaApiVehicle) -> Self {
        Self {
            vin: vehicle.vin,
            display_name: vehicle.display_name,
            state: vehicle.state,
        }
    }
}

/// Fleet API reports charging state in PascalCase, unlike the surrounding
/// snake_case payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TeslaChargingState {
    Charging,
    Complete,
    Disconnected,
    NoPower,
    Starting,
    Stopped,
    #[default]
    #[serde(other)]
    Unknown,
}

/// Charge and battery readings. High-value fields are typed; anything else
/// Fleet API returns for the group is preserved verbatim under `extra` so a
/// schema change upstream does not silently drop data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TeslaChargeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usable_battery_level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_range: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub est_battery_range: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ideal_battery_range: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charging_state: Option<TeslaChargingState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge_limit_soc: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge_energy_added: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charger_power: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charger_voltage: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charger_actual_current: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes_to_full_charge: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_full_charge: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_charger_type: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TeslaClimateState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside_temp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside_temp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_temp_setting: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passenger_temp_setting: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_climate_on: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_preconditioning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_heater: Option<bool>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TeslaDriveState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gps_as_of: Option<i64>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TeslaVehicleStateData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub odometer: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub car_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentry_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_user_present: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tpms_pressure_fl: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tpms_pressure_fr: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tpms_pressure_rl: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tpms_pressure_rr: Option<f64>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TeslaVehicleConfigData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub car_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exterior_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheel_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_badging: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TeslaGuiSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui_distance_units: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui_temperature_units: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui_charge_rate_units: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// A `vehicle_data` response, limited to the groups the dataset requested.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TeslaVehicleData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge_state: Option<TeslaChargeState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub climate_state: Option<TeslaClimateState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_state: Option<TeslaDriveState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vehicle_state: Option<TeslaVehicleStateData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vehicle_config: Option<TeslaVehicleConfigData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui_settings: Option<TeslaGuiSettings>,
}

impl TeslaVehicleData {
    /// Reports which requested groups came back populated, so a caller can tell
    /// a missing OAuth scope from an empty reading.
    #[must_use]
    pub fn present_endpoints(&self) -> Vec<TeslaVehicleEndpoint> {
        let mut present = Vec::new();
        if self.charge_state.is_some() {
            present.push(TeslaVehicleEndpoint::ChargeState);
        }
        if self.climate_state.is_some() {
            present.push(TeslaVehicleEndpoint::ClimateState);
        }
        if self.drive_state.is_some() {
            present.push(TeslaVehicleEndpoint::DriveState);
        }
        if self.vehicle_state.is_some() {
            present.push(TeslaVehicleEndpoint::VehicleState);
        }
        if self.vehicle_config.is_some() {
            present.push(TeslaVehicleEndpoint::VehicleConfig);
        }
        if self.gui_settings.is_some() {
            present.push(TeslaVehicleEndpoint::GuiSettings);
        }
        present
    }
}

/// Stored body of a Tesla connector item. State datasets keep one per vehicle;
/// history datasets keep one per observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeslaVehicleBody {
    pub schema_version: u32,
    pub vin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub online_state: TeslaVehicleOnlineState,
    /// RFC 3339 timestamp of the sync that produced this reading.
    pub observed_at: String,
    pub data: TeslaVehicleData,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TeslaSnapshot {
    pub items: Vec<ConnectorItemInput>,
    pub source_observations: Vec<SourceObservation>,
    pub vehicles: Vec<TeslaVehicleSummary>,
    /// VINs that were not sampled because the vehicle was not online. Reported
    /// rather than woken, so a schedule never drains a parked car's battery.
    pub skipped_unreachable: Vec<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn unknown_vehicle_state_does_not_fail_parsing() {
        let vehicle: TeslaApiVehicle = serde_json::from_str(
            r#"{"vin":"5YJ3E1EAXKF123456","display_name":"Car","state":"hibernating"}"#,
        )
        .unwrap();
        assert_eq!(vehicle.state, TeslaVehicleOnlineState::Unknown);
        assert!(!vehicle.state.is_reachable());
    }

    #[test]
    fn only_online_vehicles_are_reachable() {
        assert!(TeslaVehicleOnlineState::Online.is_reachable());
        for state in [
            TeslaVehicleOnlineState::Asleep,
            TeslaVehicleOnlineState::Offline,
            TeslaVehicleOnlineState::Waking,
            TeslaVehicleOnlineState::Unknown,
        ] {
            assert!(!state.is_reachable(), "{state:?} must not be reachable");
        }
    }

    #[test]
    fn unmodelled_fields_are_preserved_in_extra() {
        let charge: TeslaChargeState = serde_json::from_str(
            r#"{"battery_level":72,"charging_state":"Charging","not_yet_modelled":42}"#,
        )
        .unwrap();
        assert_eq!(charge.battery_level, Some(72));
        assert_eq!(charge.charging_state, Some(TeslaChargingState::Charging));
        assert_eq!(
            charge.extra.get("not_yet_modelled"),
            Some(&Value::from(42_i64))
        );

        let round_trip = serde_json::to_value(&charge).unwrap();
        assert_eq!(round_trip.get("not_yet_modelled"), Some(&Value::from(42)));
        assert!(round_trip.get("extra").is_none());
    }

    #[test]
    fn unknown_charging_state_falls_back_without_error() {
        let charge: TeslaChargeState =
            serde_json::from_str(r#"{"charging_state":"SomethingNew"}"#).unwrap();
        assert_eq!(charge.charging_state, Some(TeslaChargingState::Unknown));
    }

    #[test]
    fn present_endpoints_reflects_returned_groups() {
        let data = TeslaVehicleData {
            charge_state: Some(TeslaChargeState::default()),
            vehicle_state: Some(TeslaVehicleStateData::default()),
            ..TeslaVehicleData::default()
        };
        assert_eq!(data.present_endpoints(), vec![
            TeslaVehicleEndpoint::ChargeState,
            TeslaVehicleEndpoint::VehicleState,
        ]);
        assert!(TeslaVehicleData::default().present_endpoints().is_empty());
    }

    #[test]
    fn response_envelope_unwraps_vehicle_list() {
        let parsed: TeslaResponse<Vec<TeslaApiVehicle>> = serde_json::from_str(
            r#"{"response":[{"vin":"5YJ3E1EAXKF123456","state":"online"}],"count":1}"#,
        )
        .unwrap();
        assert_eq!(parsed.response.len(), 1);
        assert_eq!(parsed.response[0].state, TeslaVehicleOnlineState::Online);
    }
}
