use std::sync::Arc;

use {
    moltis_channels::{ChannelRegistry, ChannelType},
    moltis_connectors::{
        Account, ConnectorItemInput, Dataset, SourceDisposition, SourceObservation, SyncRun,
    },
    serde_json::json,
    sha2::{Digest, Sha256},
};

use super::{
    ChannelHistoryAccountConfig, ChannelHistoryDatasetConfigView, ChannelSourceView,
    ConnectorManager, ConnectorManagerError, Result, internal, next_sync_at,
    validate_channel_dataset_config,
};

impl ConnectorManager {
    pub fn configure_channel_registry(&self, registry: Arc<ChannelRegistry>) -> Result<()> {
        self.channel_registry.set(registry).map_err(|_| {
            ConnectorManagerError::Internal(anyhow::anyhow!(
                "connector channel registry was initialized more than once"
            ))
        })
    }

    pub async fn list_channel_sources(&self) -> Result<Vec<ChannelSourceView>> {
        let registry = self.channel_registry()?;
        let mut sources = Vec::new();
        for (channel_type, account_id) in registry.all_accounts() {
            let Ok(channel_type) = channel_type.parse::<ChannelType>() else {
                continue;
            };
            if !registry.supports_thread_history(&account_id).await {
                continue;
            }
            sources.push(ChannelSourceView {
                channel_type,
                display_name: format!("{} ({account_id})", channel_type.display_name()),
                account_id,
            });
        }
        sources.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        Ok(sources)
    }

    pub(super) async fn validate_channel_source(
        &self,
        channel_type: ChannelType,
        account_id: &str,
    ) -> Result<()> {
        if account_id.trim().is_empty() {
            return Err(ConnectorManagerError::InvalidInput(
                "channelAccountId must not be empty".to_owned(),
            ));
        }
        if !matches!(
            channel_type,
            ChannelType::Slack | ChannelType::Discord | ChannelType::Matrix | ChannelType::MsTeams
        ) {
            return Err(ConnectorManagerError::InvalidInput(format!(
                "{} does not expose reusable message history",
                channel_type.display_name()
            )));
        }
        let registry = self.channel_registry()?;
        if registry.resolve_channel_type(account_id).as_deref() != Some(channel_type.as_str()) {
            return Err(ConnectorManagerError::InvalidInput(
                "channel account is not active or has a different type".to_owned(),
            ));
        }
        if !registry.supports_thread_history(account_id).await {
            return Err(ConnectorManagerError::InvalidInput(
                "channel account does not support message history".to_owned(),
            ));
        }
        Ok(())
    }

    pub(super) async fn sync_channel_dataset(
        &self,
        dataset: &Dataset,
        run_id: &str,
        account: Account,
    ) -> Result<SyncRun> {
        let account_config: ChannelHistoryAccountConfig = serde_json::from_value(account.config)
            .map_err(|error| internal(error, "deserialize channel connector account"))?;
        self.validate_channel_source(
            account_config.channel_type,
            &account_config.channel_account_id,
        )
        .await?;
        let dataset_config: ChannelHistoryDatasetConfigView =
            serde_json::from_value(dataset.config.clone())
                .map_err(|error| internal(error, "deserialize channel dataset config"))?;
        validate_channel_dataset_config(&dataset_config, dataset.schedule_minutes)?;
        let messages = self
            .channel_registry()?
            .fetch_thread_messages(
                &account_config.channel_account_id,
                &dataset_config.channel_id,
                &dataset_config.thread_id,
                dataset_config.limit,
            )
            .await
            .map_err(ConnectorManagerError::Channel)?;
        let items = messages
            .into_iter()
            .map(|message| {
                if message.message_id.trim().is_empty() {
                    return Err(ConnectorManagerError::Channel(
                        moltis_channels::Error::unavailable(
                            "channel provider returned a message without a stable ID",
                        ),
                    ));
                }
                let body = json!({
                    "channelType": account_config.channel_type,
                    "channelAccountId": account_config.channel_account_id,
                    "channelId": dataset_config.channel_id,
                    "threadId": dataset_config.thread_id,
                    "messageId": message.message_id,
                    "senderId": message.sender_id,
                    "isBot": message.is_bot,
                    "text": message.text,
                    "timestamp": message.timestamp,
                });
                let encoded = serde_json::to_vec(&body)
                    .map_err(|error| internal(error, "serialize channel message"))?;
                Ok(ConnectorItemInput {
                    remote_id: message.message_id,
                    kind: "channel_message".to_owned(),
                    remote_version: None,
                    occurred_at: (!message.timestamp.is_empty()).then_some(message.timestamp),
                    updated_at: None,
                    search_text: format!("{} {}", message.sender_id, message.text),
                    content_hash: format!("{:x}", Sha256::digest(encoded)),
                    body_json: body,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let observations = items
            .iter()
            .map(|item| SourceObservation {
                remote_id: item.remote_id.clone(),
                remote_version: item.remote_version.clone(),
                disposition: SourceDisposition::Included,
                filter_reason: None,
                evaluated_plan_revision: dataset.plan_revision,
            })
            .collect::<Vec<_>>();
        let next_sync_at = next_sync_at(dataset.enabled, dataset.schedule_minutes)?;
        let (_stats, run) = self
            .store
            .commit_sync_snapshot(run_id, &dataset.id, &items, &observations, next_sync_at)
            .await
            .map_err(ConnectorManagerError::from)?;
        if dataset.projections.jsonl || dataset.projections.markdown {
            let projected_dataset = self.dataset_required(&dataset.id).await?;
            self.write_dataset_projection(&projected_dataset).await?;
        }
        Ok(run)
    }

    fn channel_registry(&self) -> Result<&ChannelRegistry> {
        self.channel_registry.get().map(Arc::as_ref).ok_or_else(|| {
            ConnectorManagerError::Unavailable("channel registry is not initialized".to_owned())
        })
    }
}
