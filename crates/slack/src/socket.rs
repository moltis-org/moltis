use std::sync::Arc;

use {
    secrecy::ExposeSecret,
    slack_morphism::prelude::*,
    tracing::{debug, info, warn},
};

mod callbacks;
#[cfg(test)]
mod tests;

use moltis_channels::{
    config_view::ChannelConfigView,
    gating::{DmPolicy, GroupPolicy, is_allowed},
    message_log::MessageLogEntry,
    otp::{
        OTP_TTL, OtpInitResult, OtpVerifyResult, approve_sender_via_otp, emit_otp_challenge,
        emit_otp_resolution,
    },
    plugin::{
        ChannelEvent, ChannelEventSink, ChannelMessageKind, ChannelMessageMeta, ChannelOutbound,
        ChannelReplyTarget, ChannelType,
    },
};

use crate::{
    callback_worker::{CallbackAdmissionError, CallbackQueue},
    client::validated_slack_client_for_base_url,
    config::SlackAccountConfig,
    markdown::strip_mentions,
    outbound::SlackOutbound,
    socket_reconnect::{RECONNECT_INITIAL_BACKOFF, RECONNECT_STABLE_AFTER, backoff_sleep},
    state::{AccountState, AccountStateMap, StreamRecipient},
};

use callbacks::{command_events_callback, interaction_events_callback, push_events_callback};

const OTP_CHALLENGE_MSG: &str = "To use this bot, please enter the verification code.\n\nAsk the bot owner for the code; it is visible in the web UI under Channels > Senders.\n\nThe code expires in 5 minutes.";

/// State stored in the Socket Mode listener for callback access.
#[derive(Clone)]
struct ListenerState {
    account_id: String,
    accounts: AccountStateMap,
    callback_queue: CallbackQueue,
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
    event_sink: Option<Arc<dyn ChannelEventSink>>,
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

    let client = Arc::new(validated_slack_client_for_base_url(&config.api_base_url).await?);

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

    let otp_cooldown_secs = config.otp_cooldown_secs;

    {
        let mut accts = accounts.write().unwrap_or_else(|e| e.into_inner());
        accts.insert(account_id.to_string(), AccountState {
            account_id: account_id.to_string(),
            config,
            message_log,
            event_sink,
            cancel: cancel.clone(),
            bot_user_id: Some(bot_user_id),
            stream_recipients: Default::default(),
            otp: std::sync::Mutex::new(moltis_channels::otp::OtpState::new(otp_cooldown_secs)),
            dedup: std::sync::Mutex::new(crate::state::EventDedup::default()),
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

/// Run the Socket Mode listener until cancelled, reconnecting on disconnect.
///
/// slack-morphism's `serve()` returns when the socket drops; on its own that
/// silently kills inbound delivery. This supervises the connection: on any
/// unexpected exit (or a failed `listen_for`) it rebuilds the listener and
/// retries with exponential backoff + jitter, resetting the backoff after a
/// healthy connection. Cancellation always wins the race and shuts down cleanly.
async fn run_socket_listener(
    account_id: &str,
    client: Arc<SlackClient<SlackClientHyperHttpsConnector>>,
    app_token: SlackApiToken,
    accounts: AccountStateMap,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut backoff = RECONNECT_INITIAL_BACKOFF;
    let callback_queue = CallbackQueue::start(cancel.clone());

    while !cancel.is_cancelled() {
        let listener_state = ListenerState {
            account_id: account_id.to_string(),
            accounts: accounts.clone(),
            callback_queue: callback_queue.clone(),
        };

        // Callbacks must be function pointers, so per-account data rides on the
        // listener environment's user state.
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

        if let Err(e) = socket_listener.listen_for(&app_token).await {
            warn!(account_id, "slack socket mode connect failed: {e}");
            if !backoff_sleep(&cancel, &mut backoff).await {
                break;
            }
            continue;
        }

        info!(account_id, "slack socket mode listener started");
        let connected_at = tokio::time::Instant::now();

        tokio::select! {
            () = cancel.cancelled() => {
                info!(account_id, "slack socket mode shutting down");
                socket_listener.shutdown().await;
                break;
            }
            _code = socket_listener.serve() => {
                socket_listener.shutdown().await;
                // A connection that lasted a while was healthy; reset backoff.
                if connected_at.elapsed() >= RECONNECT_STABLE_AFTER {
                    backoff = RECONNECT_INITIAL_BACKOFF;
                }
                warn!(
                    account_id,
                    backoff_secs = backoff.as_secs(),
                    "slack socket mode disconnected; reconnecting"
                );
                if !backoff_sleep(&cancel, &mut backoff).await {
                    break;
                }
            }
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
    if err.downcast_ref::<CallbackAdmissionError>().is_some() {
        HttpStatusCode::SERVICE_UNAVAILABLE
    } else {
        HttpStatusCode::OK
    }
}

/// Handle a push event body. Runs off the Socket Mode acknowledgment path.
pub(crate) async fn handle_push_event(
    event: SlackPushEventCallback,
    account_id: &str,
    accounts: &AccountStateMap,
) {
    let team_id = event.team_id.to_string();
    match event.event {
        SlackEventCallbackBody::Message(msg_event) => {
            handle_message_event(account_id, msg_event, Some(team_id), accounts).await;
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

            let message_ts = Some(mention_event.origin.ts.to_string());

            handle_inbound(
                account_id,
                &channel,
                &user,
                text,
                thread_ts,
                message_ts,
                Some(team_id),
                None,
                true, // is_mention
                accounts,
            )
            .await;
        },
        SlackEventCallbackBody::ReactionAdded(reaction_event) => {
            handle_reaction_event(
                account_id,
                reaction_event.user.as_ref(),
                reaction_event.reaction.as_ref(),
                &reaction_event.item,
                reaction_event.item_user.as_ref().map(|u| u.to_string()),
                Some(team_id),
                true,
                accounts,
            )
            .await;
        },
        SlackEventCallbackBody::ReactionRemoved(reaction_event) => {
            handle_reaction_event(
                account_id,
                reaction_event.user.as_ref(),
                reaction_event.reaction.as_ref(),
                &reaction_event.item,
                reaction_event.item_user.as_ref().map(|u| u.to_string()),
                Some(team_id),
                false,
                accounts,
            )
            .await;
        },
        _ => {
            debug!("unhandled slack push event");
        },
    }
}

/// Handle a Slack message event.
pub(crate) async fn handle_message_event(
    account_id: &str,
    event: SlackMessageEvent,
    team_id: Option<String>,
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
    let message_ts = event.origin.ts.to_string();
    let reply_thread = thread_ts.or_else(|| Some(message_ts.clone()));

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
        Some(message_ts),
        team_id,
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
    message_ts: Option<String>,
    team_id: Option<String>,
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

    let is_dm = channel_id.starts_with('D');
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
        if is_dm && config.dm_policy == DmPolicy::Allowlist && config.otp_self_approval {
            handle_otp_flow(
                accounts,
                account_id,
                user_id,
                username.as_deref(),
                text,
                channel_id,
                event_sink.as_deref(),
            )
            .await;
        }
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

    let thread_root = thread_ts.or_else(|| message_ts.clone());

    if let (Some(thread_root), Some(team_id)) = (&thread_root, team_id) {
        let mut accts = accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accts.get_mut(account_id) {
            state.note_stream_recipient(channel_id, thread_root, StreamRecipient {
                user_id: user_id.to_string(),
                team_id,
            });
        }
    }

    // Acknowledge with reactions only when the bot is directly addressed
    // (DM or @mention) — never on general channel chatter — and the account
    // has ack reactions enabled.
    let ack_message_id = ack_reaction_target(config.ack_reactions, is_dm, is_mention, message_ts);

    // Dispatch to chat.
    if let Some(sink) = &event_sink {
        let reply_to = ChannelReplyTarget {
            ack_message_id,
            channel_type: ChannelType::Slack,
            account_id: account_id.to_string(),
            chat_id: channel_id.to_string(),
            message_id: thread_root,
            thread_id: None,
        };

        let meta = ChannelMessageMeta {
            channel_type: ChannelType::Slack,
            sender_name: None,
            username,
            sender_id: Some(user_id.to_string()),
            message_kind: Some(ChannelMessageKind::Text),
            model: config.resolve_model(channel_id, user_id).map(String::from),
            agent_id: config
                .resolve_agent_id(channel_id, user_id)
                .map(String::from),
            audio_filename: None,
            documents: None,
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

pub(crate) fn account_access_allowed(
    accounts: &AccountStateMap,
    account_id: &str,
    user_id: &str,
    channel_id: &str,
) -> bool {
    let accounts = accounts.read().unwrap_or_else(|error| error.into_inner());
    let Some(state) = accounts.get(account_id) else {
        return false;
    };
    check_access(
        channel_id.starts_with('D'),
        user_id,
        channel_id,
        &state.config.dm_policy,
        &state.config.group_policy,
        &state.config.allowlist,
        &state.config.channel_allowlist,
    )
}

/// Handle a reaction_added or reaction_removed event.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_reaction_event(
    account_id: &str,
    user_id: &str,
    emoji: &str,
    item: &SlackReactionsItem,
    item_user: Option<String>,
    team_id: Option<String>,
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
    dispatch_reaction(
        account_id, user_id, emoji, channel_id, message_ts, item_user, team_id, added, accounts,
    )
    .await;
}

/// Core reaction handling shared by Socket Mode and the Events API.
/// Always emits a [`ChannelEvent::ReactionChange`] for observers; additionally
/// routes the reaction into the agent as a synthetic message when
/// `reaction_triggers` is enabled and the reaction is eligible.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_reaction(
    account_id: &str,
    user_id: &str,
    emoji: &str,
    channel_id: String,
    message_ts: String,
    item_user: Option<String>,
    team_id: Option<String>,
    added: bool,
    accounts: &AccountStateMap,
) {
    let (config, event_sink, bot_user_id) = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        match accts.get(account_id) {
            Some(state) => (
                state.config.clone(),
                state.event_sink.clone(),
                state.bot_user_id.clone(),
            ),
            None => return,
        }
    };

    let Some(sink) = event_sink else {
        return;
    };

    // Always surface the raw reaction change to observers (web UI, hooks).
    sink.emit(ChannelEvent::ReactionChange {
        channel_type: ChannelType::Slack,
        account_id: account_id.to_string(),
        chat_id: channel_id.clone(),
        message_id: message_ts.clone(),
        user_id: user_id.to_string(),
        emoji: emoji.to_string(),
        added,
    })
    .await;

    // Optionally route the reaction into the agent as a message. The bot's own
    // acknowledgment reactions (👀/✅/❌) are always ignored to avoid loops.
    //
    // Fail closed: if the bot user id is unknown we cannot distinguish the bot's
    // own reactions, so treat the reactor as "self" and skip. Triggering here
    // would let the bot's own ACK reactions fire agent turns and loop.
    let is_self = match bot_user_id.as_deref() {
        Some(bot_id) => bot_id == user_id,
        None => {
            debug!(
                account_id,
                user_id, "slack reaction trigger skipped: bot_user_id unknown"
            );
            true
        },
    };
    // Only reactions on the bot's *own* messages drive the agent. Without this
    // any member could point the agent at an arbitrary third party's message
    // simply by reacting to it. Fail closed when authorship is unknown.
    let target_is_bot = match (bot_user_id.as_deref(), item_user.as_deref()) {
        (Some(bot_id), Some(author)) => bot_id == author,
        _ => false,
    };
    if !reaction_should_trigger(
        config.reaction_triggers,
        added,
        is_self,
        target_is_bot,
        emoji,
        &config.reaction_trigger_emojis,
    ) {
        return;
    }

    let is_dm = channel_id.starts_with('D');
    let access_granted = check_access(
        is_dm,
        user_id,
        &channel_id,
        &config.dm_policy,
        &config.group_policy,
        &config.allowlist,
        &config.channel_allowlist,
    );
    if !access_granted {
        debug!(
            account_id,
            user_id, "slack reaction trigger denied by access control"
        );
        return;
    }
    if let Some(team_id) = team_id {
        let mut accts = accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accts.get_mut(account_id) {
            state.note_stream_recipient(&channel_id, &message_ts, StreamRecipient {
                user_id: user_id.to_string(),
                team_id,
            });
        }
    }
    // Thread the synthetic message under the reacted message so the agent sees
    // the original content as thread context (dispatch fetches thread history).
    let reply_to = ChannelReplyTarget {
        channel_type: ChannelType::Slack,
        account_id: account_id.to_string(),
        chat_id: channel_id,
        message_id: Some(message_ts),
        thread_id: None,
        // Never acknowledge a reaction trigger with another reaction.
        ack_message_id: None,
    };

    let meta = ChannelMessageMeta {
        channel_type: ChannelType::Slack,
        sender_name: None,
        username: None,
        sender_id: Some(user_id.to_string()),
        message_kind: Some(ChannelMessageKind::Text),
        model: config
            .resolve_model(&reply_to.chat_id, user_id)
            .map(String::from),
        agent_id: config
            .resolve_agent_id(&reply_to.chat_id, user_id)
            .map(String::from),
        audio_filename: None,
        documents: None,
    };

    let synthetic = format!("<@{user_id}> reacted :{emoji}: to this message.");
    sink.dispatch_to_chat(&synthetic, reply_to, meta).await;
}

/// Decide whether an inbound reaction should be routed to the agent.
///
/// Triggers only when: the feature is enabled, the reaction was *added* (not
/// removed), it is not the bot's own reaction, the reacted-to message was
/// authored by the bot, and — if an emoji allowlist is configured — the emoji
/// is on it. An empty allowlist matches any emoji.
fn reaction_should_trigger(
    enabled: bool,
    added: bool,
    is_self: bool,
    target_is_bot: bool,
    emoji: &str,
    allowlist: &[String],
) -> bool {
    enabled
        && added
        && !is_self
        && target_is_bot
        && (allowlist.is_empty() || allowlist.iter().any(|e| e == emoji))
}

/// Decide which inbound message (if any) to acknowledge with reactions.
///
/// Returns the message ts only when acknowledgment reactions are enabled for
/// the account and the bot was directly addressed — a 1:1 DM or an @mention.
/// General channel chatter never earns a reaction (avoids emoji noise).
fn ack_reaction_target(
    ack_reactions_enabled: bool,
    is_dm: bool,
    is_mention: bool,
    message_ts: Option<String>,
) -> Option<String> {
    if ack_reactions_enabled && (is_dm || is_mention) {
        message_ts
    } else {
        None
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
            DmPolicy::Allowlist => {
                !user_allowlist.is_empty() && is_allowed(user_id, user_allowlist)
            },
            DmPolicy::Disabled => false,
        }
    } else {
        match group_policy {
            GroupPolicy::Open => true,
            GroupPolicy::Allowlist => {
                !channel_allowlist.is_empty() && is_allowed(channel_id, channel_allowlist)
            },
            GroupPolicy::Disabled => false,
        }
    }
}

async fn handle_otp_flow(
    accounts: &AccountStateMap,
    account_id: &str,
    user_id: &str,
    username: Option<&str>,
    text: &str,
    channel_id: &str,
    event_sink: Option<&dyn ChannelEventSink>,
) {
    let has_pending = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts
            .get(account_id)
            .map(|s| {
                let otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                otp.has_pending(user_id)
            })
            .unwrap_or(false)
    };

    if has_pending {
        let body = text.trim();
        let is_code = body.len() == 6 && body.chars().all(|c| c.is_ascii_digit());
        if !is_code {
            return;
        }

        let result = {
            let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
            match accts.get(account_id) {
                Some(s) => {
                    let mut otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                    otp.verify(user_id, body)
                },
                None => return,
            }
        };

        #[cfg(feature = "metrics")]
        record_otp_verification(&result);

        match result {
            OtpVerifyResult::Approved => {
                approve_sender_via_otp(
                    event_sink,
                    ChannelType::Slack,
                    account_id,
                    user_id,
                    user_id,
                    username,
                )
                .await;
                send_otp_status(
                    accounts,
                    account_id,
                    channel_id,
                    "Verified! You now have access to this bot.",
                )
                .await;
            },
            OtpVerifyResult::WrongCode { attempts_left } => {
                let msg = format!(
                    "Incorrect code. {attempts_left} attempt{} remaining.",
                    if attempts_left == 1 {
                        ""
                    } else {
                        "s"
                    }
                );
                send_otp_status(accounts, account_id, channel_id, &msg).await;
            },
            OtpVerifyResult::LockedOut => {
                send_otp_status(
                    accounts,
                    account_id,
                    channel_id,
                    "Too many failed attempts. Please try again later.",
                )
                .await;
                emit_otp_resolution(
                    event_sink,
                    ChannelType::Slack,
                    account_id,
                    user_id,
                    username,
                    "locked_out",
                )
                .await;
            },
            OtpVerifyResult::Expired => {
                send_otp_status(
                    accounts,
                    account_id,
                    channel_id,
                    "Your code has expired. Send any message to get a new one.",
                )
                .await;
                emit_otp_resolution(
                    event_sink,
                    ChannelType::Slack,
                    account_id,
                    user_id,
                    username,
                    "expired",
                )
                .await;
            },
            OtpVerifyResult::NoPending => {},
        }
        return;
    }

    let init_result = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        match accts.get(account_id) {
            Some(s) => {
                let mut otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                otp.initiate(user_id, username.map(String::from), None)
            },
            None => return,
        }
    };

    match init_result {
        OtpInitResult::Created(code) => {
            #[cfg(feature = "metrics")]
            moltis_metrics::counter!(
                moltis_metrics::channels::OTP_CHALLENGES_TOTAL,
                moltis_metrics::labels::CHANNEL => "slack"
            )
            .increment(1);
            send_otp_status(accounts, account_id, channel_id, OTP_CHALLENGE_MSG).await;
            let expires_at = std::time::SystemTime::now()
                .checked_add(OTP_TTL)
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or_default();
            emit_otp_challenge(
                event_sink,
                ChannelType::Slack,
                account_id,
                user_id,
                username,
                None,
                code,
                expires_at,
            )
            .await;
        },
        OtpInitResult::AlreadyPending | OtpInitResult::LockedOut => {},
    }
}

#[cfg(feature = "metrics")]
fn record_otp_verification(result: &OtpVerifyResult) {
    let label = match result {
        OtpVerifyResult::Approved => "approved",
        OtpVerifyResult::WrongCode { .. } => "wrong_code",
        OtpVerifyResult::LockedOut => "locked_out",
        OtpVerifyResult::Expired => "expired",
        OtpVerifyResult::NoPending => return,
    };
    moltis_metrics::counter!(
        moltis_metrics::channels::OTP_VERIFICATIONS_TOTAL,
        moltis_metrics::labels::CHANNEL => "slack",
        "result" => label
    )
    .increment(1);
}

async fn send_otp_status(
    accounts: &AccountStateMap,
    account_id: &str,
    channel_id: &str,
    text: &str,
) {
    let outbound = SlackOutbound {
        accounts: Arc::clone(accounts),
    };
    if let Err(e) = outbound.send_text(account_id, channel_id, text, None).await {
        warn!(
            account_id,
            channel_id, "failed to send slack OTP status: {e}"
        );
    }
}
