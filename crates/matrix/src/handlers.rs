use std::sync::Arc;

use {
    matrix_sdk::{
        Client, Room,
        ruma::events::room::message::{
            MessageType, OriginalSyncRoomMessageEvent,
        },
    },
    tracing::{debug, warn},
};

use crate::{
    access,
    state::AccountStateMap,
};

#[cfg(feature = "metrics")]
use moltis_metrics::counter;

pub fn register_event_handlers(client: &Client, accounts: AccountStateMap, account_id: String) {
    let accounts_clone = Arc::clone(&accounts);
    let aid = account_id.clone();

    client.add_event_handler(
        move |event: OriginalSyncRoomMessageEvent, room: Room, client: Client| {
            let accounts = Arc::clone(&accounts_clone);
            let account_id = aid.clone();
            async move {
                if let Err(e) = handle_room_message(event, room, client, &accounts, &account_id).await {
                    warn!(account_id, error = %e, "failed to handle matrix message");
                }
            }
        },
    );
}

#[tracing::instrument(skip(event, room, client, accounts))]
async fn handle_room_message(
    event: OriginalSyncRoomMessageEvent,
    room: Room,
    client: Client,
    accounts: &AccountStateMap,
    account_id: &str,
) -> crate::Result<()> {
    let own_user_id = client.user_id().ok_or_else(|| crate::Error::message("not logged in"))?;

    if event.sender == own_user_id {
        return Ok(());
    }

    let member_count = room.joined_members_count();
    let chat_type = if member_count <= 2 {
        moltis_common::types::ChatType::Dm
    } else {
        moltis_common::types::ChatType::Group
    };

    let sender_id = event.sender.as_str();
    let room_id = room.room_id().as_str();

    let body = match &event.content.msgtype {
        MessageType::Text(text) => &text.body,
        _ => {
            debug!(account_id, sender = sender_id, "ignoring non-text message type");
            return Ok(());
        },
    };

    let bot_mentioned = body.contains(own_user_id.as_str())
        || body.contains(own_user_id.localpart());

    let (event_sink, model, message_log) = {
        let accounts_guard = accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts_guard
            .get(account_id)
            .ok_or_else(|| crate::Error::message("account not found"))?;

        if let Err(denied) = access::check_access(
            &state.config,
            &chat_type,
            sender_id,
            None,
            Some(room_id),
            bot_mentioned,
        ) {
            debug!(account_id, sender = sender_id, reason = %denied, "access denied");
            return Ok(());
        }

        (state.event_sink.clone(), state.config.model.clone(), state.message_log.clone())
    };

    if let Some(sink) = &event_sink {
        let reply_to = moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Matrix,
            account_id: account_id.to_string(),
            chat_id: room_id.to_string(),
            message_id: Some(event.event_id.to_string()),
            thread_id: None,
        };

        let meta = moltis_channels::ChannelMessageMeta {
            channel_type: moltis_channels::ChannelType::Matrix,
            sender_name: Some(sender_id.to_string()),
            username: Some(sender_id.to_string()),
            message_kind: Some(moltis_channels::ChannelMessageKind::Text),
            model,
            audio_filename: None,
        };

        sink.dispatch_to_chat(body, reply_to, meta).await;

        #[cfg(feature = "metrics")]
        counter!("matrix.messages.received").increment(1);
    }

    if let Some(log) = &message_log {
        use moltis_channels::message_log::MessageLogEntry;
        let entry = MessageLogEntry {
            id: 0,
            account_id: account_id.to_string(),
            channel_type: "matrix".to_string(),
            peer_id: sender_id.to_string(),
            username: Some(sender_id.to_string()),
            sender_name: None,
            chat_id: room_id.to_string(),
            chat_type: match chat_type {
                moltis_common::types::ChatType::Dm => "dm".to_string(),
                _ => "group".to_string(),
            },
            body: body.to_string(),
            access_granted: true,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        };
        log.log(entry).await.ok();
    }

    Ok(())
}
