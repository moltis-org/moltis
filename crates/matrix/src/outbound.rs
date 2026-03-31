use {
    async_trait::async_trait,
    matrix_sdk::ruma::{
        OwnedRoomId,
        events::room::message::RoomMessageEventContent,
    },
    tracing::warn,
};

use moltis_channels::{Error as ChannelError, Result as ChannelResult, plugin::InteractiveMessage};
use moltis_common::types::ReplyPayload;

#[cfg(feature = "metrics")]
use moltis_metrics::counter;

use crate::{markdown, state::AccountStateMap};

pub struct MatrixOutbound {
    pub accounts: AccountStateMap,
}

impl MatrixOutbound {
    fn get_room(
        &self,
        account_id: &str,
        room_id_str: &str,
    ) -> ChannelResult<(matrix_sdk::Client, matrix_sdk::Room)> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;

        let room_id: OwnedRoomId = room_id_str
            .try_into()
            .map_err(|e| ChannelError::invalid_input(format!("invalid room ID '{room_id_str}': {e}")))?;

        let room = state
            .client
            .get_room(&room_id)
            .ok_or_else(|| ChannelError::invalid_input(format!("room '{room_id_str}' not found")))?;

        Ok((state.client.clone(), room))
    }
}

#[async_trait]
impl moltis_channels::ChannelOutbound for MatrixOutbound {
    #[tracing::instrument(skip(self))]
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let (_client, room) = self.get_room(account_id, to)?;
        let html = markdown::markdown_to_html(text);
        let content = RoomMessageEventContent::text_html(text, html);
        room.send(content).await.map_err(|e| {
            ChannelError::external("matrix send_text", e)
        })?;
        #[cfg(feature = "metrics")]
        counter!("matrix.messages.sent").increment(1);
        Ok(())
    }

    #[tracing::instrument(skip(self, payload))]
    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let (_client, room) = self.get_room(account_id, to)?;
        crate::media::send_media(&room, payload).await.map_err(|e| {
            ChannelError::external("matrix send_media", std::io::Error::other(e.to_string()))
        })?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        let (_client, room) = self.get_room(account_id, to)?;
        room.typing_notice(true).await.map_err(|e| {
            ChannelError::external("matrix send_typing", e)
        })?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let (_client, room) = self.get_room(account_id, to)?;
        let content = RoomMessageEventContent::text_html(html, html);
        room.send(content).await.map_err(|e| {
            ChannelError::external("matrix send_html", e)
        })?;
        Ok(())
    }

    async fn send_text_with_suffix(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let combined = format!("{text}\n\n{suffix_html}");
        let combined_html = format!("{}\n\n{suffix_html}", markdown::markdown_to_html(text));
        let (_client, room) = self.get_room(account_id, to)?;
        let content = RoomMessageEventContent::text_html(&combined, combined_html);
        room.send(content).await.map_err(|e| {
            ChannelError::external("matrix send_text_with_suffix", e)
        })?;
        let _ = reply_to;
        Ok(())
    }

    async fn add_reaction(
        &self,
        account_id: &str,
        _channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> ChannelResult<()> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;

        let event_id: matrix_sdk::ruma::OwnedEventId = message_id
            .try_into()
            .map_err(|e| ChannelError::invalid_input(format!("invalid event ID: {e}")))?;

        use matrix_sdk::ruma::events::reaction::ReactionEventContent;
        use matrix_sdk::ruma::events::relation::Annotation;

        let annotation = Annotation::new(event_id, emoji.to_string());
        let content = ReactionEventContent::new(annotation);

        warn!(account_id, message_id, emoji, "add_reaction requires room context");
        let _ = content;
        let _ = state;

        Ok(())
    }

    async fn remove_reaction(
        &self,
        account_id: &str,
        _channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> ChannelResult<()> {
        let _ = (account_id, message_id, emoji);
        warn!(account_id, message_id, emoji, "remove_reaction not yet implemented for matrix");
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn send_location(
        &self,
        account_id: &str,
        to: &str,
        latitude: f64,
        longitude: f64,
        title: Option<&str>,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let (_client, room) = self.get_room(account_id, to)?;
        let geo_uri = format!("geo:{latitude},{longitude}");
        let body = title.map_or_else(|| geo_uri.clone(), |t| format!("{t}\n{geo_uri}"));

        use matrix_sdk::ruma::events::room::message::LocationMessageEventContent;
        let location = LocationMessageEventContent::new(body, geo_uri);
        let content = RoomMessageEventContent::new(
            matrix_sdk::ruma::events::room::message::MessageType::Location(location),
        );
        room.send(content).await.map_err(|e| {
            ChannelError::external("matrix send_location", e)
        })?;
        Ok(())
    }

    async fn send_interactive(
        &self,
        account_id: &str,
        to: &str,
        message: &InteractiveMessage,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let mut text = message.text.clone();
        let mut idx = 1;
        for row in &message.button_rows {
            for btn in row {
                text.push_str(&format!("\n{idx}. {}", btn.label));
                idx += 1;
            }
        }
        self.send_text(account_id, to, &text, reply_to).await
    }
}
