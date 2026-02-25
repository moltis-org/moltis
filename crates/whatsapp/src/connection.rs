use std::sync::Arc;

use {
    tokio_util::sync::CancellationToken,
    tracing::{info, warn},
};

use moltis_channels::{ChannelEventSink, message_log::MessageLog};

use crate::{
    config::WhatsAppAccountConfig,
    handlers,
    state::{AccountState, AccountStateMap},
};

/// Start a WhatsApp connection for the given account.
///
/// Builds the `Bot` with a persistent sled store, registers the event handler,
/// and spawns the event loop as a background tokio task. Session state persists
/// across restarts so the user does not need to re-scan the QR code.
pub async fn start_connection(
    account_id: String,
    config: WhatsAppAccountConfig,
    accounts: AccountStateMap,
    data_dir: std::path::PathBuf,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
) -> crate::Result<()> {
    // Use persistent sled store at <data_dir>/whatsapp/<account_id>/.
    let store_path = config
        .store_path
        .clone()
        .unwrap_or_else(|| data_dir.join("whatsapp").join(&account_id));

    info!(account_id = %account_id, path = %store_path.display(), "opening sled WhatsApp store");

    let backend = Arc::new(
        crate::sled_store::SledStore::open(&store_path).map_err(|e| crate::Error::Store {
            message: format!("failed to open sled store at {}: {e}", store_path.display()),
        })?,
    );

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    // Build the bot.
    let account_id_clone = account_id.clone();
    let event_sink_clone = event_sink.clone();
    let message_log_clone = message_log.clone();

    // We need to create a temporary accounts ref for the state that will be
    // populated after bot.build().
    let state_ref: Arc<tokio::sync::OnceCell<Arc<AccountState>>> =
        Arc::new(tokio::sync::OnceCell::new());
    let state_ref_handler = Arc::clone(&state_ref);
    let accounts_handler = Arc::clone(&accounts);

    let mut bot = whatsapp_rust::bot::Bot::builder()
        .with_backend(backend)
        .with_transport_factory(
            whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory::new(),
        )
        .with_http_client(whatsapp_rust_ureq_http_client::UreqHttpClient::new())
        .on_event(move |event, client| {
            let state_ref = Arc::clone(&state_ref_handler);
            let accounts = Arc::clone(&accounts_handler);
            async move {
                if let Some(state) = state_ref.get() {
                    handlers::handle_event(event, client, Arc::clone(state), accounts).await;
                }
            }
        })
        .build()
        .await
        .map_err(|e| crate::Error::Whatsapp {
            message: e.to_string(),
        })?;

    let client = bot.client();

    // Create account state.
    let otp_cooldown = config.otp_cooldown_secs;
    let account_state = Arc::new(AccountState {
        client: Arc::clone(&client),
        account_id: account_id_clone.clone(),
        config,
        cancel: cancel_clone,
        message_log: message_log_clone,
        event_sink: event_sink_clone,
        latest_qr: std::sync::RwLock::new(None),
        connected: std::sync::atomic::AtomicBool::new(false),
        otp: std::sync::Mutex::new(crate::otp::OtpState::new(otp_cooldown)),
        recent_sent_ids: std::sync::Mutex::new(std::collections::VecDeque::new()),
    });

    // Populate the OnceCell so the event handler can access state.
    let _ = state_ref.set(Arc::clone(&account_state));

    // Insert into the shared map.
    {
        let mut map = accounts.write().unwrap_or_else(|e| e.into_inner());
        map.insert(account_id.clone(), AccountState {
            client: Arc::clone(&client),
            account_id: account_id.clone(),
            config: account_state.config.clone(),
            cancel: cancel.clone(),
            message_log: account_state.message_log.clone(),
            event_sink: account_state.event_sink.clone(),
            latest_qr: std::sync::RwLock::new(None),
            connected: std::sync::atomic::AtomicBool::new(false),
            otp: std::sync::Mutex::new(crate::otp::OtpState::new(otp_cooldown)),
            recent_sent_ids: std::sync::Mutex::new(std::collections::VecDeque::new()),
        });
    }

    // Spawn the event loop.
    let account_id_task = account_id.clone();
    tokio::spawn(async move {
        tokio::select! {
            result = bot.run() => {
                match result {
                    Ok(handle) => {
                        tokio::select! {
                            _ = handle => {
                                info!(account_id = %account_id_task, "WhatsApp bot task completed");
                            },
                            _ = cancel.cancelled() => {
                                info!(account_id = %account_id_task, "WhatsApp account cancelled");
                            },
                        }
                    },
                    Err(e) => {
                        warn!(account_id = %account_id_task, error = %e, "WhatsApp bot failed to start");
                    },
                }
            },
            _ = cancel.cancelled() => {
                info!(account_id = %account_id_task, "WhatsApp account cancelled before start");
            },
        }
    });

    Ok(())
}
