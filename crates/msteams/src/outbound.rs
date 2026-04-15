use {
    async_trait::async_trait,
    secrecy::ExposeSecret,
    tracing::{debug, warn},
};

use {
    moltis_channels::{
        Error as ChannelError, Result as ChannelResult,
        plugin::{ChannelOutbound, ChannelStreamOutbound, StreamEvent, StreamReceiver},
    },
    moltis_common::types::ReplyPayload,
};

use crate::{
    auth::get_access_token,
    chunking::{self, DEFAULT_CHUNK_LIMIT},
    config::{MsTeamsAccountConfig, StreamMode},
    errors::{self, SendErrorKind},
    state::AccountStateMap,
    streaming::{self, StreamSession},
};

/// Outbound sender for Microsoft Teams channel accounts.
pub struct MsTeamsOutbound {
    pub(crate) accounts: AccountStateMap,
}

struct AccountSnapshot {
    config: MsTeamsAccountConfig,
    http: reqwest::Client,
    token_cache: std::sync::Arc<tokio::sync::Mutex<Option<crate::auth::CachedAccessToken>>>,
    service_url: String,
}

/// Result of a send operation that may include the created activity ID.
pub(crate) struct SendOutcome {
    pub(crate) activity_id: Option<String>,
}

impl MsTeamsOutbound {
    fn account_snapshot(
        &self,
        account_id: &str,
        conversation_id: &str,
    ) -> ChannelResult<AccountSnapshot> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;
        let service_url = {
            let service_urls = state.service_urls.read().unwrap_or_else(|e| e.into_inner());
            service_urls
                .get(conversation_id)
                .cloned()
                .ok_or_else(|| {
                    ChannelError::unavailable(format!(
                        "missing Teams service URL for account '{account_id}' and conversation '{conversation_id}'"
                    ))
                })?
        };

        Ok(AccountSnapshot {
            config: state.config.clone(),
            http: state.http.clone(),
            token_cache: std::sync::Arc::clone(&state.token_cache),
            service_url,
        })
    }

    /// Send an activity to Teams with retry logic.
    ///
    /// Returns the activity ID from the response, needed for edit-in-place
    /// streaming and message updates.
    pub(crate) async fn send_activity_with_retry(
        &self,
        account_id: &str,
        conversation_id: &str,
        activity: serde_json::Value,
    ) -> ChannelResult<SendOutcome> {
        let snapshot = self.account_snapshot(account_id, conversation_id)?;
        let max_attempts = snapshot.config.max_retries.max(1);
        let base_delay = std::time::Duration::from_millis(snapshot.config.retry_base_delay_ms);
        let max_delay = std::time::Duration::from_millis(snapshot.config.retry_max_delay_ms);

        let mut last_error = None;

        for attempt in 1..=max_attempts {
            let token = get_access_token(&snapshot.http, &snapshot.config, &snapshot.token_cache)
                .await
                .map_err(|e| ChannelError::unavailable(format!("Teams token acquisition: {e}")))?;

            let url = format!(
                "{}/v3/conversations/{}/activities",
                snapshot.service_url.trim_end_matches('/'),
                urlencoding::encode(conversation_id)
            );

            let resp = match snapshot
                .http
                .post(&url)
                .bearer_auth(token.expose_secret())
                .json(&activity)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    if attempt < max_attempts {
                        let delay = errors::compute_retry_delay(
                            attempt,
                            &errors::SendErrorClassification {
                                kind: SendErrorKind::Transient,
                                status_code: None,
                                retry_after: None,
                            },
                            base_delay,
                            max_delay,
                        );
                        debug!(
                            account_id,
                            attempt,
                            delay_ms = delay.as_millis(),
                            "Teams send network error, retrying: {e}"
                        );
                        tokio::time::sleep(delay).await;
                        last_error = Some(format!("network error: {e}"));
                        continue;
                    }
                    return Err(ChannelError::external("Teams HTTP send", e));
                },
            };

            if resp.status().is_success() {
                // Extract activity ID from response.
                let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
                let activity_id = body["id"].as_str().map(String::from);

                #[cfg(feature = "metrics")]
                moltis_metrics::counter!(
                    moltis_metrics::channels::MESSAGES_SENT_TOTAL,
                    moltis_metrics::labels::CHANNEL => "msteams"
                )
                .increment(1);

                return Ok(SendOutcome { activity_id });
            }

            let status = resp.status();
            let headers = resp.headers().clone();
            let body = resp.text().await.unwrap_or_default();
            let classification = errors::classify_send_error(status, &headers);

            if !errors::should_retry(classification.kind) || attempt >= max_attempts {
                return Err(ChannelError::external(
                    "Teams send failed",
                    std::io::Error::other(format!("{status}: {body}")),
                ));
            }

            let delay =
                errors::compute_retry_delay(attempt, &classification, base_delay, max_delay);
            debug!(
                account_id,
                attempt,
                status = %status,
                delay_ms = delay.as_millis(),
                "Teams send failed, retrying"
            );
            tokio::time::sleep(delay).await;
            last_error = Some(format!("{status}: {body}"));
        }

        Err(ChannelError::external(
            "Teams send exhausted retries",
            std::io::Error::other(last_error.unwrap_or_default()),
        ))
    }

    /// Update (edit) an existing activity by ID.
    pub(crate) async fn update_activity(
        &self,
        account_id: &str,
        conversation_id: &str,
        activity_id: &str,
        activity: serde_json::Value,
    ) -> ChannelResult<()> {
        let snapshot = self.account_snapshot(account_id, conversation_id)?;
        let token = get_access_token(&snapshot.http, &snapshot.config, &snapshot.token_cache)
            .await
            .map_err(|e| ChannelError::unavailable(format!("Teams token acquisition: {e}")))?;

        let url = format!(
            "{}/v3/conversations/{}/activities/{}",
            snapshot.service_url.trim_end_matches('/'),
            urlencoding::encode(conversation_id),
            urlencoding::encode(activity_id),
        );

        let resp = snapshot
            .http
            .put(&url)
            .bearer_auth(token.expose_secret())
            .json(&activity)
            .send()
            .await
            .map_err(|e| ChannelError::external("Teams activity update", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::external(
                "Teams activity update failed",
                std::io::Error::other(format!("{status}: {body}")),
            ));
        }

        Ok(())
    }

    /// Send an arbitrary Adaptive Card.
    #[allow(dead_code)]
    pub(crate) async fn send_card(
        &self,
        account_id: &str,
        conversation_id: &str,
        card: serde_json::Value,
        fallback_text: Option<&str>,
        reply_to: Option<&str>,
    ) -> ChannelResult<SendOutcome> {
        let mut activity = crate::cards::card_activity(card, fallback_text);
        if let Some(reply_id) = reply_to {
            activity["replyToId"] = serde_json::Value::String(reply_id.to_string());
        }
        self.send_activity_with_retry(account_id, conversation_id, activity)
            .await
    }

    /// Edit the text of an existing message by activity ID.
    #[allow(dead_code)]
    pub(crate) async fn edit_message(
        &self,
        account_id: &str,
        conversation_id: &str,
        activity_id: &str,
        new_text: &str,
    ) -> ChannelResult<()> {
        let update = serde_json::json!({
            "type": "message",
            "id": activity_id,
            "text": new_text,
        });
        self.update_activity(account_id, conversation_id, activity_id, update)
            .await
    }

    /// Delete an existing activity by ID.
    #[allow(dead_code)]
    pub(crate) async fn delete_activity(
        &self,
        account_id: &str,
        conversation_id: &str,
        activity_id: &str,
    ) -> ChannelResult<()> {
        let snapshot = self.account_snapshot(account_id, conversation_id)?;
        let token = get_access_token(&snapshot.http, &snapshot.config, &snapshot.token_cache)
            .await
            .map_err(|e| ChannelError::unavailable(format!("Teams token acquisition: {e}")))?;

        let url = format!(
            "{}/v3/conversations/{}/activities/{}",
            snapshot.service_url.trim_end_matches('/'),
            urlencoding::encode(conversation_id),
            urlencoding::encode(activity_id),
        );

        let resp = snapshot
            .http
            .delete(&url)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| ChannelError::external("Teams activity delete", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::external(
                "Teams delete failed",
                std::io::Error::other(format!("{status}: {body}")),
            ));
        }

        Ok(())
    }

    /// Send text with chunking support.
    async fn send_text_chunked(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
        chunk_limit: usize,
    ) -> ChannelResult<()> {
        let limit = if chunk_limit == 0 {
            DEFAULT_CHUNK_LIMIT
        } else {
            chunk_limit
        };
        let chunks = chunking::chunk_message(text, limit);

        for chunk in chunks {
            let mut payload = serde_json::json!({
                "type": "message",
                "text": chunk,
            });
            if let Some(reply_to) = reply_to
                && let Some(obj) = payload.as_object_mut()
            {
                obj.insert(
                    "replyToId".into(),
                    serde_json::Value::String(reply_to.to_string()),
                );
            }
            self.send_activity_with_retry(account_id, to, payload)
                .await?;
        }
        Ok(())
    }

    /// Edit-in-place streaming: post, then edit as tokens arrive.
    async fn send_stream_edit_in_place(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
        throttle: std::time::Duration,
        chunk_limit: usize,
    ) -> ChannelResult<()> {
        let mut session = StreamSession::new(throttle);

        loop {
            let event = tokio::select! {
                event = stream.recv() => event,
            };

            match event {
                Some(StreamEvent::Delta(delta)) => {
                    session.push_delta(&delta);

                    if session.ready_for_initial_post() {
                        let activity = streaming::build_initial_activity(
                            &session.text_with_suffix(),
                            reply_to,
                        );
                        match self
                            .send_activity_with_retry(account_id, to, activity)
                            .await
                        {
                            Ok(outcome) => {
                                if let Some(id) = outcome.activity_id {
                                    session.set_activity_id(id);
                                }
                            },
                            Err(e) => {
                                warn!(account_id, to, "Teams stream initial post failed: {e}");
                                session.mark_initial_post_failed();
                                // Fall through — will send final text when done.
                            },
                        }
                    } else if session.ready_for_edit()
                        && let Some(aid) = session.activity_id()
                    {
                        let update =
                            streaming::build_update_activity(aid, &session.text_with_suffix());
                        let _ = self.update_activity(account_id, to, aid, update).await;
                        session.mark_edited();
                    }
                },
                Some(StreamEvent::Error(err)) => {
                    if !session.has_text() {
                        session.push_delta(&err);
                    }
                    break;
                },
                Some(StreamEvent::Done) | None => break,
            }
        }

        session.finalize();

        if !session.has_text() {
            return Ok(());
        }

        let final_text = session.final_text();

        // If we posted an initial message, do a final edit.
        if let Some(aid) = session.activity_id() {
            let update = streaming::build_update_activity(aid, final_text);
            let _ = self.update_activity(account_id, to, aid, update).await;
        } else {
            // Never posted — send the full text as a regular message.
            self.send_text_chunked(account_id, to, final_text, reply_to, chunk_limit)
                .await?;
        }

        Ok(())
    }
}

#[async_trait]
impl ChannelOutbound for MsTeamsOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let chunk_limit = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts
                .get(account_id)
                .map(|s| s.config.text_chunk_limit)
                .unwrap_or(DEFAULT_CHUNK_LIMIT)
        };
        self.send_text_chunked(account_id, to, text, reply_to, chunk_limit)
            .await
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        if let Some(media) = payload.media.as_ref() {
            if media.url.starts_with("data:") {
                // Parse data URL and send as inline attachment.
                if let Some(rest) = media.url.strip_prefix("data:")
                    && let Some((header, b64_data)) = rest.split_once(",")
                {
                    let media_type = header.split(';').next().unwrap_or("image/png");
                    if crate::attachments::is_inline_image(media_type)
                        && let Ok(data) = base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD,
                            b64_data,
                        )
                    {
                        let activity = crate::attachments::build_media_activity(
                            &payload.text,
                            &data,
                            media_type,
                            reply_to,
                        );
                        self.send_activity_with_retry(account_id, to, activity)
                            .await?;
                        return Ok(());
                    }
                }
                // Fall back to text if data URL parsing fails.
                let mut text = payload.text.clone();
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str("[media: image attached]");
                return self.send_text(account_id, to, &text, reply_to).await;
            }

            // External URL — send as URL attachment.
            let mut activity = serde_json::json!({
                "type": "message",
                "attachments": [
                    crate::attachments::build_url_attachment(
                        &media.url,
                        &media.mime_type,
                        media.filename.as_deref(),
                    )
                ],
            });
            if !payload.text.is_empty() {
                activity["text"] = serde_json::Value::String(payload.text.clone());
            }
            if let Some(reply_id) = reply_to {
                activity["replyToId"] = serde_json::Value::String(reply_id.to_string());
            }
            self.send_activity_with_retry(account_id, to, activity)
                .await?;
            return Ok(());
        }

        // No media, just text.
        self.send_text(account_id, to, &payload.text, reply_to)
            .await
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        self.send_activity_with_retry(account_id, to, serde_json::json!({ "type": "typing" }))
            .await?;
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
        let mut merged = text.to_string();
        if !suffix_html.is_empty() {
            merged.push_str("\n\n");
            merged.push_str(suffix_html);
        }
        self.send_text(account_id, to, &merged, reply_to).await
    }

    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.send_text(account_id, to, html, reply_to).await
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
        let mut text = String::new();
        if let Some(title) = title {
            text.push_str(title);
            text.push('\n');
        }
        text.push_str(&format!(
            "https://www.google.com/maps?q={latitude:.6},{longitude:.6}"
        ));
        self.send_text(account_id, to, &text, reply_to).await
    }

    async fn send_interactive(
        &self,
        account_id: &str,
        to: &str,
        message: &moltis_channels::InteractiveMessage,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        // Build an Adaptive Card with action buttons.
        let mut body: Vec<serde_json::Value> = vec![serde_json::json!({
            "type": "TextBlock",
            "text": message.text,
            "wrap": true,
        })];

        for row in &message.button_rows {
            let actions: Vec<serde_json::Value> = row
                .iter()
                .map(|btn| {
                    serde_json::json!({
                        "type": "Action.Submit",
                        "title": btn.label,
                        "data": {
                            "msteams": { "type": "imBack", "value": &btn.callback_data },
                        },
                    })
                })
                .collect();
            body.push(serde_json::json!({
                "type": "ActionSet",
                "actions": actions,
            }));
        }

        let card = serde_json::json!({
            "type": "AdaptiveCard",
            "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
            "version": "1.4",
            "body": body,
        });

        let activity = crate::cards::card_activity(card, Some(&message.text));
        if let Some(reply_id) = reply_to {
            let mut a = activity;
            a["replyToId"] = serde_json::Value::String(reply_id.to_string());
            self.send_activity_with_retry(account_id, to, a).await?;
        } else {
            self.send_activity_with_retry(account_id, to, activity)
                .await?;
        }
        Ok(())
    }

    async fn add_reaction(
        &self,
        account_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> ChannelResult<()> {
        let (http, config, graph_cache) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let state = accounts
                .get(account_id)
                .ok_or_else(|| ChannelError::unknown_account(account_id))?;
            (
                state.http.clone(),
                state.config.clone(),
                std::sync::Arc::clone(&state.graph_token_cache),
            )
        };

        let token = crate::auth::get_graph_token(&http, &config, &graph_cache)
            .await
            .map_err(|e| ChannelError::unavailable(format!("Teams Graph token: {e}")))?;

        crate::graph::add_reaction(&http, &token, channel_id, message_id, emoji)
            .await
            .map_err(|e| {
                ChannelError::external("Teams add reaction", std::io::Error::other(e.to_string()))
            })
    }

    async fn remove_reaction(
        &self,
        account_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> ChannelResult<()> {
        let (http, config, graph_cache) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let state = accounts
                .get(account_id)
                .ok_or_else(|| ChannelError::unknown_account(account_id))?;
            (
                state.http.clone(),
                state.config.clone(),
                std::sync::Arc::clone(&state.graph_token_cache),
            )
        };

        let token = crate::auth::get_graph_token(&http, &config, &graph_cache)
            .await
            .map_err(|e| ChannelError::unavailable(format!("Teams Graph token: {e}")))?;

        crate::graph::remove_reaction(&http, &token, channel_id, message_id, emoji)
            .await
            .map_err(|e| {
                ChannelError::external(
                    "Teams remove reaction",
                    std::io::Error::other(e.to_string()),
                )
            })
    }
}

#[async_trait]
impl ChannelStreamOutbound for MsTeamsOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        let (stream_mode, throttle, chunk_limit) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            match accounts.get(account_id) {
                Some(state) => (
                    state.config.stream_mode.clone(),
                    std::time::Duration::from_millis(state.config.edit_throttle_ms),
                    state.config.text_chunk_limit,
                ),
                None => (
                    StreamMode::Off,
                    streaming::DEFAULT_EDIT_THROTTLE,
                    DEFAULT_CHUNK_LIMIT,
                ),
            }
        };

        match stream_mode {
            StreamMode::EditInPlace => {
                self.send_stream_edit_in_place(
                    account_id,
                    to,
                    reply_to,
                    stream,
                    throttle,
                    chunk_limit,
                )
                .await
            },
            StreamMode::Off => {
                // Accumulate and send once (original behaviour).
                let mut text = String::new();
                while let Some(event) = stream.recv().await {
                    match event {
                        StreamEvent::Delta(delta) => text.push_str(&delta),
                        StreamEvent::Done => break,
                        StreamEvent::Error(err) => {
                            debug!(account_id, chat_id = to, "Teams stream error: {err}");
                            if text.is_empty() {
                                text = err;
                            }
                            break;
                        },
                    }
                }
                if text.is_empty() {
                    return Ok(());
                }
                self.send_text_chunked(account_id, to, &text, reply_to, chunk_limit)
                    .await
            },
        }
    }

    async fn is_stream_enabled(&self, account_id: &str) -> bool {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .is_some_and(|s| s.config.stream_mode == StreamMode::EditInPlace)
    }
}
