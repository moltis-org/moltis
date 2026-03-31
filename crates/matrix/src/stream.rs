use {
    async_trait::async_trait,
    matrix_sdk::ruma::{
        OwnedRoomId,
        events::room::message::RoomMessageEventContent,
    },
    tracing::warn,
};

use moltis_channels::{
    Error as ChannelError, Result as ChannelResult,
    plugin::{StreamEvent, StreamReceiver},
};

#[cfg(feature = "metrics")]
use moltis_metrics::counter;

use crate::{markdown, state::AccountStateMap};

use matrix_sdk::ruma::events::room::message::ReplacementMetadata;

pub struct MatrixStreamOutbound {
    pub accounts: AccountStateMap,
}

#[async_trait]
impl moltis_channels::ChannelStreamOutbound for MatrixStreamOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        _reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        let (config, room) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let state = accounts
                .get(account_id)
                .ok_or_else(|| ChannelError::unknown_account(account_id))?;

            let room_id: OwnedRoomId = to
                .try_into()
                .map_err(|e| ChannelError::invalid_input(format!("invalid room ID: {e}")))?;
            let room = state
                .client
                .get_room(&room_id)
                .ok_or_else(|| ChannelError::invalid_input(format!("room '{to}' not found")))?;

            (state.config.clone(), room)
        };

        let throttle = std::time::Duration::from_millis(config.edit_throttle_ms);
        let min_initial = config.stream_min_initial_chars;

        let mut buffer = String::new();
        let mut sent_event_id: Option<matrix_sdk::ruma::OwnedEventId> = None;
        let mut last_edit = std::time::Instant::now();

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(text) => {
                    buffer.push_str(&text);

                    if sent_event_id.is_none() && buffer.len() < min_initial {
                        continue;
                    }

                    if last_edit.elapsed() < throttle && sent_event_id.is_some() {
                        continue;
                    }

                    let html = markdown::markdown_to_html(&buffer);
                    if let Some(ref eid) = sent_event_id {
                        let edit_content = RoomMessageEventContent::text_html(&buffer, &html)
                            .make_replacement(ReplacementMetadata::new(eid.clone(), None));
                        if let Err(e) = room.send(edit_content).await {
                            warn!(account_id, error = %e, "stream edit failed");
                        }
                    } else {
                        let content = RoomMessageEventContent::text_html(&buffer, &html);
                        match room.send(content).await {
                            Ok(response) => {
                                sent_event_id = Some(response.event_id);
                            },
                            Err(e) => {
                                warn!(account_id, error = %e, "stream initial send failed");
                            },
                        }
                    }
                    last_edit = std::time::Instant::now();
                    #[cfg(feature = "metrics")]
                    counter!("matrix.stream.edits").increment(1);
                },
                StreamEvent::Done => {
                    if !buffer.is_empty() {
                        let html = markdown::markdown_to_html(&buffer);
                        if let Some(ref eid) = sent_event_id {
                            let edit_content = RoomMessageEventContent::text_html(&buffer, &html)
                                .make_replacement(ReplacementMetadata::new(eid.clone(), None));
                            if let Err(e) = room.send(edit_content).await {
                                warn!(account_id, error = %e, "stream final edit failed");
                            }
                        } else {
                            let content = RoomMessageEventContent::text_html(&buffer, &html);
                            if let Err(e) = room.send(content).await {
                                warn!(account_id, error = %e, "stream final send failed");
                            }
                        }
                    }
                    break;
                },
                StreamEvent::Error(msg) => {
                    let error_text = format!("Error: {msg}");
                    if let Some(ref eid) = sent_event_id {
                        let edit_content =
                            RoomMessageEventContent::text_plain(&error_text)
                                .make_replacement(ReplacementMetadata::new(eid.clone(), None));
                        room.send(edit_content).await.ok();
                    } else {
                        let content = RoomMessageEventContent::text_plain(&error_text);
                        room.send(content).await.ok();
                    }
                    break;
                },
            }
        }

        Ok(())
    }

    async fn is_stream_enabled(&self, account_id: &str) -> bool {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .is_some_and(|s| s.config.stream_mode == crate::config::StreamMode::EditInPlace)
    }
}
