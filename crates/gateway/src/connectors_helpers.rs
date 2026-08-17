use {super::*, serde_json::json};

pub(super) fn validate_account_fields(
    server_url: &str,
    username: &str,
    timeout_seconds: u64,
    allow_insecure_http: bool,
) -> Result<()> {
    CalDavAccountConfig::validate_non_secret_fields(
        server_url,
        username,
        timeout_seconds,
        allow_insecure_http,
    )
    .map(|_| ())
    .map_err(invalid_caldav)
}

pub(super) fn validate_dataset_request(
    config: &CalDavDatasetConfig,
    schedule_minutes: Option<u64>,
) -> Result<()> {
    config.validate().map_err(invalid_caldav)?;
    validate_schedule(schedule_minutes)
}

pub(super) fn validate_schedule(schedule_minutes: Option<u64>) -> Result<()> {
    if schedule_minutes == Some(0) {
        return Err(ConnectorManagerError::InvalidInput(
            "scheduleMinutes must be greater than zero".to_owned(),
        ));
    }
    if let Some(minutes) = schedule_minutes {
        i64::try_from(minutes).map_err(|_| {
            ConnectorManagerError::InvalidInput("scheduleMinutes is too large".to_owned())
        })?;
    }
    Ok(())
}

pub(super) fn validate_channel_dataset_config(
    config: &ChannelHistoryDatasetConfigView,
    schedule_minutes: Option<u64>,
) -> Result<()> {
    if config.schema_version != 1 {
        return Err(ConnectorManagerError::InvalidInput(
            "unsupported channel dataset schema version".to_owned(),
        ));
    }
    if config.channel_id.trim().is_empty() || config.thread_id.trim().is_empty() {
        return Err(ConnectorManagerError::InvalidInput(
            "channelId and threadId must not be empty".to_owned(),
        ));
    }
    if !(1..=200).contains(&config.limit) {
        return Err(ConnectorManagerError::InvalidInput(
            "channel message limit must be between 1 and 200".to_owned(),
        ));
    }
    if schedule_minutes == Some(0) {
        return Err(ConnectorManagerError::InvalidInput(
            "scheduleMinutes must be greater than zero".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn validate_instruction(instruction: &str) -> Result<()> {
    if instruction.trim().is_empty() {
        return Err(ConnectorManagerError::InvalidInput(
            "instruction must not be empty".to_owned(),
        ));
    }
    if instruction.len() > MAX_INSTRUCTION_BYTES {
        return Err(ConnectorManagerError::InvalidInput(format!(
            "instruction must not exceed {MAX_INSTRUCTION_BYTES} bytes"
        )));
    }
    Ok(())
}

pub(super) fn validate_password_replacement(password: &str) -> Result<()> {
    if password.trim().is_empty() {
        return Err(ConnectorManagerError::InvalidInput(
            "password must not be empty".to_owned(),
        ));
    }
    if password == REDACTED_PASSWORD {
        return Err(ConnectorManagerError::InvalidInput(
            "a redacted password cannot be used as a new password".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn account_config_value(
    server_url: String,
    username: String,
    password: Value,
    timeout_seconds: u64,
    allow_insecure_http: bool,
    allow_private_network: bool,
) -> Value {
    json!({
        "server_url": server_url,
        "username": username,
        "password": password,
        "timeout_seconds": timeout_seconds,
        "allow_insecure_http": allow_insecure_http,
        "allow_private_network": allow_private_network,
    })
}

pub(super) fn stored_password(config: &Value) -> Result<&Value> {
    config
        .as_object()
        .and_then(|config| config.get("password"))
        .ok_or_else(|| {
            internal(
                anyhow::anyhow!("missing password field"),
                "read stored password",
            )
        })
}

pub(super) fn next_sync_at(
    enabled: bool,
    schedule_minutes: Option<u64>,
) -> Result<Option<OffsetDateTime>> {
    if !enabled {
        return Ok(None);
    }
    let Some(minutes) = schedule_minutes else {
        return Ok(None);
    };
    let minutes = i64::try_from(minutes).map_err(|_| {
        ConnectorManagerError::InvalidInput("scheduleMinutes is too large".to_owned())
    })?;
    OffsetDateTime::now_utc()
        .checked_add(Duration::minutes(minutes))
        .map(Some)
        .ok_or_else(|| {
            ConnectorManagerError::InvalidInput(
                "scheduleMinutes produces an invalid next sync time".to_owned(),
            )
        })
}

pub(super) fn ensure_caldav(kind: ConnectorKind) -> Result<()> {
    match kind {
        ConnectorKind::Caldav => Ok(()),
        _ => Err(ConnectorManagerError::InvalidInput(
            "operation is only available for CalDAV connectors".to_owned(),
        )),
    }
}

pub(super) fn invalid_caldav(
    error: moltis_connector_caldav::CalDavConnectorError,
) -> ConnectorManagerError {
    ConnectorManagerError::InvalidInput(error.to_string())
}

pub(super) fn not_found(entity: &'static str, id: &str) -> ConnectorManagerError {
    ConnectorManagerError::NotFound {
        entity,
        id: id.to_owned(),
    }
}

pub(super) fn internal(
    error: impl Into<anyhow::Error>,
    context: &'static str,
) -> ConnectorManagerError {
    ConnectorManagerError::Internal(error.into().context(context))
}

pub(super) fn default_timeout_seconds() -> u64 {
    30
}

pub(super) fn default_enabled() -> bool {
    true
}

pub(super) fn default_query_limit() -> u64 {
    100
}

#[cfg(feature = "vault")]
pub(super) fn secret_aad_scope(account_id: &str) -> String {
    format!("connector:caldav:{account_id}")
}

pub(super) async fn restrict_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|error| internal(error, "restrict connector database permissions"))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(super) async fn remove_directory_best_effort(path: &Path) {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => tracing::warn!(
            path = %path.display(),
            %error,
            "failed to remove obsolete connector projection directory"
        ),
    }
}

pub(super) async fn projection_needs_rebuild(dataset: &Dataset, path: &Path) -> bool {
    let Some(last_sync_at) = dataset.last_sync_at else {
        return false;
    };
    let Ok(contents) = tokio::fs::read(path.join("manifest.json")).await else {
        return true;
    };
    let Ok(manifest) = serde_json::from_slice::<ProjectionManifest>(&contents) else {
        return true;
    };
    if manifest.dataset_id != dataset.id
        || manifest.source_sync_at != Some(last_sync_at)
        || manifest.source_plan_revision != dataset.synced_plan_revision
        || manifest.projections != dataset.projections
        || u64::try_from(manifest.item_count).ok() != Some(dataset.item_count)
        || manifest.jsonl_path.is_some() != dataset.projections.jsonl
    {
        return true;
    }
    if dataset.projections.jsonl {
        let Ok(metadata) = tokio::fs::metadata(path.join("items.jsonl")).await else {
            return true;
        };
        if !metadata.is_file() {
            return true;
        }
    }
    if !dataset.projections.markdown {
        return manifest
            .items
            .iter()
            .any(|item| item.markdown_path.is_some());
    }
    let Ok(items_metadata) = tokio::fs::metadata(path.join("items")).await else {
        return true;
    };
    if !items_metadata.is_dir() {
        return true;
    }
    for item in &manifest.items {
        let Some(markdown_path) = item.markdown_path.as_deref() else {
            return true;
        };
        let Ok(metadata) = tokio::fs::metadata(path.join(markdown_path)).await else {
            return true;
        };
        if !metadata.is_file() {
            return true;
        }
    }
    false
}

pub(super) async fn restrict_directory_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|error| internal(error, "restrict connector export permissions"))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
