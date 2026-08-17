use std::sync::Arc;

use {
    slack_morphism::prelude::*,
    tracing::{debug, warn},
};

use moltis_channels::plugin::{ChannelReplyTarget, ChannelType};

use crate::{
    callback_worker::{CallbackAdmission, CallbackAdmissionError, CallbackJob},
    state::DedupKind,
};

use super::{ListenerState, account_access_allowed};

/// Push events callback (messages, app_mention, etc.).
pub(super) async fn push_events_callback(
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

    let event_id = event.event_id.to_string();
    match admit_callback(
        &listener_state,
        DedupKind::Event,
        &event_id,
        CallbackJob::Push {
            event: Box::new(event),
            account_id: listener_state.account_id.clone(),
            accounts: listener_state.accounts.clone(),
        },
    ) {
        Ok(CallbackAdmission::Queued) => {},
        Ok(CallbackAdmission::Duplicate) => {
            debug!(
                account_id = %listener_state.account_id,
                event_id,
                "dropping duplicate slack event (retry)"
            );
        },
        Err(error) => {
            warn!(
                account_id = %listener_state.account_id,
                event_id,
                "slack push event was not admitted: {error}"
            );
            return Err(Box::new(error));
        },
    }

    Ok(())
}

/// Command events callback (slash commands).
pub(super) async fn command_events_callback(
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
    let sender_id = event.user_id.to_string();
    let channel_id = event.channel_id.to_string();
    let trigger_id = format!("command:{}", event.trigger_id);

    if !account_access_allowed(
        &listener_state.accounts,
        account_id,
        &sender_id,
        &channel_id,
    ) {
        return Ok(SlackCommandEventResponse::new(
            SlackMessageContent::new().with_text("Access denied.".to_string()),
        ));
    }

    let event_sink = {
        let accts = listener_state
            .accounts
            .read()
            .unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.event_sink.clone())
    };

    if let Some(sink) = event_sink {
        let reply_to = ChannelReplyTarget {
            ack_message_id: None,
            channel_type: ChannelType::Slack,
            account_id: account_id.to_string(),
            chat_id: channel_id,
            message_id: None,
            thread_id: None,
        };
        let response_url = event.response_url.0.as_str().to_string();
        match admit_callback(
            &listener_state,
            DedupKind::Command,
            &trigger_id,
            CallbackJob::Command {
                sink,
                command: full_command,
                reply_to,
                sender_id,
                response_url,
            },
        ) {
            Ok(CallbackAdmission::Queued) => Ok(SlackCommandEventResponse::new(
                SlackMessageContent::new().with_text("Command accepted.".to_string()),
            )),
            Ok(CallbackAdmission::Duplicate) => Ok(SlackCommandEventResponse::new(
                SlackMessageContent::new().with_text("Command already accepted.".to_string()),
            )),
            Err(error) => {
                warn!(
                    account_id,
                    trigger_id, "slack command was not admitted: {error}"
                );
                Err(Box::new(error))
            },
        }
    } else {
        Ok(SlackCommandEventResponse::new(
            SlackMessageContent::new().with_text("Channel not configured".to_string()),
        ))
    }
}

/// Interaction events callback (block actions / button clicks).
pub(super) async fn interaction_events_callback(
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

    let (action_id, channel_id, user_id, thread_root, response_url, trigger_id) = match &event {
        SlackInteractionEvent::BlockActions(ba) => {
            let action = ba.actions.as_ref().and_then(|a| a.first());
            let channel = ba.channel.as_ref().map(|c| c.id.to_string());
            let user = ba.user.as_ref().map(|user| user.id.to_string());
            let thread_root = ba.message.as_ref().map(|message| {
                message
                    .origin
                    .thread_ts
                    .as_ref()
                    .unwrap_or(&message.origin.ts)
                    .to_string()
            });
            match (action, channel, user) {
                (Some(act), Some(ch), Some(user)) => (
                    act.action_id.to_string(),
                    ch,
                    user,
                    thread_root,
                    ba.response_url
                        .as_ref()
                        .map(|url| url.0.as_str().to_string()),
                    format!("interaction:{}", ba.trigger_id),
                ),
                _ => {
                    debug!("block_actions missing action, channel, or user");
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
    if !account_access_allowed(&listener_state.accounts, account_id, &user_id, &channel_id) {
        if let Some(response_url) = response_url {
            match admit_callback(
                &listener_state,
                DedupKind::Interaction,
                &trigger_id,
                CallbackJob::ResponseUrl {
                    account_id: account_id.to_string(),
                    response_url,
                    text: "Access denied.".to_string(),
                },
            ) {
                Ok(CallbackAdmission::Queued | CallbackAdmission::Duplicate) => {},
                Err(error) => {
                    warn!(
                        account_id,
                        trigger_id, "slack denial response was not admitted: {error}"
                    );
                    return Err(Box::new(error));
                },
            }
        }
        return Ok(());
    }

    let event_sink = {
        let accts = listener_state
            .accounts
            .read()
            .unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.event_sink.clone())
    };

    if let Some(sink) = event_sink {
        let reply_to = ChannelReplyTarget {
            ack_message_id: None,
            channel_type: ChannelType::Slack,
            account_id: account_id.to_string(),
            chat_id: channel_id,
            message_id: thread_root,
            thread_id: None,
        };
        match admit_callback(
            &listener_state,
            DedupKind::Interaction,
            &trigger_id,
            CallbackJob::Interaction {
                sink,
                action_id,
                reply_to,
                sender_id: user_id,
                response_url,
            },
        ) {
            Ok(CallbackAdmission::Queued) => {},
            Ok(CallbackAdmission::Duplicate) => {
                debug!(
                    account_id,
                    trigger_id, "dropping duplicate slack interaction (retry)"
                );
            },
            Err(error) => {
                warn!(
                    account_id,
                    trigger_id, "slack interaction was not admitted: {error}"
                );
                return Err(Box::new(error));
            },
        }
    }

    Ok(())
}

fn admit_callback(
    listener_state: &ListenerState,
    kind: DedupKind,
    id: &str,
    job: CallbackJob,
) -> Result<CallbackAdmission, CallbackAdmissionError> {
    let accounts = listener_state
        .accounts
        .read()
        .unwrap_or_else(|error| error.into_inner());
    let state = accounts
        .get(&listener_state.account_id)
        .ok_or(CallbackAdmissionError::Canceled)?;
    let mut dedup = state
        .dedup
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    listener_state
        .callback_queue
        .try_send_deduplicated(&mut dedup, kind, id, job)
}
