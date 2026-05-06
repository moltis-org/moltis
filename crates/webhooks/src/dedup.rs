//! Delivery deduplication.
//!
//! Deduplication is handled by the store's `find_by_delivery_key` method.
//! This module provides the dedup check used by the ingress handler.

use crate::{Result, store::WebhookStore};

/// Check if a delivery with this key already exists for the given webhook.
/// Returns `Some(delivery_id)` if duplicate, `None` if new.
pub async fn check_duplicate(
    store: &dyn WebhookStore,
    webhook_id: i64,
    delivery_key: Option<&str>,
) -> Result<Option<i64>> {
    match delivery_key {
        Some(key) if !key.is_empty() => store.find_by_delivery_key(webhook_id, key).await,
        _ => Ok(None),
    }
}
