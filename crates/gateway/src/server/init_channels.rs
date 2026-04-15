use std::{collections::HashSet, sync::Arc};

use tracing::info;

use moltis_channels::ChannelPlugin;

use crate::services::GatewayServices;

/// Return type for channel initialization, carrying handles that the caller
/// needs to store in gateway state or pass to other init phases.
pub(crate) struct ChannelInitResult {
    pub(crate) services: GatewayServices,
    pub(crate) msteams_webhook_plugin: Arc<tokio::sync::RwLock<moltis_msteams::MsTeamsPlugin>>,
    #[cfg(feature = "slack")]
    pub(crate) slack_webhook_plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>,
}

/// Wire the channel store, channel registry, and all channel plugins.
///
/// Extracted from `prepare_gateway_core` for readability.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn init_channels(
    mut services: GatewayServices,
    config: &moltis_config::MoltisConfig,
    db_pool: sqlx::SqlitePool,
    #[cfg(feature = "vault")] vault: Option<Arc<moltis_vault::Vault>>,
    message_log: Arc<dyn moltis_channels::message_log::MessageLog>,
    session_metadata: Arc<moltis_sessions::metadata::SqliteSessionMetadata>,
    deferred_state: Arc<tokio::sync::OnceCell<Arc<crate::state::GatewayState>>>,
    data_dir: &std::path::Path,
) -> ChannelInitResult {
    #[cfg(not(feature = "whatsapp"))]
    let _ = data_dir;

    use moltis_channels::{
        registry::{ChannelRegistry, RegistryOutboundRouter},
        store::ChannelStore,
    };

    #[cfg(feature = "vault")]
    let channel_store: Arc<dyn ChannelStore> = {
        let inner: Arc<dyn ChannelStore> = Arc::new(crate::channel_store::SqliteChannelStore::new(
            db_pool.clone(),
        ));
        Arc::new(crate::channel_store::VaultChannelStore::new(
            inner,
            vault.clone(),
        ))
    };
    #[cfg(not(feature = "vault"))]
    let channel_store: Arc<dyn ChannelStore> = Arc::new(
        crate::channel_store::SqliteChannelStore::new(db_pool.clone()),
    );

    let channel_sink: Arc<dyn moltis_channels::ChannelEventSink> = Arc::new(
        crate::channel_events::GatewayChannelEventSink::new(Arc::clone(&deferred_state)),
    );

    // Create plugins and register with the registry.
    let mut registry = ChannelRegistry::new();

    let tg_plugin = Arc::new(tokio::sync::RwLock::new(
        moltis_telegram::TelegramPlugin::new()
            .with_message_log(Arc::clone(&message_log))
            .with_event_sink(Arc::clone(&channel_sink)),
    ));
    registry
        .register(tg_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
        .await;

    let msteams_plugin = Arc::new(tokio::sync::RwLock::new(
        moltis_msteams::MsTeamsPlugin::new()
            .with_message_log(Arc::clone(&message_log))
            .with_event_sink(Arc::clone(&channel_sink)),
    ));
    let msteams_webhook_plugin = Arc::clone(&msteams_plugin);
    registry
        .register(msteams_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
        .await;

    let discord_plugin = Arc::new(tokio::sync::RwLock::new(
        moltis_discord::DiscordPlugin::new()
            .with_message_log(Arc::clone(&message_log))
            .with_event_sink(Arc::clone(&channel_sink)),
    ));
    registry
        .register(discord_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
        .await;

    #[cfg(feature = "matrix")]
    {
        let matrix_plugin = Arc::new(tokio::sync::RwLock::new(
            moltis_matrix::MatrixPlugin::new()
                .with_message_log(Arc::clone(&message_log))
                .with_event_sink(Arc::clone(&channel_sink)),
        ));
        registry
            .register(matrix_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
            .await;
    }

    #[cfg(feature = "nostr")]
    {
        let nostr_plugin = Arc::new(tokio::sync::RwLock::new(
            moltis_nostr::NostrPlugin::new()
                .with_message_log(Arc::clone(&message_log))
                .with_event_sink(Arc::clone(&channel_sink)),
        ));
        registry
            .register(nostr_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
            .await;
    }

    #[cfg(feature = "whatsapp")]
    {
        let wa_data_dir = data_dir.join("whatsapp");
        if let Err(e) = std::fs::create_dir_all(&wa_data_dir) {
            tracing::warn!("failed to create whatsapp data dir: {e}");
        }
        let whatsapp_plugin = Arc::new(tokio::sync::RwLock::new(
            moltis_whatsapp::WhatsAppPlugin::new(wa_data_dir)
                .with_message_log(Arc::clone(&message_log))
                .with_event_sink(Arc::clone(&channel_sink)),
        ));
        registry
            .register(whatsapp_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
            .await;
    }
    #[cfg(not(feature = "whatsapp"))]
    let _ = &channel_sink; // silence unused warning

    #[cfg(feature = "slack")]
    let slack_webhook_plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>;
    #[cfg(feature = "slack")]
    {
        let slack_plugin = Arc::new(tokio::sync::RwLock::new(
            moltis_slack::SlackPlugin::new()
                .with_message_log(Arc::clone(&message_log))
                .with_event_sink(Arc::clone(&channel_sink)),
        ));
        slack_webhook_plugin = Arc::clone(&slack_plugin);
        registry
            .register(slack_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
            .await;
    }

    // Collect all channel accounts to start (config + stored), then
    // spawn them concurrently so slow network calls (e.g. Telegram)
    // don't block startup sequentially.
    let mut pending_starts: Vec<(String, String, serde_json::Value)> = Vec::new();
    let mut queued: HashSet<(String, String)> = HashSet::new();

    for (channel_type, accounts) in config.channels.all_channel_configs() {
        if registry.get(channel_type).is_none() {
            if !accounts.is_empty() {
                tracing::debug!(
                    channel_type,
                    "skipping config — no plugin registered for this channel type"
                );
            }
            continue;
        }
        for (account_id, account_config) in accounts {
            let key = (channel_type.to_string(), account_id.clone());
            if queued.insert(key) {
                pending_starts.push((
                    channel_type.to_string(),
                    account_id.clone(),
                    account_config.clone(),
                ));
            }
        }
    }

    // Load persisted channels that were not queued from config.
    match channel_store.list().await {
        Ok(stored) => {
            info!("{} stored channel(s) found in database", stored.len());
            for ch in stored {
                let key = (ch.channel_type.clone(), ch.account_id.clone());
                if queued.contains(&key) {
                    info!(
                        account_id = ch.account_id,
                        channel_type = ch.channel_type,
                        "skipping stored channel (already started from config)"
                    );
                    continue;
                }
                if registry.get(&ch.channel_type).is_none() {
                    tracing::warn!(
                        account_id = ch.account_id,
                        channel_type = ch.channel_type,
                        "unsupported channel type, skipping stored account"
                    );
                    continue;
                }
                info!(
                    account_id = ch.account_id,
                    channel_type = ch.channel_type,
                    "starting stored channel"
                );
                if queued.insert(key) {
                    pending_starts.push((ch.channel_type, ch.account_id, ch.config));
                }
            }
        },
        Err(e) => tracing::warn!("failed to load stored channels: {e}"),
    }

    let registry = Arc::new(registry);

    // Spawn all channel starts concurrently.
    if !pending_starts.is_empty() {
        let total = pending_starts.len();
        info!("{total} channel account(s) queued for startup");
        for (channel_type, account_id, account_config) in pending_starts {
            let reg = Arc::clone(&registry);
            tokio::spawn(async move {
                if let Err(e) = reg
                    .start_account(&channel_type, &account_id, account_config)
                    .await
                {
                    tracing::warn!(
                        account_id,
                        channel_type,
                        "failed to start channel account: {e}"
                    );
                } else {
                    info!(account_id, channel_type, "channel account started");
                }
            });
        }
    }
    let router = Arc::new(RegistryOutboundRouter::new(Arc::clone(&registry)));

    services = services.with_channel_registry(Arc::clone(&registry));
    services = services.with_channel_store(Arc::clone(&channel_store));
    let outbound_router = Arc::clone(&router) as Arc<dyn moltis_channels::ChannelOutbound>;
    services = services.with_channel_outbound(Arc::clone(&outbound_router));
    services = services
        .with_channel_stream_outbound(router as Arc<dyn moltis_channels::ChannelStreamOutbound>);

    services.channel = Arc::new(crate::channel::LiveChannelService::new(
        registry,
        outbound_router,
        channel_store,
        Arc::clone(&message_log),
        Arc::clone(&session_metadata),
    ));

    ChannelInitResult {
        services,
        msteams_webhook_plugin,
        #[cfg(feature = "slack")]
        slack_webhook_plugin,
    }
}
