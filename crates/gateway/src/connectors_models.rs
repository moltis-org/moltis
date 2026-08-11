use std::path::Path;

use {
    moltis_connector_caldav::{CalDavDatasetConfig, CalDavFilters, CalendarSelection},
    moltis_connectors::{
        Account, ConnectorItem, ConnectorKind, Dataset, ProjectionConfig, SyncRun,
    },
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    time::OffsetDateTime,
};

use super::{Result, default_enabled, default_query_limit, default_timeout_seconds, internal};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDescriptor {
    pub kind: ConnectorKind,
    pub display_name: &'static str,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCreateRequest {
    pub kind: ConnectorKind,
    pub name: String,
    pub server_url: String,
    pub username: String,
    pub password: Secret<String>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub allow_insecure_http: bool,
    #[serde(default)]
    pub allow_private_network: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdateRequest {
    pub name: String,
    pub server_url: String,
    pub username: String,
    #[serde(default)]
    pub password: Option<Secret<String>>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub allow_insecure_http: bool,
    #[serde(default)]
    pub allow_private_network: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub kind: ConnectorKind,
    pub name: String,
    pub server_url: String,
    pub username: String,
    pub timeout_seconds: u64,
    pub allow_insecure_http: bool,
    pub allow_private_network: bool,
    pub has_password: bool,
    pub managed: bool,
    pub enabled: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CalendarSelectionView {
    #[default]
    All,
    Selected {
        calendar_hrefs: Vec<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalDavFiltersView {
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub accepted_by_account: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalDavDatasetConfigView {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub selection: CalendarSelectionView,
    #[serde(default)]
    pub filters: CalDavFiltersView,
}

impl Default for CalDavDatasetConfigView {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            selection: CalendarSelectionView::default(),
            filters: CalDavFiltersView::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetCreateRequest {
    pub account_id: String,
    pub name: String,
    pub instruction: String,
    #[serde(default)]
    pub config: CalDavDatasetConfigView,
    #[serde(default)]
    pub schedule_minutes: Option<u64>,
    #[serde(default)]
    pub projections: ProjectionConfig,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetUpdateRequest {
    pub name: String,
    pub instruction: String,
    #[serde(default)]
    pub config: CalDavDatasetConfigView,
    #[serde(default)]
    pub schedule_minutes: Option<u64>,
    #[serde(default)]
    pub projections: ProjectionConfig,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetView {
    pub id: String,
    pub account_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    pub config: CalDavDatasetConfigView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_minutes: Option<u64>,
    pub projections: ProjectionConfig,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_sync_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub item_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection_path: Option<String>,
    pub needs_sync: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRunView {
    pub id: String,
    pub dataset_id: String,
    pub status: moltis_connectors::SyncRunStatus,
    pub started_at: OffsetDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<OffsetDateTime>,
    pub upserted: u64,
    pub deleted: u64,
    pub active: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorItemView {
    pub id: String,
    pub dataset_id: String,
    pub remote_id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    pub body_json: Value,
    pub content_hash: String,
    pub created_at: OffsetDateTime,
    pub stored_at: OffsetDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarView {
    pub href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection_etag: Option<String>,
    pub supports_sync: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemQueryRequest {
    #[serde(default = "default_query_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub include_deleted: bool,
    #[serde(default)]
    pub text: Option<String>,
}

impl Default for ItemQueryRequest {
    fn default() -> Self {
        Self {
            limit: default_query_limit(),
            offset: 0,
            include_deleted: false,
            text: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetCompileRequest {
    pub account_id: String,
    #[serde(default)]
    pub dataset_id: Option<String>,
    pub instruction: String,
    #[serde(default)]
    pub overrides: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetDraft {
    pub name: String,
    pub config: CalDavDatasetConfigView,
    #[serde(default)]
    pub schedule_minutes: Option<u64>,
    #[serde(default)]
    pub projections: ProjectionConfig,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetCompileResponse {
    pub draft: DatasetDraft,
    pub summary: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct StoredAccountConfigView {
    pub(super) server_url: String,
    pub(super) username: String,
    pub(super) password: Value,
    #[serde(default = "default_timeout_seconds")]
    pub(super) timeout_seconds: u64,
    #[serde(default)]
    pub(super) allow_insecure_http: bool,
    #[serde(default)]
    pub(super) allow_private_network: bool,
}

pub(super) fn account_view(account: Account) -> Result<AccountView> {
    let config: StoredAccountConfigView = serde_json::from_value(account.config)
        .map_err(|error| internal(error, "deserialize connector account view"))?;
    Ok(AccountView {
        id: account.id,
        kind: account.kind,
        name: account.name,
        server_url: config.server_url,
        username: config.username,
        timeout_seconds: config.timeout_seconds,
        allow_insecure_http: config.allow_insecure_http,
        allow_private_network: config.allow_private_network,
        has_password: stored_secret_present(&config.password)?,
        managed: account.source_key.is_some(),
        enabled: account.enabled,
        created_at: account.created_at,
        updated_at: account.updated_at,
    })
}

pub(super) fn dataset_view(dataset: Dataset, export_root: &Path) -> Result<DatasetView> {
    let config: CalDavDatasetConfig = serde_json::from_value(dataset.config)
        .map_err(|error| internal(error, "deserialize connector dataset view"))?;
    let projection_path = if dataset.projections.jsonl || dataset.projections.markdown {
        Some(
            moltis_connectors::projection_directory(export_root, &dataset.name, &dataset.id)?
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        None
    };
    Ok(DatasetView {
        id: dataset.id,
        account_id: dataset.account_id,
        name: dataset.name,
        instruction: dataset.instruction,
        config: config.into(),
        schedule_minutes: dataset.schedule_minutes,
        projections: dataset.projections,
        enabled: dataset.enabled,
        last_sync_at: dataset.last_sync_at,
        next_sync_at: dataset.next_sync_at,
        last_error: dataset.last_error,
        item_count: dataset.item_count,
        projection_path,
        needs_sync: dataset.synced_plan_revision != Some(dataset.plan_revision),
        created_at: dataset.created_at,
        updated_at: dataset.updated_at,
    })
}

pub(super) fn stored_secret_present(value: &Value) -> Result<bool> {
    match value {
        Value::String(value) => Ok(!value.is_empty()),
        Value::Object(value)
            if value.get("kind").and_then(Value::as_str) == Some("vault_encrypted") =>
        {
            Ok(true)
        },
        Value::Null => Ok(false),
        _ => Err(internal(
            anyhow::anyhow!("unsupported stored password representation"),
            "inspect stored connector password",
        )),
    }
}

impl From<CalendarSelectionView> for CalendarSelection {
    fn from(selection: CalendarSelectionView) -> Self {
        match selection {
            CalendarSelectionView::All => Self::All,
            CalendarSelectionView::Selected { calendar_hrefs } => Self::Selected { calendar_hrefs },
        }
    }
}

impl From<CalendarSelection> for CalendarSelectionView {
    fn from(selection: CalendarSelection) -> Self {
        match selection {
            CalendarSelection::All => Self::All,
            CalendarSelection::Selected { calendar_hrefs } => Self::Selected { calendar_hrefs },
        }
    }
}

impl From<CalDavFiltersView> for CalDavFilters {
    fn from(filters: CalDavFiltersView) -> Self {
        Self {
            start_date: filters.start_date,
            end_date: filters.end_date,
            accepted_by_account: filters.accepted_by_account,
        }
    }
}

impl From<CalDavFilters> for CalDavFiltersView {
    fn from(filters: CalDavFilters) -> Self {
        Self {
            start_date: filters.start_date,
            end_date: filters.end_date,
            accepted_by_account: filters.accepted_by_account,
        }
    }
}

impl From<CalDavDatasetConfigView> for CalDavDatasetConfig {
    fn from(config: CalDavDatasetConfigView) -> Self {
        Self {
            schema_version: config.schema_version,
            selection: config.selection.into(),
            filters: config.filters.into(),
        }
    }
}

impl From<CalDavDatasetConfig> for CalDavDatasetConfigView {
    fn from(config: CalDavDatasetConfig) -> Self {
        Self {
            schema_version: config.schema_version,
            selection: config.selection.into(),
            filters: config.filters.into(),
        }
    }
}

impl From<SyncRun> for SyncRunView {
    fn from(run: SyncRun) -> Self {
        Self {
            id: run.id,
            dataset_id: run.dataset_id,
            status: run.status,
            started_at: run.started_at,
            finished_at: run.finished_at,
            upserted: run.upserted,
            deleted: run.deleted,
            active: run.active,
            error: run.error,
        }
    }
}

impl From<ConnectorItem> for ConnectorItemView {
    fn from(item: ConnectorItem) -> Self {
        Self {
            id: item.id,
            dataset_id: item.dataset_id,
            remote_id: item.remote_id,
            kind: item.kind,
            remote_version: item.remote_version,
            occurred_at: item.occurred_at,
            updated_at: item.updated_at,
            body_json: item.body_json,
            content_hash: item.content_hash,
            created_at: item.created_at,
            stored_at: item.stored_at,
            deleted_at: item.deleted_at,
        }
    }
}

const fn default_schema_version() -> u32 {
    1
}
