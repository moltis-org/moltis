use {
    moltis_connectors::{ConnectorItemInput, SourceDisposition, SourceObservation, SourceStateMap},
    sha2::{Digest, Sha256},
    std::{collections::BTreeMap, sync::Arc, time::Instant},
    time::{OffsetDateTime, format_description::well_known::Rfc3339},
};

use crate::{
    NativeTeslaClient, Result, TeslaAccountConfig, TeslaClient, TeslaConnectorError,
    TeslaDatasetConfig, TeslaDatasetMode, TeslaSnapshot, TeslaVehicleBody, TeslaVehicleData,
    TeslaVehicleSummary,
};

const SCHEMA_VERSION: u32 = 1;
const MAX_VEHICLES_PER_ACCOUNT: usize = 200;
const MAX_SEARCH_TEXT_BYTES: usize = 4 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
const MAX_DISPLAY_NAME_BYTES: usize = 256;

pub const ITEM_KIND_STATE: &str = "tesla_vehicle_state";
pub const ITEM_KIND_SAMPLE: &str = "tesla_vehicle_sample";

pub struct TeslaConnector {
    client: Arc<dyn TeslaClient>,
}

impl TeslaConnector {
    pub fn new(account: &TeslaAccountConfig) -> Result<Self> {
        Ok(Self {
            client: Arc::new(NativeTeslaClient::new(account)?),
        })
    }

    #[must_use]
    pub fn with_client(client: Arc<dyn TeslaClient>) -> Self {
        Self { client }
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn test_connection(
        &self,
        account: &TeslaAccountConfig,
    ) -> Result<Vec<TeslaVehicleSummary>> {
        let started = Instant::now();
        account.validate()?;
        let vehicles = self.vehicles().await?;
        record_operation("test_connection", started, vehicles.len());
        Ok(vehicles)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn sync_dataset(
        &self,
        account: &TeslaAccountConfig,
        dataset: &TeslaDatasetConfig,
        existing: SourceStateMap,
        plan_revision: u64,
    ) -> Result<TeslaSnapshot> {
        self.sync_dataset_at(
            account,
            dataset,
            existing,
            plan_revision,
            OffsetDateTime::now_utc(),
        )
        .await
    }

    /// Same as [`Self::sync_dataset`] with an explicit observation timestamp,
    /// so callers that need a deterministic `observedAt` can supply one.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn sync_dataset_at(
        &self,
        account: &TeslaAccountConfig,
        dataset: &TeslaDatasetConfig,
        existing: SourceStateMap,
        plan_revision: u64,
        now: OffsetDateTime,
    ) -> Result<TeslaSnapshot> {
        let started = Instant::now();
        account.validate()?;
        dataset.validate()?;

        let observed_at = now.format(&Rfc3339).map_err(|error| {
            TeslaConnectorError::ServerResponse(format!("failed to format sync timestamp: {error}"))
        })?;
        let vehicles = self.vehicles().await?;
        let selected = vehicles
            .iter()
            .filter(|vehicle| dataset.selects_vin(&vehicle.vin))
            .cloned()
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err(TeslaConnectorError::DatasetConfig(
                "no vehicle on the account matches the dataset's VIN selection",
            ));
        }

        let endpoints = dataset.endpoints_parameter();
        let mut items = Vec::new();
        let mut observations = Vec::new();
        let mut skipped_unreachable = Vec::new();
        let mut aggregate_bytes = 0_usize;

        for vehicle in &selected {
            // Fleet API wakes a sleeping vehicle when asked for data, so an
            // unreachable car is reported rather than sampled. Previously
            // stored readings are carried forward untouched.
            let data = if vehicle.state.is_reachable() {
                Some(self.client.vehicle_data(&vehicle.vin, &endpoints).await?)
            } else {
                skipped_unreachable.push(vehicle.vin.clone());
                None
            };

            match dataset.mode {
                TeslaDatasetMode::State => sync_state_vehicle(
                    vehicle,
                    data,
                    &observed_at,
                    &existing,
                    plan_revision,
                    &mut items,
                    &mut observations,
                    &mut aggregate_bytes,
                )?,
                TeslaDatasetMode::History => sync_history_vehicle(
                    vehicle,
                    data,
                    &observed_at,
                    &existing,
                    dataset.max_samples,
                    plan_revision,
                    &mut items,
                    &mut observations,
                    &mut aggregate_bytes,
                )?,
            }
        }

        record_operation("sync", started, items.len());
        Ok(TeslaSnapshot {
            items,
            source_observations: observations,
            vehicles,
            skipped_unreachable,
        })
    }

    async fn vehicles(&self) -> Result<Vec<TeslaVehicleSummary>> {
        let vehicles = self.client.list_vehicles().await?;
        if vehicles.len() > MAX_VEHICLES_PER_ACCOUNT {
            return Err(TeslaConnectorError::ServerResponse(
                "vehicle list exceeds connector limit".to_owned(),
            ));
        }
        let mut summaries = Vec::with_capacity(vehicles.len());
        let mut seen = std::collections::HashSet::with_capacity(vehicles.len());
        for vehicle in vehicles {
            crate::validate_vin(&vehicle.vin)?;
            if vehicle
                .display_name
                .as_ref()
                .is_some_and(|name| name.len() > MAX_DISPLAY_NAME_BYTES)
            {
                return Err(TeslaConnectorError::ServerResponse(
                    "vehicle display name exceeds connector limit".to_owned(),
                ));
            }
            if !seen.insert(vehicle.vin.clone()) {
                return Err(TeslaConnectorError::ServerResponse(
                    "vehicle list contains duplicate VINs".to_owned(),
                ));
            }
            summaries.push(vehicle.into());
        }
        Ok(summaries)
    }
}

/// State datasets keep exactly one row per vehicle, keyed by VIN.
fn sync_state_vehicle(
    vehicle: &TeslaVehicleSummary,
    data: Option<TeslaVehicleData>,
    observed_at: &str,
    existing: &SourceStateMap,
    plan_revision: u64,
    items: &mut Vec<ConnectorItemInput>,
    observations: &mut Vec<SourceObservation>,
    aggregate_bytes: &mut usize,
) -> Result<()> {
    let remote_id = vehicle.vin.clone();
    let Some(data) = data else {
        // Carrying the stored row forward preserves the last good reading. With
        // no stored row yet, an empty one still records that the vehicle exists
        // and why it has no data.
        if let Some(state) = existing.get(&remote_id) {
            observations.push(SourceObservation {
                remote_id,
                remote_version: state.remote_version.clone(),
                disposition: state.disposition,
                filter_reason: state.filter_reason.clone(),
                evaluated_plan_revision: plan_revision,
            });
            return Ok(());
        }
        let item = vehicle_item(
            vehicle,
            TeslaVehicleData::default(),
            observed_at,
            remote_id.clone(),
            ITEM_KIND_STATE,
            aggregate_bytes,
        )?;
        observations.push(included_observation(
            remote_id,
            item.remote_version.clone(),
            plan_revision,
        ));
        items.push(item);
        return Ok(());
    };

    let item = vehicle_item(
        vehicle,
        data,
        observed_at,
        remote_id.clone(),
        ITEM_KIND_STATE,
        aggregate_bytes,
    )?;
    observations.push(included_observation(
        remote_id,
        item.remote_version.clone(),
        plan_revision,
    ));
    items.push(item);
    Ok(())
}

/// History datasets append one immutable row per reading and carry forward the
/// most recent `max_samples` rows. Anything older is left unobserved, which is
/// what retires it from the dataset on commit.
#[allow(clippy::too_many_arguments)]
fn sync_history_vehicle(
    vehicle: &TeslaVehicleSummary,
    data: Option<TeslaVehicleData>,
    observed_at: &str,
    existing: &SourceStateMap,
    max_samples: u32,
    plan_revision: u64,
    items: &mut Vec<ConnectorItemInput>,
    observations: &mut Vec<SourceObservation>,
    aggregate_bytes: &mut usize,
) -> Result<()> {
    let new_item = match data {
        Some(data) => {
            let remote_id = sample_remote_id(&vehicle.vin, observed_at);
            Some(vehicle_item(
                vehicle,
                data,
                observed_at,
                remote_id,
                ITEM_KIND_SAMPLE,
                aggregate_bytes,
            )?)
        },
        None => None,
    };

    let retained_capacity = usize::try_from(max_samples)
        .map_err(|_| TeslaConnectorError::DatasetConfig("maxSamples does not fit this platform"))?
        .saturating_sub(usize::from(new_item.is_some()));
    for state in retained_samples(existing, &vehicle.vin, retained_capacity) {
        observations.push(SourceObservation {
            remote_id: state.remote_id.clone(),
            remote_version: state.remote_version.clone(),
            disposition: state.disposition,
            filter_reason: state.filter_reason.clone(),
            evaluated_plan_revision: plan_revision,
        });
    }

    if let Some(item) = new_item {
        observations.push(included_observation(
            item.remote_id.clone(),
            item.remote_version.clone(),
            plan_revision,
        ));
        items.push(item);
    }
    Ok(())
}

/// Selects the newest stored samples for one vehicle. Remote IDs embed an
/// RFC 3339 timestamp after the VIN, so lexical order is chronological order.
fn retained_samples<'a>(
    existing: &'a SourceStateMap,
    vin: &str,
    capacity: usize,
) -> Vec<&'a moltis_connectors::SourceState> {
    if capacity == 0 {
        return Vec::new();
    }
    let prefix = format!("{vin}:");
    let mut ordered = existing
        .values()
        .filter(|state| state.remote_id.starts_with(&prefix))
        .collect::<Vec<_>>();
    ordered.sort_unstable_by(|left, right| right.remote_id.cmp(&left.remote_id));
    ordered.truncate(capacity);
    ordered
}

fn sample_remote_id(vin: &str, observed_at: &str) -> String {
    format!("{vin}:{observed_at}")
}

fn vehicle_item(
    vehicle: &TeslaVehicleSummary,
    data: TeslaVehicleData,
    observed_at: &str,
    remote_id: String,
    kind: &str,
    aggregate_bytes: &mut usize,
) -> Result<ConnectorItemInput> {
    let body = TeslaVehicleBody {
        schema_version: SCHEMA_VERSION,
        vin: vehicle.vin.clone(),
        display_name: vehicle.display_name.clone(),
        online_state: vehicle.state,
        observed_at: observed_at.to_owned(),
        data,
    };
    let search_text = search_text(&body);
    let body_json = serde_json::to_value(&body)?;
    let encoded = serde_json::to_vec(&body_json)?;
    *aggregate_bytes = aggregate_bytes.checked_add(encoded.len()).ok_or_else(|| {
        TeslaConnectorError::ServerResponse(
            "Tesla snapshot exceeds aggregate size limit".to_owned(),
        )
    })?;
    if *aggregate_bytes > MAX_SNAPSHOT_BYTES {
        return Err(TeslaConnectorError::ServerResponse(
            "Tesla snapshot exceeds aggregate size limit".to_owned(),
        ));
    }
    let content_hash = format!("{:x}", Sha256::digest(&encoded));
    Ok(ConnectorItemInput {
        remote_id,
        kind: kind.to_owned(),
        remote_version: Some(content_hash.clone()),
        occurred_at: Some(observed_at.to_owned()),
        updated_at: Some(observed_at.to_owned()),
        body_json,
        search_text,
        content_hash,
    })
}

fn included_observation(
    remote_id: String,
    remote_version: Option<String>,
    plan_revision: u64,
) -> SourceObservation {
    SourceObservation {
        remote_id,
        remote_version,
        disposition: SourceDisposition::Included,
        filter_reason: None,
        evaluated_plan_revision: plan_revision,
    }
}

/// Builds the free-text index entry. Readings are labelled so a search for
/// "battery" or "odometer" reaches the vehicle that recorded them.
fn search_text(body: &TeslaVehicleBody) -> String {
    let mut parts = BTreeMap::new();
    parts.insert("vin", body.vin.clone());
    if let Some(name) = &body.display_name {
        parts.insert("name", name.clone());
    }
    parts.insert("state", body.online_state.as_str().to_owned());
    parts.insert("observed", body.observed_at.clone());
    if let Some(charge) = &body.data.charge_state {
        if let Some(level) = charge.battery_level {
            parts.insert("battery", format!("{level}%"));
        }
        if let Some(range) = charge.battery_range {
            parts.insert("range", format!("{range:.1}"));
        }
        if let Some(state) = charge.charging_state {
            parts.insert("charging", format!("{state:?}"));
        }
        if let Some(limit) = charge.charge_limit_soc {
            parts.insert("charge_limit", format!("{limit}%"));
        }
    }
    if let Some(climate) = &body.data.climate_state {
        if let Some(inside) = climate.inside_temp {
            parts.insert("inside_temp", format!("{inside:.1}"));
        }
        if let Some(outside) = climate.outside_temp {
            parts.insert("outside_temp", format!("{outside:.1}"));
        }
    }
    if let Some(state) = &body.data.vehicle_state {
        if let Some(odometer) = state.odometer {
            parts.insert("odometer", format!("{odometer:.1}"));
        }
        if let Some(version) = &state.car_version {
            parts.insert("software", version.clone());
        }
        if let Some(locked) = state.locked {
            parts.insert("locked", locked.to_string());
        }
    }
    if let Some(config) = &body.data.vehicle_config
        && let Some(car_type) = &config.car_type
    {
        parts.insert("model", car_type.clone());
    }

    let mut text = parts
        .into_iter()
        .map(|(label, value)| format!("{label}: {value}"))
        .collect::<Vec<_>>()
        .join("\n");
    if text.len() > MAX_SEARCH_TEXT_BYTES {
        // Truncate on a character boundary so the stored text stays valid UTF-8.
        let mut end = MAX_SEARCH_TEXT_BYTES;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

#[cfg_attr(not(feature = "metrics"), allow(unused_variables))]
fn record_operation(operation: &'static str, started: Instant, items: usize) {
    #[cfg(feature = "metrics")]
    {
        metrics::counter!("moltis_connector_tesla_operations_total", "operation" => operation)
            .increment(1);
        metrics::histogram!("moltis_connector_tesla_operation_duration_seconds", "operation" => operation)
            .record(started.elapsed().as_secs_f64());
        metrics::histogram!("moltis_connector_tesla_operation_items", "operation" => operation)
            .record(items as f64);
    }
}
