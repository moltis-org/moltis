//! Tesla connector account and dataset lifecycle coverage.

use {
    super::*,
    moltis_connector_tesla::{TeslaDatasetConfig, TeslaDatasetMode, TeslaRegion},
};

const CLIENT_ID: &str = "tesla-client-id";
const VIN: &str = "5YJ3E1EAXKF123456";

fn create_request(refresh_token: &str) -> AccountCreateRequest {
    AccountCreateRequest {
        kind: ConnectorKind::Tesla,
        name: "My Tesla".to_owned(),
        server_url: String::new(),
        username: String::new(),
        password: Secret::new(String::new()),
        channel_type: None,
        channel_account_id: None,
        himalaya_account_name: None,
        himalaya_backend: None,
        tesla_region: Some(TeslaRegion::Europe),
        tesla_client_id: Some(CLIENT_ID.to_owned()),
        tesla_refresh_token: Some(Secret::new(refresh_token.to_owned())),
        timeout_seconds: 30,
        allow_insecure_http: false,
        allow_private_network: false,
        enabled: true,
    }
}

fn update_request(name: &str, refresh_token: Option<&str>) -> AccountUpdateRequest {
    AccountUpdateRequest {
        name: name.to_owned(),
        server_url: String::new(),
        username: String::new(),
        password: None,
        tesla_region: Some(TeslaRegion::Europe),
        tesla_client_id: Some(CLIENT_ID.to_owned()),
        tesla_refresh_token: refresh_token.map(|token| Secret::new(token.to_owned())),
        timeout_seconds: 30,
        allow_insecure_http: false,
        allow_private_network: false,
        enabled: true,
    }
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn account_view_never_exposes_the_refresh_token() {
    let (_temp, manager) = manager().await;
    let token = test_password();
    let created = manager.add_account(create_request(&token)).await.unwrap();

    assert_eq!(created.kind, ConnectorKind::Tesla);
    assert_eq!(created.tesla_region, Some(TeslaRegion::Europe));
    assert_eq!(created.tesla_client_id, Some(CLIENT_ID.to_owned()));
    assert!(created.has_password, "a stored credential is reported");
    let serialized = serde_json::to_string(&created).unwrap();
    assert!(!serialized.contains(&token));
    assert!(!serialized.contains("refreshToken"));
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn an_omitted_or_redacted_refresh_token_preserves_the_stored_one() {
    let (_temp, manager) = manager().await;
    let token = test_password();
    let created = manager.add_account(create_request(&token)).await.unwrap();

    for (label, replacement) in [("omitted", None), ("redacted", Some(REDACTED_PASSWORD))] {
        let updated = manager
            .update_account(&created.id, update_request("Renamed", replacement))
            .await
            .unwrap();
        assert!(updated.has_password, "{label} update kept the credential");
        let stored = manager
            .store
            .get_account(&created.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.config["refreshToken"], token,
            "{label} update must not overwrite the stored refresh token"
        );
    }
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn a_supplied_refresh_token_replaces_the_stored_one() {
    let (_temp, manager) = manager().await;
    let created = manager
        .add_account(create_request(&test_password()))
        .await
        .unwrap();
    let rotated = test_password();
    manager
        .update_account(&created.id, update_request("My Tesla", Some(&rotated)))
        .await
        .unwrap();
    let stored = manager
        .store
        .get_account(&created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.config["refreshToken"], rotated);
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn incomplete_credentials_are_rejected() {
    let (_temp, manager) = manager().await;
    let mut missing_token = create_request("");
    missing_token.tesla_refresh_token = None;
    assert!(manager.add_account(missing_token).await.is_err());

    let mut missing_client = create_request(&test_password());
    missing_client.tesla_client_id = None;
    assert!(manager.add_account(missing_client).await.is_err());

    // A redacted placeholder is never a usable credential.
    let redacted = create_request(REDACTED_PASSWORD);
    assert!(manager.add_account(redacted).await.is_err());
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn datasets_round_trip_both_modes_and_reject_foreign_configs() {
    let (_temp, manager) = manager().await;
    let account = manager
        .add_account(create_request(&test_password()))
        .await
        .unwrap();

    for mode in [TeslaDatasetMode::State, TeslaDatasetMode::History] {
        let dataset = manager
            .add_dataset(DatasetCreateRequest {
                account_id: account.id.clone(),
                name: format!("{mode:?} readings"),
                instruction: "Track the car".to_owned(),
                config: ConnectorDatasetConfigView::Tesla(TeslaDatasetConfig {
                    mode,
                    vins: vec![VIN.to_owned()],
                    max_samples: 500,
                    ..TeslaDatasetConfig::default()
                }),
                schedule_minutes: Some(60),
                projections: ProjectionConfig::default(),
                enabled: true,
            })
            .await
            .unwrap();
        let ConnectorDatasetConfigView::Tesla(config) = dataset.config else {
            panic!("stored dataset did not read back as a Tesla config");
        };
        assert_eq!(config.mode, mode);
        assert_eq!(config.vins, vec![VIN.to_owned()]);
        assert_eq!(config.max_samples, 500);
    }

    // A CalDAV dataset config must not attach to a Tesla connection.
    assert!(
        manager
            .add_dataset(DatasetCreateRequest {
                account_id: account.id.clone(),
                name: "Wrong kind".to_owned(),
                instruction: "Track the car".to_owned(),
                config: ConnectorDatasetConfigView::CalDav(CalDavDatasetConfigView::default()),
                schedule_minutes: None,
                projections: ProjectionConfig::default(),
                enabled: true,
            })
            .await
            .is_err()
    );
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn invalid_dataset_configuration_is_rejected() {
    let (_temp, manager) = manager().await;
    let account = manager
        .add_account(create_request(&test_password()))
        .await
        .unwrap();
    for config in [
        TeslaDatasetConfig {
            endpoints: Vec::new(),
            ..TeslaDatasetConfig::default()
        },
        TeslaDatasetConfig {
            vins: vec!["not-a-vin".to_owned()],
            ..TeslaDatasetConfig::default()
        },
        TeslaDatasetConfig {
            max_samples: 0,
            ..TeslaDatasetConfig::default()
        },
    ] {
        assert!(
            manager
                .add_dataset(DatasetCreateRequest {
                    account_id: account.id.clone(),
                    name: "Invalid".to_owned(),
                    instruction: "Track the car".to_owned(),
                    config: ConnectorDatasetConfigView::Tesla(config),
                    schedule_minutes: None,
                    projections: ProjectionConfig::default(),
                    enabled: true,
                })
                .await
                .is_err()
        );
    }
}

#[tokio::test]
#[cfg_attr(feature = "vault", serial_test::serial(vault_runtime))]
async fn tesla_is_offered_as_a_connector_kind() {
    let (_temp, manager) = manager().await;
    assert!(
        manager
            .available()
            .iter()
            .any(|descriptor| descriptor.kind == ConnectorKind::Tesla)
    );
}
