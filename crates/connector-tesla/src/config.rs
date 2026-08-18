use {
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Deserializer, Serialize},
    std::{collections::HashSet, fmt},
};

use crate::{Result, TeslaConnectorError};

const SCHEMA_VERSION: u32 = 1;
const VIN_LENGTH: usize = 17;
const MAX_VEHICLES: usize = 50;
const MAX_HISTORY_SAMPLES: u32 = 20_000;
const MIN_HISTORY_SAMPLES: u32 = 1;

const fn schema_version() -> u32 {
    SCHEMA_VERSION
}

const fn default_max_samples() -> u32 {
    2_000
}

/// Fleet API is partitioned by region and a token issued for one region is not
/// accepted by the others, so the region is part of the account identity rather
/// than a free-form base URL.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeslaRegion {
    #[default]
    NorthAmerica,
    Europe,
    China,
}

impl TeslaRegion {
    #[must_use]
    pub const fn base_url(self) -> &'static str {
        match self {
            Self::NorthAmerica => "https://fleet-api.prd.na.vn.cloud.tesla.com/",
            Self::Europe => "https://fleet-api.prd.eu.vn.cloud.tesla.com/",
            Self::China => "https://fleet-api.prd.cn.vn.cloud.tesla.cn/",
        }
    }

    /// Token exchange is served by a single global host outside China, which
    /// Tesla documents as having higher rate limits than the regional hosts.
    #[must_use]
    pub const fn token_url(self) -> &'static str {
        match self {
            Self::NorthAmerica | Self::Europe => {
                "https://fleet-auth.prd.vn.cloud.tesla.com/oauth2/v3/token"
            },
            Self::China => "https://auth.tesla.cn/oauth2/v3/token",
        }
    }

    /// Where a user completes the authorization-code grant that produces the
    /// refresh token they paste into Moltis.
    #[must_use]
    pub const fn authorize_url(self) -> &'static str {
        match self {
            Self::NorthAmerica | Self::Europe => "https://auth.tesla.com/oauth2/v3/authorize",
            Self::China => "https://auth.tesla.cn/oauth2/v3/authorize",
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NorthAmerica => "north_america",
            Self::Europe => "europe",
            Self::China => "china",
        }
    }
}

impl fmt::Display for TeslaRegion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Read-only account credentials. Moltis never performs the authorization-code
/// exchange itself: that requires a developer application registered against a
/// domain the operator controls, so the refresh token is supplied by the user.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeslaAccountConfig {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub region: TeslaRegion,
    pub client_id: String,
    #[serde(
        deserialize_with = "deserialize_secret",
        serialize_with = "moltis_oauth::serialize_secret"
    )]
    pub refresh_token: Secret<String>,
}

impl fmt::Debug for TeslaAccountConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TeslaAccountConfig")
            .field("schema_version", &self.schema_version)
            .field("region", &self.region)
            .field("client_id", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .finish()
    }
}

impl TeslaAccountConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(TeslaConnectorError::AccountConfig(
                "schemaVersion must be 1",
            ));
        }
        Self::validate_client_id(&self.client_id)?;
        if self.refresh_token.expose_secret().trim().is_empty() {
            return Err(TeslaConnectorError::AccountConfig(
                "refreshToken must not be empty",
            ));
        }
        Ok(())
    }

    /// Validates the non-secret half so the gateway can check an edit request
    /// that keeps the stored refresh token.
    pub fn validate_client_id(client_id: &str) -> Result<()> {
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(TeslaConnectorError::AccountConfig(
                "clientId must not be empty",
            ));
        }
        if client_id.len() > 256 {
            return Err(TeslaConnectorError::AccountConfig(
                "clientId must not exceed 256 characters",
            ));
        }
        Ok(())
    }
}

/// The two shapes a Tesla dataset can take. They differ in how `remote_id` is
/// derived, which is what decides whether a sync replaces the previous rows or
/// appends to them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeslaDatasetMode {
    /// One row per vehicle, replaced on every sync. Answers "what is true now".
    #[default]
    State,
    /// One row per observation, retained up to `maxSamples`. Answers "how has
    /// this changed over time".
    History,
}

/// Vehicle data groups Fleet API can return from a single `vehicle_data` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeslaVehicleEndpoint {
    ChargeState,
    ClimateState,
    DriveState,
    LocationData,
    VehicleState,
    VehicleConfig,
    GuiSettings,
}

impl TeslaVehicleEndpoint {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChargeState => "charge_state",
            Self::ClimateState => "climate_state",
            Self::DriveState => "drive_state",
            Self::LocationData => "location_data",
            Self::VehicleState => "vehicle_state",
            Self::VehicleConfig => "vehicle_config",
            Self::GuiSettings => "gui_settings",
        }
    }

    /// Precise location needs its own OAuth scope, so it is opt-in rather than
    /// part of the default set.
    #[must_use]
    pub const fn requires_location_scope(self) -> bool {
        matches!(self, Self::LocationData)
    }
}

impl fmt::Display for TeslaVehicleEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[must_use]
pub fn default_endpoints() -> Vec<TeslaVehicleEndpoint> {
    vec![
        TeslaVehicleEndpoint::ChargeState,
        TeslaVehicleEndpoint::ClimateState,
        TeslaVehicleEndpoint::DriveState,
        TeslaVehicleEndpoint::VehicleState,
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TeslaDatasetConfig {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    /// Required rather than defaulted: state-versus-history is the choice that
    /// decides what the dataset stores, and it also keeps this config shape
    /// distinguishable from other connectors' dataset configs on the wire.
    pub mode: TeslaDatasetMode,
    /// Empty means every vehicle on the account.
    #[serde(default)]
    pub vins: Vec<String>,
    #[serde(default = "default_endpoints")]
    pub endpoints: Vec<TeslaVehicleEndpoint>,
    /// History retention. Ignored in state mode.
    #[serde(default = "default_max_samples")]
    pub max_samples: u32,
}

impl Default for TeslaDatasetConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            mode: TeslaDatasetMode::default(),
            vins: Vec::new(),
            endpoints: default_endpoints(),
            max_samples: default_max_samples(),
        }
    }
}

impl TeslaDatasetConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(TeslaConnectorError::DatasetConfig(
                "schemaVersion must be 1",
            ));
        }
        if self.endpoints.is_empty() {
            return Err(TeslaConnectorError::DatasetConfig(
                "at least one vehicle data endpoint must be selected",
            ));
        }
        let mut seen = HashSet::with_capacity(self.endpoints.len());
        if self
            .endpoints
            .iter()
            .any(|endpoint| !seen.insert(*endpoint))
        {
            return Err(TeslaConnectorError::DatasetConfig(
                "endpoints must not contain duplicates",
            ));
        }
        if self.vins.len() > MAX_VEHICLES {
            return Err(TeslaConnectorError::DatasetConfig(
                "vins must not exceed 50 entries",
            ));
        }
        let mut seen_vins = HashSet::with_capacity(self.vins.len());
        for vin in &self.vins {
            validate_vin(vin)?;
            if !seen_vins.insert(vin.as_str()) {
                return Err(TeslaConnectorError::DatasetConfig(
                    "vins must not contain duplicates",
                ));
            }
        }
        if !(MIN_HISTORY_SAMPLES..=MAX_HISTORY_SAMPLES).contains(&self.max_samples) {
            return Err(TeslaConnectorError::DatasetConfig(
                "maxSamples must be between 1 and 20000",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn selects_vin(&self, vin: &str) -> bool {
        self.vins.is_empty() || self.vins.iter().any(|selected| selected == vin)
    }

    /// Fleet API expects the requested groups joined by `;`.
    #[must_use]
    pub fn endpoints_parameter(&self) -> String {
        let mut endpoints = self.endpoints.clone();
        endpoints.sort_unstable();
        endpoints
            .iter()
            .map(|endpoint| endpoint.as_str())
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// Tesla VINs are fixed-length and alphanumeric. Rejecting anything else keeps
/// caller-supplied values out of request paths.
pub fn validate_vin(vin: &str) -> Result<()> {
    if vin.len() != VIN_LENGTH {
        return Err(TeslaConnectorError::DatasetConfig(
            "vin must be exactly 17 characters",
        ));
    }
    if !vin.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(TeslaConnectorError::DatasetConfig(
            "vin must be alphanumeric",
        ));
    }
    Ok(())
}

fn deserialize_secret<'de, D>(deserializer: D) -> std::result::Result<Secret<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Secret::new)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn account() -> TeslaAccountConfig {
        TeslaAccountConfig {
            schema_version: SCHEMA_VERSION,
            region: TeslaRegion::Europe,
            client_id: "client-value".to_owned(),
            refresh_token: Secret::new("refresh-value".to_owned()),
        }
    }

    #[test]
    fn account_config_validates_and_redacts_debug() {
        let config = account();
        assert!(config.validate().is_ok());
        let debug = format!("{config:?}");
        assert!(!debug.contains("client-value"));
        assert!(!debug.contains("refresh-value"));

        assert!(
            TeslaAccountConfig {
                client_id: "  ".to_owned(),
                ..account()
            }
            .validate()
            .is_err()
        );
        assert!(
            TeslaAccountConfig {
                refresh_token: Secret::new(String::new()),
                ..account()
            }
            .validate()
            .is_err()
        );
        assert!(
            TeslaAccountConfig {
                schema_version: 2,
                ..account()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn regions_map_to_distinct_hosts() {
        assert!(
            TeslaRegion::NorthAmerica
                .base_url()
                .contains("prd.na.vn.cloud.tesla.com")
        );
        assert!(
            TeslaRegion::Europe
                .base_url()
                .contains("prd.eu.vn.cloud.tesla.com")
        );
        assert!(TeslaRegion::China.base_url().ends_with("tesla.cn/"));
        assert_ne!(
            TeslaRegion::China.token_url(),
            TeslaRegion::Europe.token_url()
        );
    }

    #[test]
    fn dataset_defaults_are_state_mode_without_location() {
        let config: TeslaDatasetConfig = serde_json::from_str(r#"{"mode":"state"}"#).unwrap();
        assert_eq!(config, TeslaDatasetConfig::default());
        assert_eq!(config.mode, TeslaDatasetMode::State);
        assert!(
            !config
                .endpoints
                .iter()
                .any(|endpoint| endpoint.requires_location_scope())
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    fn dataset_validation_rejects_bad_input() {
        let base = TeslaDatasetConfig::default();
        assert!(
            TeslaDatasetConfig {
                endpoints: Vec::new(),
                ..base.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            TeslaDatasetConfig {
                endpoints: vec![
                    TeslaVehicleEndpoint::ChargeState,
                    TeslaVehicleEndpoint::ChargeState
                ],
                ..base.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            TeslaDatasetConfig {
                vins: vec!["TOOSHORT".to_owned()],
                ..base.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            TeslaDatasetConfig {
                vins: vec!["5YJ3E1EA/KF12345".to_owned()],
                ..base.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            TeslaDatasetConfig {
                vins: vec![
                    "5YJ3E1EAXKF123456".to_owned(),
                    "5YJ3E1EAXKF123456".to_owned()
                ],
                ..base.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            TeslaDatasetConfig {
                max_samples: 0,
                ..base.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            TeslaDatasetConfig {
                max_samples: MAX_HISTORY_SAMPLES + 1,
                ..base
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn dataset_mode_must_be_stated_explicitly() {
        // Every other connector's dataset config accepts a bare object, so an
        // omitted mode must fail rather than silently pick one.
        assert!(serde_json::from_str::<TeslaDatasetConfig>("{}").is_err());
        assert!(serde_json::from_str::<TeslaDatasetConfig>(r#"{"schemaVersion":1}"#).is_err());
    }

    #[test]
    fn endpoints_parameter_is_sorted_and_semicolon_joined() {
        let config = TeslaDatasetConfig {
            endpoints: vec![
                TeslaVehicleEndpoint::VehicleState,
                TeslaVehicleEndpoint::ChargeState,
            ],
            ..TeslaDatasetConfig::default()
        };
        assert_eq!(config.endpoints_parameter(), "charge_state;vehicle_state");
    }

    #[test]
    fn empty_vin_selection_matches_every_vehicle() {
        let all = TeslaDatasetConfig::default();
        assert!(all.selects_vin("5YJ3E1EAXKF123456"));
        let selected = TeslaDatasetConfig {
            vins: vec!["5YJ3E1EAXKF123456".to_owned()],
            ..TeslaDatasetConfig::default()
        };
        assert!(selected.selects_vin("5YJ3E1EAXKF123456"));
        assert!(!selected.selects_vin("5YJ3E1EAXKF999999"));
    }
}
