#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Sync-shape coverage: what a Tesla dataset stores, what it carries forward,
//! and what it refuses to do to a sleeping car.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use {
    async_trait::async_trait,
    moltis_connector_tesla::{
        ITEM_KIND_SAMPLE, ITEM_KIND_STATE, Result, TeslaAccountConfig, TeslaApiVehicle,
        TeslaChargeState, TeslaClient, TeslaConnector, TeslaConnectorError, TeslaDatasetConfig,
        TeslaDatasetMode, TeslaRegion, TeslaVehicleBody, TeslaVehicleData, TeslaVehicleOnlineState,
        TeslaVehicleStateData,
    },
    moltis_connectors::{SourceDisposition, SourceObservation, SourceState, SourceStateMap},
    secrecy::Secret,
    time::OffsetDateTime,
};

const VIN_A: &str = "5YJ3E1EAXKF123456";
const VIN_B: &str = "5YJ3E1EAXKF999999";

struct StubClient {
    vehicles: Vec<TeslaApiVehicle>,
    battery: u8,
    data_calls: AtomicUsize,
}

impl StubClient {
    fn new(vehicles: Vec<TeslaApiVehicle>, battery: u8) -> Arc<Self> {
        Arc::new(Self {
            vehicles,
            battery,
            data_calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl TeslaClient for StubClient {
    async fn list_vehicles(&self) -> Result<Vec<TeslaApiVehicle>> {
        Ok(self.vehicles.clone())
    }

    async fn vehicle_data(&self, _vin: &str, _endpoints: &str) -> Result<TeslaVehicleData> {
        self.data_calls.fetch_add(1, Ordering::Relaxed);
        Ok(TeslaVehicleData {
            charge_state: Some(TeslaChargeState {
                battery_level: Some(self.battery),
                ..TeslaChargeState::default()
            }),
            vehicle_state: Some(TeslaVehicleStateData {
                odometer: Some(12_345.6),
                ..TeslaVehicleStateData::default()
            }),
            ..TeslaVehicleData::default()
        })
    }
}

fn vehicle(vin: &str, state: TeslaVehicleOnlineState) -> TeslaApiVehicle {
    TeslaApiVehicle {
        vin: vin.to_owned(),
        display_name: Some("Car".to_owned()),
        state,
    }
}

fn account() -> TeslaAccountConfig {
    TeslaAccountConfig {
        schema_version: 1,
        region: TeslaRegion::Europe,
        client_id: "client".to_owned(),
        refresh_token: Secret::new("refresh".to_owned()),
    }
}

fn history_dataset(max_samples: u32) -> TeslaDatasetConfig {
    TeslaDatasetConfig {
        mode: TeslaDatasetMode::History,
        max_samples,
        ..TeslaDatasetConfig::default()
    }
}

fn at(timestamp: &str) -> OffsetDateTime {
    OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339).unwrap()
}

/// Rebuilds the source-state map the store would hold after committing a run,
/// so the next sync sees what production would give it.
fn states_after(observations: &[SourceObservation], plan_revision: u64) -> SourceStateMap {
    observations
        .iter()
        .map(|observation| {
            (observation.remote_id.clone(), SourceState {
                dataset_id: "dataset-1".to_owned(),
                remote_id: observation.remote_id.clone(),
                remote_version: observation.remote_version.clone(),
                disposition: observation.disposition,
                filter_reason: observation.filter_reason.clone(),
                evaluated_plan_revision: plan_revision,
                last_seen_run_id: "run-1".to_owned(),
                observed_at: OffsetDateTime::UNIX_EPOCH,
            })
        })
        .collect::<BTreeMap<_, _>>()
}

fn body(item: &moltis_connectors::ConnectorItemInput) -> TeslaVehicleBody {
    serde_json::from_value(item.body_json.clone()).unwrap()
}

#[tokio::test]
async fn state_mode_keeps_one_row_per_vehicle_and_replaces_it() {
    let client = StubClient::new(
        vec![
            vehicle(VIN_A, TeslaVehicleOnlineState::Online),
            vehicle(VIN_B, TeslaVehicleOnlineState::Online),
        ],
        80,
    );
    let connector = TeslaConnector::with_client(client.clone());
    let dataset = TeslaDatasetConfig::default();

    let first = connector
        .sync_dataset_at(
            &account(),
            &dataset,
            SourceStateMap::new(),
            1,
            at("2026-08-17T10:00:00Z"),
        )
        .await
        .unwrap();
    assert_eq!(first.items.len(), 2);
    assert!(first.items.iter().all(|item| item.kind == ITEM_KIND_STATE));
    let mut remote_ids = first
        .items
        .iter()
        .map(|item| item.remote_id.clone())
        .collect::<Vec<_>>();
    remote_ids.sort();
    assert_eq!(remote_ids, vec![VIN_A, VIN_B]);

    // A second run must not accumulate rows: the remote IDs are stable.
    let second = connector
        .sync_dataset_at(
            &account(),
            &dataset,
            states_after(&first.source_observations, 1),
            1,
            at("2026-08-17T11:00:00Z"),
        )
        .await
        .unwrap();
    assert_eq!(second.items.len(), 2);
    assert_eq!(body(&second.items[0]).observed_at, "2026-08-17T11:00:00Z");
}

#[tokio::test]
async fn history_mode_appends_a_sample_per_sync() {
    let client = StubClient::new(vec![vehicle(VIN_A, TeslaVehicleOnlineState::Online)], 80);
    let connector = TeslaConnector::with_client(client);
    let dataset = history_dataset(10);

    let mut states = SourceStateMap::new();
    for hour in 0..3 {
        let snapshot = connector
            .sync_dataset_at(
                &account(),
                &dataset,
                states,
                1,
                at(&format!("2026-08-17T1{hour}:00:00Z")),
            )
            .await
            .unwrap();
        assert_eq!(snapshot.items.len(), 1, "each sync adds exactly one sample");
        assert_eq!(snapshot.items[0].kind, ITEM_KIND_SAMPLE);
        states = states_after(&snapshot.source_observations, 1);
    }
    // Three syncs leave three retained observations: two carried, one new.
    assert_eq!(states.len(), 3);
    assert!(
        states
            .keys()
            .all(|key| key.starts_with(&format!("{VIN_A}:")))
    );
}

#[tokio::test]
async fn history_retention_drops_the_oldest_samples_beyond_max_samples() {
    let client = StubClient::new(vec![vehicle(VIN_A, TeslaVehicleOnlineState::Online)], 80);
    let connector = TeslaConnector::with_client(client);
    let dataset = history_dataset(3);

    let mut states = SourceStateMap::new();
    let mut last_observations = Vec::new();
    for hour in 0..5 {
        let snapshot = connector
            .sync_dataset_at(
                &account(),
                &dataset,
                states,
                1,
                at(&format!("2026-08-17T1{hour}:00:00Z")),
            )
            .await
            .unwrap();
        last_observations = snapshot.source_observations.clone();
        states = states_after(&snapshot.source_observations, 1);
    }

    assert_eq!(states.len(), 3, "retention caps the observed window");
    let mut retained = last_observations
        .iter()
        .map(|observation| observation.remote_id.clone())
        .collect::<Vec<_>>();
    retained.sort();
    // The two oldest samples are simply not observed, which is what retires
    // them from the dataset when the run commits.
    assert_eq!(retained, vec![
        format!("{VIN_A}:2026-08-17T12:00:00Z"),
        format!("{VIN_A}:2026-08-17T13:00:00Z"),
        format!("{VIN_A}:2026-08-17T14:00:00Z"),
    ]);
}

#[tokio::test]
async fn a_sleeping_vehicle_is_never_woken_and_its_last_reading_survives() {
    let online = StubClient::new(vec![vehicle(VIN_A, TeslaVehicleOnlineState::Online)], 80);
    let connector = TeslaConnector::with_client(online.clone());
    let dataset = TeslaDatasetConfig::default();
    let first = connector
        .sync_dataset_at(
            &account(),
            &dataset,
            SourceStateMap::new(),
            1,
            at("2026-08-17T10:00:00Z"),
        )
        .await
        .unwrap();
    assert_eq!(online.data_calls.load(Ordering::Relaxed), 1);
    let stored_version = first.items[0].remote_version.clone();

    let asleep = StubClient::new(vec![vehicle(VIN_A, TeslaVehicleOnlineState::Asleep)], 20);
    let connector = TeslaConnector::with_client(asleep.clone());
    let second = connector
        .sync_dataset_at(
            &account(),
            &dataset,
            states_after(&first.source_observations, 1),
            1,
            at("2026-08-17T11:00:00Z"),
        )
        .await
        .unwrap();

    assert_eq!(
        asleep.data_calls.load(Ordering::Relaxed),
        0,
        "an asleep vehicle must not be polled for data"
    );
    assert!(second.items.is_empty(), "no row is rewritten");
    assert_eq!(second.skipped_unreachable, vec![VIN_A.to_owned()]);
    // The carried-forward observation keeps the stored row alive unchanged.
    assert_eq!(second.source_observations.len(), 1);
    assert_eq!(second.source_observations[0].remote_id, VIN_A);
    assert_eq!(second.source_observations[0].remote_version, stored_version);
    assert_eq!(
        second.source_observations[0].disposition,
        SourceDisposition::Included
    );
}

#[tokio::test]
async fn a_first_sync_of_an_unreachable_vehicle_still_records_it() {
    let client = StubClient::new(vec![vehicle(VIN_A, TeslaVehicleOnlineState::Offline)], 80);
    let connector = TeslaConnector::with_client(client.clone());
    let snapshot = connector
        .sync_dataset_at(
            &account(),
            &TeslaDatasetConfig::default(),
            SourceStateMap::new(),
            1,
            at("2026-08-17T10:00:00Z"),
        )
        .await
        .unwrap();

    assert_eq!(client.data_calls.load(Ordering::Relaxed), 0);
    assert_eq!(snapshot.items.len(), 1);
    let body = body(&snapshot.items[0]);
    assert_eq!(body.online_state, TeslaVehicleOnlineState::Offline);
    assert_eq!(body.data, TeslaVehicleData::default());
}

#[tokio::test]
async fn history_mode_records_nothing_new_while_a_vehicle_is_unreachable() {
    let online = StubClient::new(vec![vehicle(VIN_A, TeslaVehicleOnlineState::Online)], 80);
    let dataset = history_dataset(10);
    let first = TeslaConnector::with_client(online)
        .sync_dataset_at(
            &account(),
            &dataset,
            SourceStateMap::new(),
            1,
            at("2026-08-17T10:00:00Z"),
        )
        .await
        .unwrap();

    let asleep = StubClient::new(vec![vehicle(VIN_A, TeslaVehicleOnlineState::Asleep)], 20);
    let second = TeslaConnector::with_client(asleep)
        .sync_dataset_at(
            &account(),
            &dataset,
            states_after(&first.source_observations, 1),
            1,
            at("2026-08-17T11:00:00Z"),
        )
        .await
        .unwrap();

    assert!(second.items.is_empty());
    assert_eq!(second.source_observations.len(), 1, "history is preserved");
    assert_eq!(
        second.source_observations[0].remote_id,
        format!("{VIN_A}:2026-08-17T10:00:00Z")
    );
}

#[tokio::test]
async fn vin_selection_limits_the_dataset_and_rejects_an_empty_match() {
    let client = StubClient::new(
        vec![
            vehicle(VIN_A, TeslaVehicleOnlineState::Online),
            vehicle(VIN_B, TeslaVehicleOnlineState::Online),
        ],
        80,
    );
    let connector = TeslaConnector::with_client(client);

    let selected = TeslaDatasetConfig {
        vins: vec![VIN_B.to_owned()],
        ..TeslaDatasetConfig::default()
    };
    let snapshot = connector
        .sync_dataset_at(
            &account(),
            &selected,
            SourceStateMap::new(),
            1,
            at("2026-08-17T10:00:00Z"),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.items.len(), 1);
    assert_eq!(snapshot.items[0].remote_id, VIN_B);

    let missing = TeslaDatasetConfig {
        vins: vec!["5YJ3E1EAXKF000000".to_owned()],
        ..TeslaDatasetConfig::default()
    };
    assert!(matches!(
        connector
            .sync_dataset_at(
                &account(),
                &missing,
                SourceStateMap::new(),
                1,
                at("2026-08-17T10:00:00Z"),
            )
            .await,
        Err(TeslaConnectorError::DatasetConfig(_))
    ));
}

#[tokio::test]
async fn a_malformed_vehicle_list_is_rejected_before_it_reaches_the_store() {
    let duplicate = StubClient::new(
        vec![
            vehicle(VIN_A, TeslaVehicleOnlineState::Online),
            vehicle(VIN_A, TeslaVehicleOnlineState::Online),
        ],
        80,
    );
    assert!(matches!(
        TeslaConnector::with_client(duplicate)
            .test_connection(&account())
            .await,
        Err(TeslaConnectorError::ServerResponse(_))
    ));

    let bad_vin = StubClient::new(vec![vehicle("SHORT", TeslaVehicleOnlineState::Online)], 80);
    assert!(matches!(
        TeslaConnector::with_client(bad_vin)
            .test_connection(&account())
            .await,
        Err(TeslaConnectorError::DatasetConfig(_))
    ));
}

#[tokio::test]
async fn search_text_indexes_the_readings_a_user_would_search_for() {
    let client = StubClient::new(vec![vehicle(VIN_A, TeslaVehicleOnlineState::Online)], 80);
    let snapshot = TeslaConnector::with_client(client)
        .sync_dataset_at(
            &account(),
            &TeslaDatasetConfig::default(),
            SourceStateMap::new(),
            1,
            at("2026-08-17T10:00:00Z"),
        )
        .await
        .unwrap();
    let search_text = &snapshot.items[0].search_text;
    for expected in ["vin: ", VIN_A, "battery: 80%", "odometer: 12345.6"] {
        assert!(
            search_text.contains(expected),
            "search text missing {expected}: {search_text}"
        );
    }
}
