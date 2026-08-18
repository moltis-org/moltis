mod domain;
mod error;
mod migration;
pub mod projection;
mod reader;
mod search;
mod source;
mod store;

pub use {
    domain::{
        Account, AccountCreate, AccountUpdate, ConnectorItem, ConnectorItemInput, ConnectorKind,
        Dataset, DatasetCreate, DatasetUpdate, ItemQuery, ProjectionConfig, ProjectionManifest,
        ProjectionManifestItem, SnapshotResult, SourceDisposition, SourceObservation, SourceState,
        SourceStateMap, SyncRun, SyncRunStatus,
    },
    error::{ConnectorError, Result},
    migration::run_migrations,
    projection::{cleanup_projection_artifacts, projection_directory, write_projection},
    reader::ConnectorReader,
    search::MAX_SEARCH_QUERY_LENGTH,
    store::{MAX_QUERY_LIMIT, SqliteConnectorStore},
};
