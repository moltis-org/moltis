use {
    serde::{Deserialize, Serialize},
    serde_json::Value,
    std::{collections::BTreeMap, fmt},
    time::OffsetDateTime,
};

use crate::{ConnectorError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    Caldav,
    ChannelHistory,
    Gmail,
    Himalaya,
}

impl ConnectorKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Caldav => "caldav",
            Self::ChannelHistory => "channel_history",
            Self::Gmail => "gmail",
            Self::Himalaya => "himalaya",
        }
    }
}

impl fmt::Display for ConnectorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<&str> for ConnectorKind {
    type Error = ConnectorError;

    fn try_from(value: &str) -> Result<Self> {
        match value {
            "caldav" => Ok(Self::Caldav),
            "channel_history" => Ok(Self::ChannelHistory),
            "gmail" => Ok(Self::Gmail),
            "himalaya" => Ok(Self::Himalaya),
            other => Err(ConnectorError::UnknownConnectorKind(other.to_owned())),
        }
    }
}

impl TryFrom<String> for ConnectorKind {
    type Error = ConnectorError;

    fn try_from(value: String) -> Result<Self> {
        Self::try_from(value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub kind: ConnectorKind,
    pub source_key: Option<String>,
    pub name: String,
    pub config: Value,
    pub enabled: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountCreate {
    pub kind: ConnectorKind,
    pub source_key: Option<String>,
    pub name: String,
    pub config: Value,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountUpdate {
    pub name: String,
    pub config: Value,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionConfig {
    pub jsonl: bool,
    pub markdown: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dataset {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub instruction: Option<String>,
    pub config: Value,
    pub plan_revision: u64,
    pub synced_plan_revision: Option<u64>,
    pub schedule_minutes: Option<u64>,
    pub projections: ProjectionConfig,
    pub enabled: bool,
    pub last_sync_at: Option<OffsetDateTime>,
    pub next_sync_at: Option<OffsetDateTime>,
    pub last_error: Option<String>,
    pub item_count: u64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetCreate {
    pub account_id: String,
    pub name: String,
    pub instruction: Option<String>,
    pub config: Value,
    pub schedule_minutes: Option<u64>,
    pub projections: ProjectionConfig,
    pub enabled: bool,
    pub next_sync_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetUpdate {
    pub name: String,
    pub instruction: Option<String>,
    pub config: Value,
    pub schedule_minutes: Option<u64>,
    pub projections: ProjectionConfig,
    pub enabled: bool,
    pub next_sync_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorItemInput {
    pub remote_id: String,
    pub kind: String,
    pub remote_version: Option<String>,
    pub occurred_at: Option<String>,
    pub updated_at: Option<String>,
    pub body_json: Value,
    pub search_text: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorItem {
    pub id: String,
    pub dataset_id: String,
    pub remote_id: String,
    pub kind: String,
    pub remote_version: Option<String>,
    pub occurred_at: Option<String>,
    pub updated_at: Option<String>,
    pub body_json: Value,
    pub search_text: String,
    pub content_hash: String,
    pub created_at: OffsetDateTime,
    pub stored_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceDisposition {
    Included,
    Filtered,
}

impl SourceDisposition {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Included => "included",
            Self::Filtered => "filtered",
        }
    }
}

impl TryFrom<&str> for SourceDisposition {
    type Error = ConnectorError;

    fn try_from(value: &str) -> Result<Self> {
        match value {
            "included" => Ok(Self::Included),
            "filtered" => Ok(Self::Filtered),
            other => Err(ConnectorError::InvalidInput(format!(
                "unknown source disposition: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceState {
    pub dataset_id: String,
    pub remote_id: String,
    pub remote_version: Option<String>,
    pub disposition: SourceDisposition,
    pub filter_reason: Option<String>,
    pub evaluated_plan_revision: u64,
    pub last_seen_run_id: String,
    pub observed_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceObservation {
    pub remote_id: String,
    pub remote_version: Option<String>,
    pub disposition: SourceDisposition,
    pub filter_reason: Option<String>,
    pub evaluated_plan_revision: u64,
}

pub type SourceStateMap = BTreeMap<String, SourceState>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemQuery {
    pub limit: u64,
    pub offset: u64,
    pub include_deleted: bool,
    pub text: Option<String>,
}

impl Default for ItemQuery {
    fn default() -> Self {
        Self {
            limit: 100,
            offset: 0,
            include_deleted: false,
            text: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotResult {
    pub upserted: u64,
    pub deleted: u64,
    pub active: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncRunStatus {
    Running,
    Succeeded,
    Failed,
}

impl SyncRunStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

impl TryFrom<&str> for SyncRunStatus {
    type Error = ConnectorError;

    fn try_from(value: &str) -> Result<Self> {
        match value {
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            other => Err(ConnectorError::UnknownSyncRunStatus(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncRun {
    pub id: String,
    pub dataset_id: String,
    pub status: SyncRunStatus,
    pub started_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
    pub upserted: u64,
    pub deleted: u64,
    pub active: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionManifestItem {
    pub remote_id: String,
    pub kind: String,
    pub content_hash: String,
    pub markdown_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionManifest {
    pub dataset_id: String,
    pub dataset_name: String,
    pub source_sync_at: Option<OffsetDateTime>,
    pub source_plan_revision: Option<u64>,
    pub projections: ProjectionConfig,
    pub generated_at: OffsetDateTime,
    pub item_count: usize,
    pub jsonl_path: Option<String>,
    pub items: Vec<ProjectionManifestItem>,
}
