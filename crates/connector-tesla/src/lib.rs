//! Typed, read-only Tesla Fleet API adapter for the connector snapshot store.
//!
//! Moltis never sends vehicle commands and never wakes a sleeping car: a
//! vehicle that is not already online is reported as unreachable and its last
//! stored reading is carried forward unchanged.

mod client;
mod config;
mod connector;
mod error;
mod model;
mod tool;

pub use {
    client::{NativeTeslaClient, TeslaClient},
    config::{
        TeslaAccountConfig, TeslaDatasetConfig, TeslaDatasetMode, TeslaRegion,
        TeslaVehicleEndpoint, default_endpoints, validate_vin,
    },
    connector::{ITEM_KIND_SAMPLE, ITEM_KIND_STATE, TeslaConnector},
    error::{Result, TeslaConnectorError},
    model::{
        TeslaApiVehicle, TeslaChargeState, TeslaChargingState, TeslaClimateState, TeslaDriveState,
        TeslaGuiSettings, TeslaResponse, TeslaSnapshot, TeslaVehicleBody, TeslaVehicleConfigData,
        TeslaVehicleData, TeslaVehicleOnlineState, TeslaVehicleStateData, TeslaVehicleSummary,
    },
    tool::{TeslaConnectorTool, register_tesla_connector_tool},
};
