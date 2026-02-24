//! Socket Mode connection handler for Slack.
//!
//! Uses slack-morphism's socket mode listener to receive events via WebSocket
//! without requiring a public HTTP endpoint.

use std::sync::Arc;

use {
    secrecy::ExposeSecret,
    slack_morphism::prelude::*,
    tracing::{debug, error, info, warn},
};

use moltis_channels::{
    ChannelEventSink, ChannelMessageMeta, ChannelReplyTarget, ChannelType, Error as ChannelError,
    Result as ChannelResult,
    gating::{DmPolicy, GroupPolicy, MentionMode, is_allowed},
    message_log::{MessageLog, MessageLogEntry},
    plugin::ChannelEvent,
};

use crate::{
    config::SlackAccountConfig,
    markdown::strip_mentions,
    outbound::SlackOutbound,
    state::{AccountState, AccountStateMap},
};

/// Shared state for socket mode callbacks.
#[derive(Clone)]
struct SocketModeState {
    account_id: String,
    config: SlackAccountConfig,
    bot_user_id: Option<String>,
    accounts: AccountStateMap,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
}

/// Start Socket Mode listener for a Slack account.
pub async fn start_socket_mode(
    account_id: String,
    config: SlackAccountConfig,
    accounts: AccountStateMap,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
) -> ChannelResult<()> {
    let connector = SlackClientHyperConnector::new()
        .map_err(|e| ChannelError::external("create slack hyper connector", e))?;
    let client = Arc::new(SlackClient::new(connector));

    // Get bot info
    let token = SlackApiToken::new(config.bot_token.expose_secret().into());
    let session = client.open_session(&token);
    let auth_test = session
        .auth_test()
        .await
        .map_err(|e| ChannelError::external("slack auth.test", e))?;
    let bot_user_id = Some(auth_test.user_id.to_string());

    info!(
        account_id = %account_id,
        bot_user = ?auth_test.user,
        "slack bot authenticated"
    );

    // Create outbound sender
    let outbound = Arc::new(SlackOutbound {
        accounts: Arc::clone(&accounts),
    });

    // Create cancellation token
    let cancel = tokio_util::sync::CancellationToken::new();

    // Store initial state
    {
        let mut accts = accounts.write().unwrap_or_else(|e| e.into_inner());
        accts.insert(account_id.clone(), AccountState {
            client: Arc::clone(&client),
            bot_user_id: bot_user_id.clone(),
            account_id: account_id.clone(),
            config: config.clone(),
            outbound,
            cancel: cancel.clone(),
            message_log: message_log.clone(),
            event_sink: event_sink.clone(),
            pending_threads: std::collections::HashMap::new(),
        });
    }

    // Spawn socket mode listener
    let account_id_clone = account_id.clone();
    let cancel_clone = cancel.clone();

    tokio::spawn(async move {
        let result = run_socket_listener(
            account_id_clone.clone(),
            config,
            client,
            bot_user_id,
            accounts.clone(),
            message_log,
            event_sink,
            cancel_clone,
        )
        .await;

        if let Err(e) = result {
            error!(
                account_id = %account_id_clone,
                error = %e,
                "socket mode listener failed"
            );
        }

        // Clean up on exit
        let mut accts = accounts.write().unwrap_or_else(|e| e.into_inner());
        accts.remove(&account_id_clone);
    });

    Ok(())
}

async fn run_socket_listener(
    account_id: String,
    config: SlackAccountConfig,
    client: Arc<SlackClient<SlackClientHyperConnector<SlackHyperHttpsConnector>>>,
    bot_user_id: Option<String>,
    accounts: AccountStateMap,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
    cancel: tokio_util::sync::CancellationToken,
) -> ChannelResult<()> {
    let app_token = SlackApiToken::new(config.app_token.expose_secret().into());

    // Create shared state for callbacks
    let state = SocketModeState {
        account_id: account_id.clone(),
        config,
        bot_user_id,
        accounts,
        message_log,
        event_sink,
    };

    // Create socket mode callbacks
    let callbacks = SlackSocketModeListenerCallbacks::new()
        .with_push_events(handle_push_events)
        .with_command_events(handle_command_events)
        .with_interaction_events(handle_interaction_events);

    // Create socket mode listener environment
    let listener_env =
        Arc::new(SlackClientEventsListenerEnvironment::new(client.clone()).with_user_state(state));

    // Create and run the listener
    let socket_listener = SlackClientSocketModeListener::new(
        &SlackClientSocketModeConfig::new(),
        listener_env,
        callbacks,
    );

    tokio::select! {
        result = socket_listener.listen_for(&app_token) => {
            if let Err(e) = result {
                error!(account_id = %account_id, error = %e, "socket mode error");
            }
        }
        _ = cancel.cancelled() => {
            info!(account_id = %account_id, "socket mode cancelled");
        }
    }

    Ok(())
}

/// Handle push events (messages, etc.)
type CallbackResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn handle_push_events(
    event: SlackPushEventCallback,
    _client: Arc<SlackHyperClient>,
    states: SlackClientEventsUserState,
) -> CallbackResult<()> {
    let guard = states.read().await;
    let state = guard
        .get_user_state::<SocketModeState>()
        .ok_or_else(|| std::io::Error::other("missing socket mode state"))?;

    if let Err(e) = handle_push_event_inner(state, event).await {
        warn!(
            account_id = %state.account_id,
            error = %e,
            "failed to handle push event"
        );
    }

    Ok(())
}

/// Handle command events (slash commands)
async fn handle_command_events(
    event: SlackCommandEvent,
    _client: Arc<SlackHyperClient>,
    states: SlackClientEventsUserState,
) -> CallbackResult<SlackCommandEventResponse> {
    let guard = states.read().await;
    let state = guard
        .get_user_state::<SocketModeState>()
        .ok_or_else(|| std::io::Error::other("missing socket mode state"))?;

    debug!(
        account_id = %state.account_id,
        command = ?event.command,
        "received slash command"
    );

    // Handle moltis commands
    if let Some(sink) = &state.event_sink {
        let command = event.command.to_string();
        let command = command.trim_start_matches('/');
        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::Slack,
            account_id: state.account_id.clone(),
            chat_id: event.channel_id.to_string(),
            message_id: None,
        };

        if let Ok(response) = sink.dispatch_command(command, reply_to).await {
            return Ok(SlackCommandEventResponse::new(
                SlackMessageContent::new().with_text(response),
            ));
        }
    }

    Ok(SlackCommandEventResponse::new(
        SlackMessageContent::new().with_text("Command received".into()),
    ))
}

/// Handle interaction events (buttons, menus)
async fn handle_interaction_events(
    _event: SlackInteractionEvent,
    _client: Arc<SlackHyperClient>,
    states: SlackClientEventsUserState,
) -> CallbackResult<()> {
    let guard = states.read().await;
    if let Some(state) = guard.get_user_state::<SocketModeState>() {
        debug!(account_id = %state.account_id, "received interaction event");
    }
    Ok(())
}

async fn handle_push_event_inner(
    state: &SocketModeState,
    event: SlackPushEventCallback,
) -> ChannelResult<()> {
    match &event.event {
        SlackEventCallbackBody::Message(msg) => {
            handle_message_event(state, msg).await?;
        },
        _ => {
            debug!(account_id = %state.account_id, "ignoring event callback type");
        },
    }
    Ok(())
}

async fn handle_message_event(
    state: &SocketModeState,
    event: &SlackMessageEvent,
) -> ChannelResult<()> {
    // Skip bot messages to prevent loops
    if event.sender.bot_id.is_some() {
        return Ok(());
    }

    // Skip message subtypes (edits, deletes, etc.)
    if event.subtype.is_some() {
        return Ok(());
    }

    let channel_id = event
        .origin
        .channel
        .as_ref()
        .map(|c| c.to_string())
        .unwrap_or_default();
    let user_id = event.sender.user.as_ref().map(|u| u.to_string());
    let text = event
        .content
        .as_ref()
        .and_then(|c| c.text.clone())
        .unwrap_or_default();
    let thread_ts = event.origin.thread_ts.clone();

    // Determine if DM or channel
    let is_dm = channel_id.starts_with('D');

    // Check if bot is mentioned
    let is_mention = state
        .bot_user_id
        .as_ref()
        .is_some_and(|bot_id| text.contains(&format!("<@{bot_id}>")));

    // Access control
    let access_granted = check_access(state, &user_id, &channel_id, is_dm);

    // Log message
    if let Some(log) = &state.message_log {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let entry = MessageLogEntry {
            id: 0,
            account_id: state.account_id.clone(),
            channel_type: ChannelType::Slack.to_string(),
            peer_id: user_id.clone().unwrap_or_else(|| "unknown".into()),
            username: None,
            sender_name: None,
            chat_id: channel_id.clone(),
            chat_type: if is_dm {
                "dm"
            } else {
                "channel"
            }
            .into(),
            body: text.clone(),
            access_granted,
            created_at: now,
        };
        let _ = log.log(entry).await;
    }

    // Emit channel event
    if let Some(sink) = &state.event_sink {
        sink.emit(ChannelEvent::InboundMessage {
            channel_type: ChannelType::Slack,
            account_id: state.account_id.clone(),
            peer_id: user_id.clone().unwrap_or_default(),
            username: None,
            sender_name: None,
            message_count: None,
            access_granted,
        })
        .await;
    }

    if !access_granted {
        return Ok(());
    }

    // Check activation mode
    let should_respond = match state.config.mention_mode {
        MentionMode::Mention => is_dm || is_mention,
        MentionMode::Always => true,
        MentionMode::None => is_dm,
    };

    if !should_respond {
        return Ok(());
    }

    // Strip mentions and clean up text
    let clean_text = strip_mentions(&text, state.bot_user_id.as_deref());

    if clean_text.is_empty() {
        return Ok(());
    }

    // Store thread timestamp for replies
    if state.config.thread_replies
        && let Some(ts) = thread_ts.or_else(|| Some(event.origin.ts.clone()))
    {
        let mut accts = state.accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(account_state) = accts.get_mut(&state.account_id) {
            account_state
                .pending_threads
                .insert(channel_id.clone(), ts.to_string());
        }
    }

    // Build reply target
    let reply_to = ChannelReplyTarget {
        channel_type: ChannelType::Slack,
        account_id: state.account_id.clone(),
        chat_id: channel_id,
        message_id: None,
    };

    let meta = ChannelMessageMeta {
        channel_type: ChannelType::Slack,
        sender_name: None,
        username: user_id,
        message_kind: None,
        model: state.config.model.clone(),
        audio_filename: None,
    };

    // Dispatch to chat
    if let Some(sink) = &state.event_sink {
        sink.dispatch_to_chat(&clean_text, reply_to, meta).await;
    }

    Ok(())
}

fn check_access(
    state: &SocketModeState,
    user_id: &Option<String>,
    channel_id: &str,
    is_dm: bool,
) -> bool {
    if is_dm {
        match state.config.dm_policy {
            DmPolicy::Open => true,
            DmPolicy::Allowlist => user_id
                .as_ref()
                .is_some_and(|uid| is_allowed(uid, &state.config.user_allowlist)),
            DmPolicy::Disabled => false,
        }
    } else {
        match state.config.channel_policy {
            GroupPolicy::Open => true,
            GroupPolicy::Allowlist => is_allowed(channel_id, &state.config.channel_allowlist),
            GroupPolicy::Disabled => false,
        }
    }
}
