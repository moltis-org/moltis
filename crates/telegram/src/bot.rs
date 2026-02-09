use std::sync::Arc;

use {
    secrecy::ExposeSecret,
    teloxide::{
        ApiError, RequestError,
        prelude::*,
        types::{AllowedUpdate, BotCommand, UpdateKind},
    },
    tokio_util::sync::CancellationToken,
    tracing::{debug, error, info, warn},
};

use moltis_channels::{ChannelEventSink, message_log::MessageLog};

use crate::{
    config::TelegramAccountConfig,
    handlers,
    outbound::TelegramOutbound,
    state::{AccountState, AccountStateMap},
};

/// Start polling for a single bot account.
///
/// Spawns a background task that processes updates until the returned
/// `CancellationToken` is cancelled.
pub async fn start_polling(
    account_id: String,
    config: TelegramAccountConfig,
    accounts: AccountStateMap,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
) -> anyhow::Result<CancellationToken> {
    // Build bot with a client timeout longer than the long-polling timeout (30s)
    // so the HTTP client doesn't abort the request before Telegram responds.
    let client = teloxide::net::default_reqwest_settings()
        .timeout(std::time::Duration::from_secs(45))
        .build()?;
    let bot = teloxide::Bot::with_client(config.token.expose_secret(), client);

    // Verify credentials and get bot username.
    let me = bot.get_me().await?;
    let bot_username = me.username.clone();

    // Delete any existing webhook so long polling works.
    bot.delete_webhook().send().await?;

    // Register slash commands for autocomplete in Telegram clients.
    let commands = vec![
        BotCommand::new("new", "Start a new session"),
        BotCommand::new("sessions", "List and switch sessions"),
        BotCommand::new("model", "Switch provider/model"),
        BotCommand::new("sandbox", "Toggle sandbox and choose image"),
        BotCommand::new("clear", "Clear session history"),
        BotCommand::new("compact", "Compact session (summarize)"),
        BotCommand::new("context", "Show session context info"),
        BotCommand::new("help", "Show available commands"),
    ];
    if let Err(e) = bot.set_my_commands(commands).await {
        warn!(account_id, "failed to register bot commands: {e}");
    }

    info!(
        account_id,
        username = ?bot_username,
        "telegram bot connected (webhook cleared)"
    );

    let cancel = CancellationToken::new();

    let outbound = Arc::new(TelegramOutbound {
        accounts: Arc::clone(&accounts),
    });

    let otp_cooldown = config.otp_cooldown_secs;
    let state = AccountState {
        bot: bot.clone(),
        bot_username,
        account_id: account_id.clone(),
        config,
        outbound,
        cancel: cancel.clone(),
        message_log,
        event_sink,
        otp: std::sync::Mutex::new(crate::otp::OtpState::new(otp_cooldown)),
    };

    {
        let mut map = accounts.write().unwrap();
        map.insert(account_id.clone(), state);
    }

    let cancel_clone = cancel.clone();
    let aid = account_id.clone();
    let poll_accounts = Arc::clone(&accounts);
    tokio::spawn(async move {
        info!(account_id = aid, "starting telegram manual polling loop");
        let mut offset: i32 = 0;

        loop {
            if cancel_clone.is_cancelled() {
                info!(account_id = aid, "telegram polling stopped");
                break;
            }

            let result = bot
                .get_updates()
                .offset(offset)
                .timeout(30)
                .allowed_updates(vec![
                    AllowedUpdate::Message,
                    AllowedUpdate::EditedMessage,
                    AllowedUpdate::CallbackQuery,
                ])
                .await;

            match result {
                Ok(updates) => {
                    debug!(
                        account_id = aid,
                        count = updates.len(),
                        "got telegram updates"
                    );
                    for update in updates {
                        offset = update.id.as_offset();
                        match update.kind {
                            UpdateKind::Message(msg) => {
                                debug!(
                                    account_id = aid,
                                    chat_id = msg.chat.id.0,
                                    "received telegram message"
                                );
                                if let Err(e) =
                                    handlers::handle_message_direct(msg, &bot, &aid, &poll_accounts)
                                        .await
                                {
                                    error!(
                                        account_id = aid,
                                        error = %e,
                                        "error handling telegram message"
                                    );
                                }
                            },
                            UpdateKind::EditedMessage(msg) => {
                                debug!(
                                    account_id = aid,
                                    chat_id = msg.chat.id.0,
                                    "received telegram edited message"
                                );
                                if let Err(e) =
                                    handlers::handle_edited_location(msg, &aid, &poll_accounts)
                                        .await
                                {
                                    error!(
                                        account_id = aid,
                                        error = %e,
                                        "error handling telegram edited message"
                                    );
                                }
                            },
                            UpdateKind::CallbackQuery(query) => {
                                debug!(
                                    account_id = aid,
                                    callback_data = ?query.data,
                                    "received telegram callback query"
                                );
                                if let Err(e) = handlers::handle_callback_query(
                                    query,
                                    &bot,
                                    &aid,
                                    &poll_accounts,
                                )
                                .await
                                {
                                    error!(
                                        account_id = aid,
                                        error = %e,
                                        "error handling telegram callback query"
                                    );
                                }
                            },
                            other => {
                                debug!(account_id = aid, "ignoring non-message update: {other:?}");
                            },
                        }
                    }
                },
                Err(e) => {
                    // Detect conflict error: another bot instance is running with the same token.
                    let is_conflict =
                        matches!(&e, RequestError::Api(ApiError::TerminatedByOtherGetUpdates));

                    if is_conflict {
                        warn!(
                            account_id = aid,
                            "telegram bot disabled: another instance is already running with this token"
                        );

                        // Request the gateway to disable this channel.
                        let event_sink = {
                            let accounts = poll_accounts.read().unwrap();
                            accounts.get(&aid).and_then(|s| s.event_sink.clone())
                        };
                        if let Some(sink) = event_sink {
                            sink.request_disable_account(
                                "telegram",
                                &aid,
                                "Another bot instance is already running with this token",
                            )
                            .await;
                        }

                        // Cancel this polling loop and exit.
                        cancel_clone.cancel();
                        break;
                    }

                    warn!(account_id = aid, error = %e, "telegram getUpdates failed");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                },
            }
        }
    });

    Ok(cancel)
}
