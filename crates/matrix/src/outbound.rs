use std::time::Duration;

use {
    async_trait::async_trait,
    base64::Engine,
    matrix_sdk::{
        Room,
        attachment::{
            AttachmentConfig, AttachmentInfo, BaseAudioInfo, BaseFileInfo, BaseImageInfo,
            BaseVideoInfo,
        },
        deserialized_responses::TimelineEvent,
        room::{
            IncludeRelations, RelationsOptions,
            reply::{EnforceThread, Reply},
        },
        ruma::{
            self, OwnedEventId, OwnedRoomId,
            api::Direction,
            events::{
                AnySyncMessageLikeEvent, AnySyncTimelineEvent,
                poll::unstable_start::{
                    NewUnstablePollStartEventContent, UnstablePollAnswer, UnstablePollAnswers,
                    UnstablePollStartContentBlock, UnstablePollStartEventContent,
                },
                reaction::ReactionEventContent,
                relation::{Annotation, InReplyTo, Thread},
                room::message::{
                    LocationMessageEventContent, MessageType, Relation, RelationWithoutReplacement,
                    RoomMessageEventContent, TextMessageEventContent,
                },
            },
            serde::Raw,
        },
    },
    tracing::{debug, warn},
};

use {
    moltis_channels::{
        Error as ChannelError, Result as ChannelResult,
        plugin::{
            ChannelOutbound, ChannelStreamOutbound, ChannelThreadContext, InteractiveMessage,
            StreamEvent, StreamReceiver, ThreadMessage,
        },
    },
    moltis_common::types::{MediaAttachment, ReplyPayload},
};

use crate::{config::StreamMode, state::AccountStateMap};

/// Typing indicator refresh interval.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(4);

pub struct MatrixOutbound {
    pub accounts: AccountStateMap,
}

impl MatrixOutbound {
    fn get_room(&self, account_id: &str, room_id_str: &str) -> ChannelResult<Room> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;
        let client = state.client.clone();
        drop(accounts);

        let room_id: OwnedRoomId = room_id_str
            .parse()
            .map_err(|e: ruma::IdParseError| ChannelError::invalid_input(e.to_string()))?;

        client
            .get_room(&room_id)
            .ok_or_else(|| ChannelError::unavailable(format!("room not found: {room_id_str}")))
    }

    fn get_bot_user_id(&self, account_id: &str) -> ChannelResult<String> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|state| state.bot_user_id.clone())
            .ok_or_else(|| ChannelError::unknown_account(account_id))
    }

    fn get_stream_config(&self, account_id: &str) -> ChannelResult<(StreamMode, Duration, usize)> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;

        Ok((
            state.config.stream_mode.clone(),
            Duration::from_millis(state.config.edit_throttle_ms),
            state.config.stream_min_initial_chars,
        ))
    }

    async fn make_message_content(
        room: &Room,
        content: RoomMessageEventContent,
        reply_to: Option<&OwnedEventId>,
        context: &'static str,
    ) -> ChannelResult<RoomMessageEventContent> {
        let Some(reply_to) = reply_to else {
            return Ok(content);
        };

        room.make_reply_event(content.into(), Reply {
            event_id: reply_to.clone(),
            // If the target event belongs to a thread, stay in that thread.
            // If it doesn't, send a normal rich reply.
            enforce_thread: EnforceThread::MaybeThreaded,
        })
        .await
        .map_err(|e| ChannelError::external(context, e))
    }

    async fn poll_relation(
        room: &Room,
        reply_to: Option<&OwnedEventId>,
    ) -> Option<RelationWithoutReplacement> {
        let reply_to = reply_to?.clone();
        let reply_relation = || RelationWithoutReplacement::Reply {
            in_reply_to: InReplyTo::new(reply_to.clone()),
        };

        let Ok(target_event) = room.load_or_fetch_event(&reply_to, None).await else {
            warn!(event_id = %reply_to, "matrix interactive poll reply target fetch failed, using plain reply relation");
            return Some(reply_relation());
        };

        let Ok(target_json) = target_event
            .raw()
            .deserialize_as_unchecked::<serde_json::Value>()
        else {
            warn!(event_id = %reply_to, "matrix interactive poll reply target decode failed, using plain reply relation");
            return Some(reply_relation());
        };

        Some(poll_relation_from_value(&reply_to, &target_json))
    }

    fn parse_event_id(event_id: &str, field_name: &'static str) -> ChannelResult<OwnedEventId> {
        event_id.parse().map_err(|e: ruma::IdParseError| {
            ChannelError::invalid_input(format!("invalid {field_name}: {e}"))
        })
    }

    fn attachment_reply(reply_to: Option<&OwnedEventId>) -> Option<Reply> {
        reply_to.cloned().map(|event_id| Reply {
            event_id,
            enforce_thread: EnforceThread::MaybeThreaded,
        })
    }

    fn attachment_caption(text: &str) -> Option<TextMessageEventContent> {
        if text.is_empty() {
            None
        } else {
            Some(TextMessageEventContent::markdown(text))
        }
    }

    fn attachment_info(mime_type: &str, data_len: usize) -> AttachmentInfo {
        let size = u64::try_from(data_len).ok().and_then(ruma::UInt::new);

        if mime_type.starts_with("image/") {
            AttachmentInfo::Image(BaseImageInfo {
                size,
                ..Default::default()
            })
        } else if mime_type.starts_with("video/") {
            AttachmentInfo::Video(BaseVideoInfo {
                size,
                ..Default::default()
            })
        } else if mime_type == "audio/ogg" {
            AttachmentInfo::Voice(BaseAudioInfo {
                size,
                ..Default::default()
            })
        } else if mime_type.starts_with("audio/") {
            AttachmentInfo::Audio(BaseAudioInfo {
                size,
                ..Default::default()
            })
        } else {
            AttachmentInfo::File(BaseFileInfo { size })
        }
    }

    fn attachment_filename(media: &MediaAttachment) -> String {
        media.filename.clone().unwrap_or_else(|| {
            let extension = extension_for_mime(&media.mime_type);
            format!("file.{extension}")
        })
    }

    fn decode_data_url(url: &str) -> ChannelResult<Vec<u8>> {
        let (metadata, base64_data) = url
            .split_once(',')
            .ok_or_else(|| ChannelError::invalid_input("invalid data URI: no comma separator"))?;

        if !metadata.ends_with(";base64") {
            return Err(ChannelError::invalid_input(
                "invalid data URI: expected ';base64' payload",
            ));
        }

        base64::engine::general_purpose::STANDARD
            .decode(base64_data)
            .map_err(|e| ChannelError::invalid_input(format!("failed to decode base64: {e}")))
    }

    fn remote_media_fallback_text(payload: &ReplyPayload) -> String {
        let media_url = payload
            .media
            .as_ref()
            .map(|media| media.url.as_str())
            .unwrap_or_default();

        if payload.text.is_empty() {
            media_url.to_string()
        } else {
            format!("{}\n{media_url}", payload.text)
        }
    }

    fn interactive_poll_content(
        message: &InteractiveMessage,
        relates_to: Option<RelationWithoutReplacement>,
    ) -> ChannelResult<Option<UnstablePollStartEventContent>> {
        let buttons = message.button_rows.iter().flatten().collect::<Vec<_>>();
        if buttons.is_empty() {
            return Ok(None);
        }
        if buttons.len() > 20 {
            warn!(
                button_count = buttons.len(),
                "matrix interactive message exceeds poll answer limit, falling back to text"
            );
            return Ok(None);
        }

        let mut seen_callback_data = std::collections::HashSet::with_capacity(buttons.len());
        let answers = buttons
            .into_iter()
            .map(|button| {
                let callback_data = button.callback_data.trim();
                if callback_data.is_empty() {
                    return Err(ChannelError::invalid_input(
                        "matrix interactive button callback_data must not be empty",
                    ));
                }
                if !seen_callback_data.insert(callback_data.to_string()) {
                    return Err(ChannelError::invalid_input(format!(
                        "matrix interactive button callback_data must be unique: {callback_data}"
                    )));
                }

                Ok(UnstablePollAnswer::new(
                    callback_data.to_string(),
                    button.label.clone(),
                ))
            })
            .collect::<ChannelResult<Vec<_>>>()?;
        let answers = UnstablePollAnswers::try_from(answers).map_err(|error| {
            ChannelError::invalid_input(format!("invalid matrix poll answers: {error}"))
        })?;
        let poll_start = UnstablePollStartContentBlock::new(message.text.clone(), answers);
        let mut content = NewUnstablePollStartEventContent::plain_text(
            interactive_poll_plain_text(message),
            poll_start,
        );
        content.relates_to = relates_to;

        Ok(Some(UnstablePollStartEventContent::New(content)))
    }

    async fn collect_stream_text(stream: &mut StreamReceiver) -> String {
        let mut buffer = String::new();

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(chunk) => buffer.push_str(&chunk),
                StreamEvent::Done => break,
                StreamEvent::Error(error) => {
                    warn!("stream error: {error}");
                    if !buffer.is_empty() {
                        buffer.push_str("\n\n[stream error]");
                    }
                    break;
                },
            }
        }

        buffer
    }

    async fn find_own_reaction_event_id(
        room: &Room,
        target_event_id: &OwnedEventId,
        bot_user_id: &str,
        emoji: &str,
    ) -> ChannelResult<Option<OwnedEventId>> {
        let mut from = None;

        loop {
            let relations = room
                .relations(target_event_id.clone(), RelationsOptions {
                    from,
                    dir: Direction::Backward,
                    limit: usize_to_uint(100),
                    include_relations: IncludeRelations::RelationsOfType(
                        ruma::events::relation::RelationType::Annotation,
                    ),
                    ..Default::default()
                })
                .await
                .map_err(|e| ChannelError::external("matrix remove_reaction relations", e))?;

            if let Some(event_id) = relations.chunk.iter().find_map(|event| {
                matching_reaction_event_id(event, bot_user_id, target_event_id, emoji)
            }) {
                return Ok(Some(event_id));
            }

            if relations.next_batch_token.is_none() {
                return Ok(None);
            }

            from = relations.next_batch_token;
        }
    }
}

#[async_trait]
impl ChannelOutbound for MatrixOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let room = self.get_room(account_id, to)?;
        let reply_to = reply_to
            .map(|event_id| Self::parse_event_id(event_id, "reply_to"))
            .transpose()?;
        let content = Self::make_message_content(
            &room,
            RoomMessageEventContent::text_markdown(text),
            reply_to.as_ref(),
            "matrix send_text",
        )
        .await?;
        room.send(content)
            .await
            .map_err(|e| ChannelError::external("matrix send_text", e))?;
        record_message_sent();
        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let room = self.get_room(account_id, to)?;
        let reply_to_id = reply_to;
        let reply_to = reply_to
            .map(|event_id| Self::parse_event_id(event_id, "reply_to"))
            .transpose()?;

        if let Some(media) = &payload.media {
            if !media.url.starts_with("data:") {
                warn!(
                    account_id,
                    url = %media.url,
                    "matrix outbound media only uploads data URIs, falling back to text with URL"
                );
                let text = Self::remote_media_fallback_text(payload);
                return self.send_text(account_id, to, &text, reply_to_id).await;
            }

            let data = Self::decode_data_url(&media.url)?;
            let content_type = media.mime_type.parse().map_err(|e| {
                ChannelError::invalid_input(format!(
                    "invalid media MIME type '{}': {e}",
                    media.mime_type
                ))
            })?;
            let filename = Self::attachment_filename(media);
            let config = AttachmentConfig::new()
                .info(Self::attachment_info(&media.mime_type, data.len()))
                .caption(Self::attachment_caption(&payload.text))
                .reply(Self::attachment_reply(reply_to.as_ref()));

            debug!(
                account_id,
                mime_type = %media.mime_type,
                filename = %filename,
                bytes = data.len(),
                "matrix outbound media upload start"
            );

            room.send_attachment(filename, &content_type, data, config)
                .await
                .map_err(|e| ChannelError::external("matrix send_attachment", e))?;
            record_message_sent();
        } else if !payload.text.is_empty() {
            self.send_text(account_id, to, &payload.text, reply_to_id)
                .await?;
        }

        Ok(())
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        let room = self.get_room(account_id, to)?;
        let _ = room.typing_notice(true).await;
        Ok(())
    }

    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let room = self.get_room(account_id, to)?;
        let reply_to = reply_to
            .map(|event_id| Self::parse_event_id(event_id, "reply_to"))
            .transpose()?;
        let content = Self::make_message_content(
            &room,
            RoomMessageEventContent::text_html(html_to_plain(html), html),
            reply_to.as_ref(),
            "matrix send_html",
        )
        .await?;
        room.send(content)
            .await
            .map_err(|e| ChannelError::external("matrix send_html", e))?;
        record_message_sent();
        Ok(())
    }

    async fn send_interactive(
        &self,
        account_id: &str,
        to: &str,
        message: &InteractiveMessage,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let reply_to_id = reply_to;
        let room = self.get_room(account_id, to)?;
        let reply_to = reply_to
            .map(|event_id| Self::parse_event_id(event_id, "reply_to"))
            .transpose()?;
        let relates_to = Self::poll_relation(&room, reply_to.as_ref()).await;
        let Some(content) = Self::interactive_poll_content(message, relates_to)? else {
            return self
                .send_text(
                    account_id,
                    to,
                    &interactive_poll_plain_text(message),
                    reply_to_id,
                )
                .await;
        };

        room.send(content)
            .await
            .map_err(|e| ChannelError::external("matrix send_interactive", e))?;
        record_message_sent();
        Ok(())
    }

    async fn add_reaction(
        &self,
        account_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> ChannelResult<()> {
        let room = self.get_room(account_id, channel_id)?;
        let event_id: OwnedEventId = message_id
            .parse()
            .map_err(|e: ruma::IdParseError| ChannelError::invalid_input(e.to_string()))?;

        let annotation = Annotation::new(event_id, emoji.to_string());
        let content = ReactionEventContent::new(annotation);
        room.send(content)
            .await
            .map_err(|e| ChannelError::external("matrix add_reaction", e))?;
        Ok(())
    }

    async fn remove_reaction(
        &self,
        account_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> ChannelResult<()> {
        let room = self.get_room(account_id, channel_id)?;
        let bot_user_id = self.get_bot_user_id(account_id)?;
        let target_event_id = Self::parse_event_id(message_id, "message_id")?;
        let Some(reaction_event_id) =
            Self::find_own_reaction_event_id(&room, &target_event_id, &bot_user_id, emoji).await?
        else {
            debug!(
                account_id,
                channel_id,
                message_id,
                emoji,
                "matrix reaction removal skipped because no matching reaction was found"
            );
            return Ok(());
        };

        room.redact(&reaction_event_id, Some("Removing reaction"), None)
            .await
            .map_err(|e| ChannelError::external("matrix remove_reaction redact", e))?;
        Ok(())
    }

    async fn send_location(
        &self,
        account_id: &str,
        to: &str,
        latitude: f64,
        longitude: f64,
        title: Option<&str>,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let room = self.get_room(account_id, to)?;
        let reply_to = reply_to
            .map(|event_id| Self::parse_event_id(event_id, "reply_to"))
            .transpose()?;
        let content =
            RoomMessageEventContent::new(MessageType::Location(LocationMessageEventContent::new(
                location_body(latitude, longitude, title),
                location_geo_uri(latitude, longitude),
            )));
        let content =
            Self::make_message_content(&room, content, reply_to.as_ref(), "matrix send_location")
                .await?;
        room.send(content)
            .await
            .map_err(|e| ChannelError::external("matrix send_location", e))?;
        record_message_sent();
        Ok(())
    }
}

#[async_trait]
impl ChannelStreamOutbound for MatrixOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        let room = self.get_room(account_id, to)?;
        let (stream_mode, edit_throttle, min_initial_chars) = self.get_stream_config(account_id)?;
        let reply_to = reply_to
            .map(|event_id| Self::parse_event_id(event_id, "reply_to"))
            .transpose()?;

        if stream_mode == StreamMode::Off {
            let final_text = Self::collect_stream_text(&mut stream).await;
            if !final_text.is_empty() {
                let content = Self::make_message_content(
                    &room,
                    RoomMessageEventContent::text_markdown(&final_text),
                    reply_to.as_ref(),
                    "matrix stream final send",
                )
                .await?;
                room.send(content)
                    .await
                    .map_err(|e| ChannelError::external("matrix stream", e))?;
                record_message_sent();
            }
            return Ok(());
        }

        let mut buffer = String::new();
        let mut sent_event_id: Option<OwnedEventId> = None;
        let mut last_edit = tokio::time::Instant::now();

        let _ = room.typing_notice(true).await;
        let mut typing_refresh = tokio::time::interval(TYPING_REFRESH_INTERVAL);
        typing_refresh.tick().await;

        loop {
            tokio::select! {
                _ = typing_refresh.tick() => {
                    if sent_event_id.is_none() {
                        let _ = room.typing_notice(true).await;
                    }
                }
                event = stream.recv() => {
                    match event {
                        Some(StreamEvent::Delta(chunk)) => {
                            buffer.push_str(&chunk);

                            if sent_event_id.is_none() && buffer.len() >= min_initial_chars {
                                let content = Self::make_message_content(
                                    &room,
                                    RoomMessageEventContent::text_markdown(&buffer),
                                    reply_to.as_ref(),
                                    "matrix stream initial send",
                                )
                                .await?;
                                match room.send(content).await {
                                    Ok(response) => {
                                        sent_event_id = Some(response.event_id);
                                        last_edit = tokio::time::Instant::now();
                                        record_message_sent();
                                        let _ = room.typing_notice(false).await;
                                    }
                                    Err(e) => {
                                        warn!("stream initial send failed: {e}");
                                        return Err(ChannelError::external("matrix stream", e));
                                    }
                                }
                            } else if sent_event_id.is_some()
                                && last_edit.elapsed() >= edit_throttle
                                && let Some(eid) = &sent_event_id
                            {
                                let edit = make_edit_content(eid, &buffer);
                                if let Err(e) = room.send(edit).await {
                                    warn!("stream edit failed: {e}");
                                }
                                last_edit = tokio::time::Instant::now();
                            }
                        }
                        Some(StreamEvent::Done) => {
                            if let Some(eid) = &sent_event_id {
                                let edit = make_edit_content(eid, &buffer);
                                if let Err(error) = room.send(edit).await {
                                    warn!(account_id, room_id = to, "stream final edit failed: {error}");
                                }
                            } else if !buffer.is_empty() {
                                let content = Self::make_message_content(
                                    &room,
                                    RoomMessageEventContent::text_markdown(&buffer),
                                    reply_to.as_ref(),
                                    "matrix stream final send",
                                )
                                .await?;
                                if room.send(content).await.is_ok() {
                                    record_message_sent();
                                }
                            }
                            let _ = room.typing_notice(false).await;
                            break;
                        }
                        Some(StreamEvent::Error(e)) => {
                            warn!("stream error: {e}");
                            if !buffer.is_empty() {
                                buffer.push_str("\n\n[stream error]");
                                if let Some(eid) = &sent_event_id {
                                    let edit = make_edit_content(eid, &buffer);
                                    if let Err(error) = room.send(edit).await {
                                        warn!(account_id, room_id = to, "stream error edit failed: {error}");
                                    }
                                } else {
                                    let content = Self::make_message_content(
                                        &room,
                                        RoomMessageEventContent::text_markdown(&buffer),
                                        reply_to.as_ref(),
                                        "matrix stream error send",
                                    )
                                    .await?;
                                    if room.send(content).await.is_ok() {
                                        record_message_sent();
                                    }
                                }
                            }
                            let _ = room.typing_notice(false).await;
                            break;
                        }
                        None => {
                            let _ = room.typing_notice(false).await;
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn is_stream_enabled(&self, account_id: &str) -> bool {
        self.get_stream_config(account_id)
            .map(|(stream_mode, ..)| stream_mode != StreamMode::Off)
            .unwrap_or(false)
    }
}

#[async_trait]
impl ChannelThreadContext for MatrixOutbound {
    async fn fetch_thread_messages(
        &self,
        account_id: &str,
        channel_id: &str,
        thread_id: &str,
        limit: usize,
    ) -> ChannelResult<Vec<ThreadMessage>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let room = self.get_room(account_id, channel_id)?;
        let bot_user_id = self.get_bot_user_id(account_id)?;
        let target_event_id = Self::parse_event_id(thread_id, "thread_id")?;
        let target_event = room
            .load_or_fetch_event(&target_event_id, None)
            .await
            .map_err(|e| ChannelError::external("matrix fetch_thread_messages target", e))?;

        let thread_root_id = extract_thread_root_event_id(target_event.raw())
            .unwrap_or_else(|| target_event_id.clone());
        let mut initial_relation_messages = None;

        if thread_root_id == target_event_id {
            let relation_limit = limit.saturating_sub(1).max(1);
            let relation_limit = usize_to_uint(relation_limit);
            let relation_batch = room
                .relations(thread_root_id.clone(), RelationsOptions {
                    dir: Direction::Forward,
                    limit: relation_limit,
                    include_relations: IncludeRelations::RelationsOfType(
                        ruma::events::relation::RelationType::Thread,
                    ),
                    ..Default::default()
                })
                .await
                .map_err(|e| ChannelError::external("matrix fetch_thread_messages relations", e))?;

            if relation_batch.chunk.is_empty() {
                return Ok(Vec::new());
            }

            initial_relation_messages = Some(relation_batch.chunk);
        }

        let root_event = if thread_root_id == target_event_id {
            target_event
        } else {
            room.load_or_fetch_event(&thread_root_id, None)
                .await
                .map_err(|e| ChannelError::external("matrix fetch_thread_messages root", e))?
        };

        let mut messages = vec![timeline_event_to_thread_message(&root_event, &bot_user_id)];
        let relation_events = if let Some(relation_events) = initial_relation_messages {
            relation_events
        } else if limit > 1 {
            room.relations(thread_root_id.clone(), RelationsOptions {
                dir: Direction::Forward,
                limit: usize_to_uint(limit.saturating_sub(1)),
                include_relations: IncludeRelations::RelationsOfType(
                    ruma::events::relation::RelationType::Thread,
                ),
                ..Default::default()
            })
            .await
            .map_err(|e| ChannelError::external("matrix fetch_thread_messages relations", e))?
            .chunk
        } else {
            Vec::new()
        };

        messages.extend(
            relation_events
                .iter()
                .map(|event| timeline_event_to_thread_message(event, &bot_user_id)),
        );
        messages.truncate(limit);

        Ok(messages)
    }
}

/// Create an m.replace edit event content.
fn make_edit_content(original_event_id: &OwnedEventId, new_body: &str) -> RoomMessageEventContent {
    use matrix_sdk::ruma::events::room::message::ReplacementMetadata;
    let new_content = RoomMessageEventContent::text_markdown(new_body);
    let metadata = ReplacementMetadata::new(original_event_id.clone(), None);
    new_content.make_replacement(metadata)
}

fn extract_thread_root_event_id(raw_event: &Raw<AnySyncTimelineEvent>) -> Option<OwnedEventId> {
    let event = raw_event.deserialize().ok()?;
    let AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(event)) = event
    else {
        return None;
    };
    let event = event.as_original()?;

    match &event.content.relates_to {
        Some(Relation::Thread(thread)) => Some(thread.event_id.clone()),
        _ => None,
    }
}

fn timeline_event_to_thread_message(event: &TimelineEvent, bot_user_id: &str) -> ThreadMessage {
    let raw = event.raw();
    let (sender_id, text) = raw
        .deserialize()
        .ok()
        .and_then(deserialize_message_details)
        .unwrap_or_else(|| (String::new(), String::new()));

    ThreadMessage {
        is_bot: sender_id == bot_user_id,
        sender_id,
        text,
        timestamp: event
            .timestamp()
            .map(|timestamp| timestamp.get().to_string())
            .unwrap_or_default(),
    }
}

fn deserialize_message_details(event: AnySyncTimelineEvent) -> Option<(String, String)> {
    let AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(event)) = event
    else {
        return None;
    };
    let event = event.as_original()?;
    let text = match &event.content.msgtype {
        MessageType::Text(text) => text.body.clone(),
        MessageType::Notice(notice) => notice.body.clone(),
        MessageType::Emote(emote) => emote.body.clone(),
        MessageType::Image(image) => image.body.clone(),
        MessageType::Audio(audio) => audio.body.clone(),
        MessageType::Video(video) => video.body.clone(),
        MessageType::File(file) => file.body.clone(),
        MessageType::Location(location) => location.body.clone(),
        _ => String::new(),
    };

    Some((event.sender.to_string(), text))
}

fn matching_reaction_event_id(
    event: &TimelineEvent,
    sender_id: &str,
    target_event_id: &OwnedEventId,
    emoji: &str,
) -> Option<OwnedEventId> {
    let raw = event.raw();
    let event = raw.deserialize().ok()?;
    let AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::Reaction(event)) = event else {
        return None;
    };
    let event = event.as_original()?;

    let annotation = &event.content.relates_to;
    if event.sender != sender_id
        || annotation.event_id != *target_event_id
        || annotation.key != emoji
    {
        return None;
    }

    Some(event.event_id.clone())
}

fn usize_to_uint(limit: usize) -> Option<ruma::UInt> {
    let limit = u32::try_from(limit).ok()?;
    Some(limit.into())
}

fn poll_relation_from_value(
    reply_to: &OwnedEventId,
    target_event: &serde_json::Value,
) -> RelationWithoutReplacement {
    let thread_root = target_event
        .pointer("/content/m.relates_to")
        .and_then(|relation| {
            (relation.get("rel_type").and_then(serde_json::Value::as_str) == Some("m.thread"))
                .then(|| relation.get("event_id").and_then(serde_json::Value::as_str))
                .flatten()
        })
        .and_then(|event_id| event_id.parse().ok());

    if let Some(thread_root) = thread_root {
        RelationWithoutReplacement::Thread(Thread::plain(thread_root, reply_to.clone()))
    } else {
        RelationWithoutReplacement::Reply {
            in_reply_to: InReplyTo::new(reply_to.clone()),
        }
    }
}

fn record_message_sent() {
    #[cfg(feature = "metrics")]
    moltis_metrics::counter!(
        moltis_metrics::channels::MESSAGES_SENT_TOTAL,
        moltis_metrics::labels::CHANNEL => "matrix"
    )
    .increment(1);
}

fn location_geo_uri(latitude: f64, longitude: f64) -> String {
    format!("geo:{latitude:.6},{longitude:.6}")
}

fn location_body(latitude: f64, longitude: f64, title: Option<&str>) -> String {
    let coordinates = format!("{latitude:.6}, {longitude:.6}");
    match title.map(str::trim).filter(|title| !title.is_empty()) {
        Some(title) => format!("{title}\n{coordinates}"),
        None => format!("Location: {coordinates}"),
    }
}

fn interactive_poll_plain_text(message: &InteractiveMessage) -> String {
    let mut text = message.text.clone();

    for (index, button) in message.button_rows.iter().flatten().enumerate() {
        text.push_str(&format!("\n{}. {}", index + 1, button.label));
    }

    text
}

/// Simple HTML to plain text conversion.
fn html_to_plain(html: &str) -> String {
    let mut plain = String::with_capacity(html.len());
    let mut remaining = html;

    while let Some(ch) = remaining.chars().next() {
        if ch == '<' {
            if let Some((tag_name, consumed_len)) = consume_html_tag(remaining) {
                if is_plain_text_line_break_tag(&tag_name) && !plain.ends_with('\n') {
                    plain.push('\n');
                }
                remaining = &remaining[consumed_len..];
                continue;
            }
        } else if ch == '&'
            && let Some((decoded, consumed_len)) = consume_html_entity(remaining)
        {
            plain.push_str(&decoded);
            remaining = &remaining[consumed_len..];
            continue;
        }

        plain.push(ch);
        remaining = &remaining[ch.len_utf8()..];
    }

    plain.trim_matches('\n').to_string()
}

fn consume_html_tag(input: &str) -> Option<(String, usize)> {
    let bytes = input.as_bytes();
    if bytes.first().copied()? != b'<' {
        return None;
    }

    let mut index = 1usize;
    if bytes.get(index).copied() == Some(b'/') {
        index += 1;
    }

    let name_start = index;
    let first = bytes.get(index).copied()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }

    while let Some(next) = bytes.get(index).copied() {
        if next.is_ascii_alphanumeric() || next == b'-' {
            index += 1;
            continue;
        }
        break;
    }

    let tag_name = input[name_start..index].to_ascii_lowercase();

    let mut quote = None;
    while let Some(next) = bytes.get(index).copied() {
        match quote {
            Some(delimiter) if next == delimiter => quote = None,
            Some(_) => {},
            None if next == b'\'' || next == b'"' => quote = Some(next),
            None if next == b'>' => return Some((tag_name, index + 1)),
            None => {},
        }
        index += 1;
    }

    None
}

fn consume_html_entity(input: &str) -> Option<(String, usize)> {
    if !input.starts_with('&') {
        return None;
    }

    let mut entity = String::new();
    for (index, ch) in input.char_indices() {
        entity.push(ch);
        let consumed_len = index + ch.len_utf8();
        if ch == ';' {
            return decode_html_entity(&entity).map(|decoded| (decoded, consumed_len));
        }
        if entity.len() > 12 {
            return None;
        }
    }

    None
}

fn decode_html_entity(entity: &str) -> Option<String> {
    match entity {
        "&amp;" => Some("&".to_string()),
        "&lt;" => Some("<".to_string()),
        "&gt;" => Some(">".to_string()),
        "&quot;" => Some("\"".to_string()),
        "&apos;" | "&#39;" => Some("'".to_string()),
        "&nbsp;" | "&#160;" => Some(" ".to_string()),
        _ => decode_numeric_html_entity(entity),
    }
}

fn decode_numeric_html_entity(entity: &str) -> Option<String> {
    let value = entity
        .strip_prefix("&#x")
        .or_else(|| entity.strip_prefix("&#X"))
        .and_then(|hex| hex.strip_suffix(';'))
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .or_else(|| {
            entity
                .strip_prefix("&#")
                .and_then(|decimal| decimal.strip_suffix(';'))
                .and_then(|decimal| decimal.parse::<u32>().ok())
        })?;

    char::from_u32(value).map(|ch| ch.to_string())
}

fn is_plain_text_line_break_tag(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "blockquote" | "br" | "div" | "li" | "ol" | "p" | "pre" | "tr" | "ul"
    )
}

fn extension_for_mime(mime_type: &str) -> &str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/webm" => "webm",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "application/json" => "json",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use {
        super::{
            MatrixOutbound, deserialize_message_details, extension_for_mime,
            extract_thread_root_event_id, html_to_plain, interactive_poll_plain_text,
            location_body, location_geo_uri, matching_reaction_event_id, poll_relation_from_value,
            timeline_event_to_thread_message,
        },
        matrix_sdk::{
            deserialized_responses::TimelineEvent,
            ruma::{
                events::{AnySyncTimelineEvent, room::message::RelationWithoutReplacement},
                owned_event_id,
                serde::Raw,
            },
        },
        moltis_common::types::{MediaAttachment, ReplyPayload},
        serde_json::json,
    };

    fn timeline_event_from_json(value: serde_json::Value) -> TimelineEvent {
        let raw = Raw::<AnySyncTimelineEvent>::from_json_string(value.to_string())
            .unwrap_or_else(|error| panic!("timeline raw event: {error}"));
        TimelineEvent::from_plaintext(raw)
    }

    #[test]
    fn html_to_plain_strips_unknown_tags_and_decodes_entities() {
        let plain = html_to_plain(
            "<div>Hello <span>world</span></div><script>alert(&quot;ok&quot;)</script>",
        );

        assert_eq!(plain, "Hello world\nalert(\"ok\")");
    }

    #[test]
    fn html_to_plain_preserves_non_tag_angle_bracket_text() {
        let plain = html_to_plain("<code>if a < b && c > d</code>");

        assert_eq!(plain, "if a < b && c > d");
    }

    #[test]
    fn extract_thread_root_event_id_reads_thread_relation() {
        let event = timeline_event_from_json(json!({
            "type": "m.room.message",
            "event_id": "$reply",
            "room_id": "!room:example.org",
            "sender": "@alice:example.org",
            "origin_server_ts": 1,
            "content": {
                "msgtype": "m.text",
                "body": "reply",
                "m.relates_to": {
                    "rel_type": "m.thread",
                    "event_id": "$root",
                    "m.in_reply_to": {
                        "event_id": "$other"
                    },
                    "is_falling_back": false
                }
            }
        }));

        assert_eq!(
            extract_thread_root_event_id(event.raw()),
            Some(owned_event_id!("$root"))
        );
    }

    #[test]
    fn extract_thread_root_event_id_ignores_plain_replies() {
        let event = timeline_event_from_json(json!({
            "type": "m.room.message",
            "event_id": "$reply",
            "room_id": "!room:example.org",
            "sender": "@alice:example.org",
            "origin_server_ts": 1,
            "content": {
                "msgtype": "m.text",
                "body": "reply",
                "m.relates_to": {
                    "m.in_reply_to": {
                        "event_id": "$parent"
                    }
                }
            }
        }));

        assert_eq!(extract_thread_root_event_id(event.raw()), None);
    }

    #[test]
    fn timeline_event_to_thread_message_uses_sender_and_text() {
        let event = timeline_event_from_json(json!({
            "type": "m.room.message",
            "event_id": "$reply",
            "room_id": "!room:example.org",
            "sender": "@bot:example.org",
            "origin_server_ts": 1234,
            "content": {
                "msgtype": "m.notice",
                "body": "hello"
            }
        }));

        let message = timeline_event_to_thread_message(&event, "@bot:example.org");

        assert_eq!(message.sender_id, "@bot:example.org");
        assert!(message.is_bot);
        assert_eq!(message.text, "hello");
        assert!(!message.timestamp.is_empty());
    }

    #[test]
    fn matching_reaction_event_id_matches_own_annotation() {
        let event = timeline_event_from_json(json!({
            "type": "m.reaction",
            "event_id": "$reaction",
            "room_id": "!room:example.org",
            "sender": "@bot:example.org",
            "origin_server_ts": 1234,
            "content": {
                "m.relates_to": {
                    "rel_type": "m.annotation",
                    "event_id": "$target",
                    "key": "👀"
                }
            }
        }));

        assert_eq!(
            matching_reaction_event_id(
                &event,
                "@bot:example.org",
                &owned_event_id!("$target"),
                "👀"
            ),
            Some(owned_event_id!("$reaction"))
        );
    }

    #[test]
    fn matching_reaction_event_id_rejects_other_sender_or_emoji() {
        let event = timeline_event_from_json(json!({
            "type": "m.reaction",
            "event_id": "$reaction",
            "room_id": "!room:example.org",
            "sender": "@alice:example.org",
            "origin_server_ts": 1234,
            "content": {
                "m.relates_to": {
                    "rel_type": "m.annotation",
                    "event_id": "$target",
                    "key": "🔥"
                }
            }
        }));

        assert_eq!(
            matching_reaction_event_id(
                &event,
                "@bot:example.org",
                &owned_event_id!("$target"),
                "🔥"
            ),
            None
        );
        assert_eq!(
            matching_reaction_event_id(
                &event,
                "@alice:example.org",
                &owned_event_id!("$target"),
                "👀"
            ),
            None
        );
    }

    #[test]
    fn location_helpers_format_native_matrix_payload_fields() {
        assert_eq!(location_geo_uri(48.8566, 2.3522), "geo:48.856600,2.352200");
        assert_eq!(
            location_body(48.8566, 2.3522, Some("Paris")),
            "Paris\n48.856600, 2.352200"
        );
        assert_eq!(
            location_body(48.8566, 2.3522, Some("   ")),
            "Location: 48.856600, 2.352200"
        );
    }

    #[test]
    fn interactive_poll_plain_text_numbers_choices() {
        let text = interactive_poll_plain_text(&moltis_channels::plugin::InteractiveMessage {
            text: "Pick one".into(),
            button_rows: vec![vec![
                moltis_channels::plugin::InteractiveButton {
                    label: "Alpha".into(),
                    callback_data: "alpha".into(),
                    style: Default::default(),
                },
                moltis_channels::plugin::InteractiveButton {
                    label: "Beta".into(),
                    callback_data: "beta".into(),
                    style: Default::default(),
                },
            ]],
            replace_message_id: None,
        });

        assert_eq!(text, "Pick one\n1. Alpha\n2. Beta");
    }

    #[test]
    fn poll_relation_from_value_preserves_threads() {
        let relation = poll_relation_from_value(
            &owned_event_id!("$reply"),
            &json!({
                "content": {
                    "m.relates_to": {
                        "rel_type": "m.thread",
                        "event_id": "$root"
                    }
                }
            }),
        );

        let RelationWithoutReplacement::Thread(thread) = relation else {
            panic!("expected thread relation");
        };
        assert_eq!(thread.event_id, owned_event_id!("$root"));
        assert_eq!(
            thread
                .in_reply_to
                .as_ref()
                .map(|in_reply_to| in_reply_to.event_id.clone()),
            Some(owned_event_id!("$reply"))
        );
        assert!(thread.is_falling_back);
    }

    #[test]
    fn poll_relation_from_value_falls_back_to_plain_reply() {
        let relation = poll_relation_from_value(&owned_event_id!("$reply"), &json!({}));

        let RelationWithoutReplacement::Reply { in_reply_to } = relation else {
            panic!("expected reply relation");
        };
        assert_eq!(in_reply_to.event_id, owned_event_id!("$reply"));
    }

    #[test]
    fn deserialize_message_details_ignores_non_message_events() {
        let raw = Raw::<AnySyncTimelineEvent>::from_json_string(
            json!({
                "type": "m.room.member",
                "event_id": "$state",
                "room_id": "!room:example.org",
                "sender": "@alice:example.org",
                "state_key": "@alice:example.org",
                "origin_server_ts": 1,
                "content": {
                    "membership": "join"
                }
            })
            .to_string(),
        )
        .unwrap_or_else(|error| panic!("state event raw: {error}"));
        let event = raw
            .deserialize()
            .unwrap_or_else(|error| panic!("state event deserialize: {error}"));

        assert_eq!(deserialize_message_details(event), None);
    }

    #[test]
    fn decode_data_url_rejects_non_base64_payloads() {
        let err = match MatrixOutbound::decode_data_url("data:text/plain,hello") {
            Ok(_) => panic!("non-base64 payload should fail"),
            Err(error) => error,
        };

        assert!(err.to_string().contains("expected ';base64'"));
    }

    #[test]
    fn decode_data_url_decodes_base64_payloads() {
        let bytes = MatrixOutbound::decode_data_url("data:text/plain;base64,aGVsbG8=")
            .unwrap_or_else(|error| panic!("valid base64 data URI: {error}"));

        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn remote_media_fallback_appends_url_after_caption() {
        let payload = ReplyPayload {
            text: "caption".into(),
            media: Some(MediaAttachment {
                url: "https://example.com/file.pdf".into(),
                mime_type: "application/pdf".into(),
                filename: None,
            }),
            reply_to_id: None,
            silent: false,
        };

        assert_eq!(
            MatrixOutbound::remote_media_fallback_text(&payload),
            "caption\nhttps://example.com/file.pdf"
        );
    }

    #[test]
    fn extension_for_mime_falls_back_to_bin() {
        assert_eq!(extension_for_mime("application/octet-stream"), "bin");
    }
}
