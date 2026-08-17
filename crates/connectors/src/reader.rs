use async_trait::async_trait;

use crate::{ConnectorItem, ConnectorKind, Dataset, ItemQuery, Result};

/// Provider-scoped, read-only access to synchronized connector data.
///
/// Connector tools use this boundary so a provider cannot read another
/// provider's datasets by supplying a foreign dataset identifier.
#[async_trait]
pub trait ConnectorReader: Send + Sync {
    async fn list_datasets_for_kind(&self, kind: ConnectorKind) -> Result<Vec<Dataset>>;

    async fn query_items_for_kind(
        &self,
        kind: ConnectorKind,
        dataset_id: &str,
        query: ItemQuery,
    ) -> Result<Vec<ConnectorItem>>;

    async fn get_item_for_kind(
        &self,
        kind: ConnectorKind,
        dataset_id: &str,
        item_id: &str,
    ) -> Result<Option<ConnectorItem>>;
}
