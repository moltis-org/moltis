use std::sync::Arc;

use {
    secrecy::ExposeSecret,
    slack_morphism::prelude::*,
    tracing::{debug, info, warn},
};

use moltis_channels::{
    config_view::ChannelConfigView,
    gating::{DmPolicy, GroupPolicy, is_allowed},
    message_log::MessageLogEntry,
    plugin::{
        ChannelEvent, ChannelMessageKind, ChannelMessageMeta, ChannelReplyTarget, ChannelType,
    },
};

use crate::{
    config::SlackAccountConfig,
    markdown::strip_mentions,
    state::{AccountState, AccountStateMap},
};

/// State stored in the Socket Mode listener for callback access.
#[derive(Clone)]
struct ListenerState {
    account_id: String,
    accounts: AccountStateMap,
}

/// Start Socket Mode for a single account.
///
/// Creates a `SlackClient`, calls `auth.test` to verify the bot token and
/// obtain the bot user ID, stores state, then spawns the socket listener.
pub async fn start_socket_mode(
    account_id: &str,
    config: SlackAccountConfig,
    accounts: AccountStateMap,
    message_log: Option<Arc<dyn moltis_channels::message_log::MessageLog>>,
    event_sink: Option<Arc<dyn moltis_channels::ChannelEventSink>>,
) -> moltis_channels::Result<()> {
    let bot_token_str = config.bot_token.expose_secret().clone();
    let app_token_str = config.app_token.expose_secret().clone();

    if bot_token_str.is_empty() {
        return Err(moltis_channels::Error::invalid_input(
            "Slack bot_token is required",
        ));
    }
    if app_token_str.is_empty() {
        return Err(moltis_channels::Error::invalid_input(
            "Slack app_token is required for Socket Mode",
        ));
    }

    let client = Arc::new(SlackClient::new(SlackClientHyperConnector::new().map_err(
        |e| moltis_channels::Error::unavailable(format!("hyper connector: {e}")),
    )?));

    // Verify the bot token and get the bot user ID.
    let bot_token = SlackApiToken::new(SlackApiTokenValue::from(bot_token_str));
    let session = client.open_session(&bot_token);
    let auth_response = session
        .auth_test()
        .await
        .map_err(|e| moltis_channels::Error::unavailable(format!("auth.test failed: {e}")))?;

    let bot_user_id = auth_response.user_id.to_string();
    info!(account_id, bot_user_id, "slack bot authenticated");

    let cancel = tokio_util::sync::CancellationToken::new();

    {
        let mut accts = accounts.write().unwrap_or_else(|e| e.into_inner());
        accts.insert(account_id.to_string(), AccountState {
            account_id: account_id.to_string(),
            config,
            message_log,
            event_sink,
            cancel: cancel.clone(),
            bot_user_id: Some(bot_user_id),
            pending_threads: std::collections::HashMap::new(),
        });
    }

    // Spawn the socket listener.
    let accounts_for_task = Arc::clone(&accounts);
    let account_id_owned = account_id.to_string();
    let cancel_for_task = cancel.clone();
    let app_token = SlackApiToken::new(SlackApiTokenValue::from(app_token_str));

    tokio::spawn(async move {
        if let Err(e) = run_socket_listener(
            &account_id_owned,
            client,
            app_token,
            accounts_for_task,
            cancel_for_task,
        )
        .await
        {
            warn!(
                account_id = %account_id_owned,
                "slack socket mode listener stopped: {e}"
            );
        }
    });

    Ok(())
}

/// Run the Socket Mode listener until cancelled.
async fn run_socket_listener(
    account_id: &str,
    client: Arc<SlackClient<SlackClientHyperHttpsConnector>>,
    app_token: SlackApiToken,
    accounts: AccountStateMap,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener_state = ListenerState {
        account_id: account_id.to_string(),
        accounts,
    };

    // Socket Mode callbacks — must be function pointers, so we use user state.
    let callbacks = SlackSocketModeListenerCallbacks::new()
        .with_push_events(push_events_callback)
        .with_command_events(command_events_callback)
        .with_interaction_events(interaction_events_callback);

    let listener_environment = Arc::new(
        SlackClientEventsListenerEnvironment::new(Arc::clone(&client))
            .with_error_handler(error_handler)
            .with_user_state(listener_state),
    );

    let config = SlackClientSocketModeConfig::new();
    let socket_listener =
        SlackClientSocketModeListener::new(&config, listener_environment, callbacks);

    socket_listener.listen_for(&app_token).await?;

    info!(account_id, "slack socket mode listener started");

    tokio::select! {
        () = cancel.cancelled() => {
            info!(account_id, "slack socket mode shutting down");
            socket_listener.shutdown().await;
        }
        _code = socket_listener.serve() => {
            warn!(account_id, "slack socket mode listener unexpectedly stopped");
        }
    }

    Ok(())
}

/// Error handler for Socket Mode.
fn error_handler(
    err: Box<dyn std::error::Error + Send + Sync>,
    _client: Arc<SlackClient<SlackClientHyperHttpsConnector>>,
    _states: SlackClientEventsUserState,
) -> HttpStatusCode {
    warn!("slack socket mode error: {err}");
    HttpStatusCode::OK
}

/// Push events callback (messages, app_mention, etc.).
async fn push_events_callback(
    event: SlackPushEventCallback,
    _client: Arc<SlackClient<SlackClientHyperHttpsConnector>>,
    states: SlackClientEventsUserState,
) -> UserCallbackResult<()> {
    let guard = states.read().await;
    let listener_state = match guard.get_user_state::<ListenerState>() {
        Some(s) => s.clone(),
        None => return Ok(()),
    };
    drop(guard);

    match event.event {
        SlackEventCallbackBody::Message(msg_event) => {
            handle_message_event(
                &listener_state.account_id,
                msg_event,
                &listener_state.accounts,
            )
            .await;
        },
        SlackEventCallbackBody::AppMention(mention_event) => {
            let channel = mention_event.channel.to_string();
            let user = mention_event.user.to_string();
            let text = mention_event.content.text.as_deref().unwrap_or("");
            let thread_ts = mention_event
                .origin
                .thread_ts
                .as_ref()
                .map(|ts| ts.to_string());

            handle_inbound(
                &listener_state.account_id,
                &channel,
                &user,
                text,
                thread_ts,
                None,
                true, // is_mention
                &listener_state.accounts,
            )
            .await;
        },
        SlackEventCallbackBody::ReactionAdded(reaction_event) => {
            handle_reaction_event(
                &listener_state.account_id,
                reaction_event.user.as_ref(),
                reaction_event.reaction.as_ref(),
                &reaction_event.item,
                true,
                &listener_state.accounts,
            )
            .await;
        },
        SlackEventCallbackBody::ReactionRemoved(reaction_event) => {
            handle_reaction_event(
                &listener_state.account_id,
                reaction_event.user.as_ref(),
                reaction_event.reaction.as_ref(),
                &reaction_event.item,
                false,
                &listener_state.accounts,
            )
            .await;
        },
        _ => {
            debug!("unhandled slack push event");
        },
    }

    Ok(())
}

/// Command events callback (slash commands).
async fn command_events_callback(
    event: SlackCommandEvent,
    _client: Arc<SlackClient<SlackClientHyperHttpsConnector>>,
    states: SlackClientEventsUserState,
) -> UserCallbackResult<SlackCommandEventResponse> {
    let guard = states.read().await;
    let listener_state = match guard.get_user_state::<ListenerState>() {
        Some(s) => s.clone(),
        None => {
            return Ok(SlackCommandEventResponse::new(
                SlackMessageContent::new().with_text("Not configured".to_string()),
            ));
        },
    };
    drop(guard);

    let account_id = &listener_state.account_id;
    let command_text = event.command.to_string();
    let text = event.text.unwrap_or_default();
    let full_command = format!("{command_text} {text}").trim().to_string();

    let event_sink = {
        let accts = listener_state
            .accounts
            .read()
            .unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.event_sink.clone())
    };

    if let Some(sink) = event_sink {
        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::Slack,
            account_id: account_id.to_string(),
            chat_id: event.channel_id.to_string(),
            message_id: None,
        };
        match sink.dispatch_command(&full_command, reply_to).await {
            Ok(response_text) => Ok(SlackCommandEventResponse::new(
                SlackMessageContent::new().with_text(response_text),
            )),
            Err(e) => Ok(SlackCommandEventResponse::new(
                SlackMessageContent::new().with_text(format!("Error: {e}")),
            )),
        }
    } else {
        Ok(SlackCommandEventResponse::new(
            SlackMessageContent::new().with_text("Channel not configured".to_string()),
        ))
    }
}

/// Interaction events callback (block actions / button clicks).
async fn interaction_events_callback(
    event: SlackInteractionEvent,
    _client: Arc<SlackClient<SlackClientHyperHttpsConnector>>,
    states: SlackClientEventsUserState,
) -> UserCallbackResult<()> {
    let guard = states.read().await;
    let listener_state = match guard.get_user_state::<ListenerState>() {
        Some(s) => s.clone(),
        None => return Ok(()),
    };
    drop(guard);

    // Extract the action_id from block_actions interaction type.
    let (action_id, channel_id) = match &event {
        SlackInteractionEvent::BlockActions(ba) => {
            let action = ba.actions.as_ref().and_then(|a| a.first());
            let channel = ba.channel.as_ref().map(|c| c.id.to_string());
            match (action, channel) {
                (Some(act), Some(ch)) => (act.action_id.to_string(), ch),
                _ => {
                    debug!("block_actions missing action or channel");
                    return Ok(());
                },
            }
        },
        _ => {
            debug!("unhandled interaction event type");
            return Ok(());
        },
    };

    let account_id = &listener_state.account_id;
    let event_sink = {
        let accts = listener_state
            .accounts
            .read()
            .unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.event_sink.clone())
    };

    if let Some(sink) = event_sink {
        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::Slack,
            account_id: account_id.to_string(),
            chat_id: channel_id,
            message_id: None,
        };
        match sink.dispatch_interaction(&action_id, reply_to).await {
            Ok(_response) => {
                // Response already sent by the gateway.
            },
            Err(e) => {
                debug!(account_id, action_id, "interaction dispatch failed: {e}");
            },
        }
    }

    Ok(())
}

/// Handle a Slack message event.
pub(crate) async fn handle_message_event(
    account_id: &str,
    event: SlackMessageEvent,
    accounts: &AccountStateMap,
) {
    // Skip message subtypes (edits, deletes, bot messages, etc.).
    if event.subtype.is_some() {
        return;
    }

    let user_id = match &event.sender.user {
        Some(u) => u.to_string(),
        None => return, // No user — skip (bot message or system).
    };

    // Skip messages from our own bot.
    {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accts.get(account_id)
            && state
                .bot_user_id
                .as_ref()
                .is_some_and(|bid| bid == &user_id)
        {
            return;
        }
    }

    let channel_id = match &event.origin.channel {
        Some(c) => c.to_string(),
        None => return,
    };

    let text = event
        .content
        .as_ref()
        .and_then(|c| c.text.as_deref())
        .unwrap_or("");

    let thread_ts = event.origin.thread_ts.as_ref().map(|ts| ts.to_string());
    // Use thread_ts if available, otherwise use the message ts for threading.
    let reply_thread = thread_ts.or_else(|| Some(event.origin.ts.to_string()));

    // Detect if this is a mention.
    let bot_user_id = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.bot_user_id.clone())
    };
    let is_mention = bot_user_id
        .as_ref()
        .is_some_and(|bid| text.contains(&format!("<@{bid}>")));

    handle_inbound(
        account_id,
        &channel_id,
        &user_id,
        text,
        reply_thread,
        event.sender.username.clone(),
        is_mention,
        accounts,
    )
    .await;
}

/// Core inbound message processing.
///
/// Shared by message events, app_mention events, and webhook events.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_inbound(
    account_id: &str,
    channel_id: &str,
    user_id: &str,
    text: &str,
    thread_ts: Option<String>,
    username: Option<String>,
    is_mention: bool,
    accounts: &AccountStateMap,
) {
    let (config, message_log, event_sink, bot_user_id) = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        match accts.get(account_id) {
            Some(state) => (
                state.config.clone(),
                state.message_log.clone(),
                state.event_sink.clone(),
                state.bot_user_id.clone(),
            ),
            None => return,
        }
    };

    // Determine if this is a DM or channel message.
    // Slack DM channel IDs start with 'D'.
    let is_dm = channel_id.starts_with('D');

    // Access control check.
    let access_granted = check_access(
        is_dm,
        user_id,
        channel_id,
        &config.dm_policy,
        &config.group_policy,
        &config.allowlist,
        &config.channel_allowlist,
    );

    // Log to message_log (always, even if denied).
    if let Some(log) = &message_log {
        let chat_type = if is_dm {
            "dm"
        } else {
            "channel"
        };
        let entry = MessageLogEntry {
            id: 0,
            account_id: account_id.to_string(),
            channel_type: "slack".to_string(),
            peer_id: user_id.to_string(),
            username: username.clone(),
            sender_name: None,
            chat_id: channel_id.to_string(),
            chat_type: chat_type.to_string(),
            body: text.to_string(),
            access_granted,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        };
        if let Err(e) = log.log(entry).await {
            warn!(account_id, "failed to log slack message: {e}");
        }
    }

    // Emit inbound event (always, even if denied).
    if let Some(sink) = &event_sink {
        sink.emit(ChannelEvent::InboundMessage {
            channel_type: ChannelType::Slack,
            account_id: account_id.to_string(),
            peer_id: user_id.to_string(),
            username: username.clone(),
            sender_name: None,
            message_count: None,
            access_granted,
        })
        .await;
    }

    if !access_granted {
        debug!(
            account_id,
            user_id, channel_id, "slack message denied by access control"
        );
        return;
    }

    // Check activation mode for non-DM channels.
    if !is_dm {
        match config.mention_mode {
            moltis_channels::gating::MentionMode::Mention => {
                if !is_mention {
                    return;
                }
            },
            moltis_channels::gating::MentionMode::None => return,
            moltis_channels::gating::MentionMode::Always => {},
        }
    }

    // Strip bot mention from the text.
    let clean_text = if let Some(bid) = &bot_user_id {
        strip_mentions(text, bid)
    } else {
        text.to_string()
    };

    let clean_text = clean_text.trim();
    if clean_text.is_empty() {
        return;
    }

    // Store thread_ts for reply threading.
    if let Some(ts) = &thread_ts {
        let thread_key = format!("{channel_id}:{user_id}");
        let mut accts = accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accts.get_mut(account_id) {
            state.pending_threads.insert(thread_key, ts.clone());
        }
    }

    // Dispatch to chat.
    if let Some(sink) = &event_sink {
        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::Slack,
            account_id: account_id.to_string(),
            chat_id: channel_id.to_string(),
            message_id: thread_ts,
        };

        let meta = ChannelMessageMeta {
            channel_type: ChannelType::Slack,
            sender_name: None,
            username,
            message_kind: Some(ChannelMessageKind::Text),
            model: config.resolve_model(channel_id, user_id).map(String::from),
            audio_filename: None,
        };

        #[cfg(feature = "metrics")]
        moltis_metrics::counter!(
            moltis_metrics::channels::MESSAGES_RECEIVED_TOTAL,
            moltis_metrics::labels::CHANNEL => "slack"
        )
        .increment(1);

        sink.dispatch_to_chat(clean_text, reply_to, meta).await;
    }
}

/// Handle a reaction_added or reaction_removed event.
pub(crate) async fn handle_reaction_event(
    account_id: &str,
    user_id: &str,
    emoji: &str,
    item: &SlackReactionsItem,
    added: bool,
    accounts: &AccountStateMap,
) {
    // Only handle reactions on messages (not files).
    let (channel_id, message_ts) = match item {
        SlackReactionsItem::Message(msg) => {
            let channel = msg.origin.channel.as_ref().map(|c| c.to_string());
            let ts = msg.origin.ts.to_string();
            match channel {
                Some(c) => (c, ts),
                None => return,
            }
        },
        _ => return,
    };

    let event_sink = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.event_sink.clone())
    };

    if let Some(sink) = event_sink {
        sink.emit(ChannelEvent::ReactionChange {
            channel_type: ChannelType::Slack,
            account_id: account_id.to_string(),
            chat_id: channel_id,
            message_id: message_ts,
            user_id: user_id.to_string(),
            emoji: emoji.to_string(),
            added,
        })
        .await;
    }
}

/// Check if a message should be processed based on access policies.
pub(crate) fn check_access(
    is_dm: bool,
    user_id: &str,
    channel_id: &str,
    dm_policy: &DmPolicy,
    group_policy: &GroupPolicy,
    user_allowlist: &[String],
    channel_allowlist: &[String],
) -> bool {
    if is_dm {
        match dm_policy {
            DmPolicy::Open => true,
            DmPolicy::Allowlist => is_allowed(user_id, user_allowlist),
            DmPolicy::Disabled => false,
        }
    } else {
        match group_policy {
            GroupPolicy::Open => true,
            GroupPolicy::Allowlist => is_allowed(channel_id, channel_allowlist),
            GroupPolicy::Disabled => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dm_open_allows_anyone() {
        assert!(check_access(
            true,
            "U123",
            "D456",
            &DmPolicy::Open,
            &GroupPolicy::Open,
            &[],
            &[],
        ));
    }

    #[test]
    fn dm_allowlist_requires_user() {
        assert!(!check_access(
            true,
            "U999",
            "D456",
            &DmPolicy::Allowlist,
            &GroupPolicy::Open,
            &["U123".to_string()],
            &[],
        ));
        assert!(check_access(
            true,
            "U123",
            "D456",
            &DmPolicy::Allowlist,
            &GroupPolicy::Open,
            &["U123".to_string()],
            &[],
        ));
    }

    #[test]
    fn dm_disabled_denies_all() {
        assert!(!check_access(
            true,
            "U123",
            "D456",
            &DmPolicy::Disabled,
            &GroupPolicy::Open,
            &[],
            &[],
        ));
    }

    #[test]
    fn channel_open_allows_any() {
        assert!(check_access(
            false,
            "U123",
            "C456",
            &DmPolicy::Allowlist,
            &GroupPolicy::Open,
            &[],
            &[],
        ));
    }

    #[test]
    fn channel_allowlist_requires_channel() {
        assert!(!check_access(
            false,
            "U123",
            "C999",
            &DmPolicy::Allowlist,
            &GroupPolicy::Allowlist,
            &[],
            &["C456".to_string()],
        ));
        assert!(check_access(
            false,
            "U123",
            "C456",
            &DmPolicy::Allowlist,
            &GroupPolicy::Allowlist,
            &[],
            &["C456".to_string()],
        ));
    }

    #[test]
    fn channel_disabled_denies_all() {
        assert!(!check_access(
            false,
            "U123",
            "C456",
            &DmPolicy::Allowlist,
            &GroupPolicy::Disabled,
            &[],
            &[],
        ));
    }
}
