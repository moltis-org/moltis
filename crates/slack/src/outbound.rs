use std::time::Duration;

use {
    async_trait::async_trait,
    base64::Engine,
    secrecy::ExposeSecret,
    slack_morphism::prelude::*,
    tracing::{debug, warn},
};

use moltis_channels::{
    Error as ChannelError, Result as ChannelResult,
    plugin::{
        ButtonStyle, ChannelOutbound, ChannelStreamOutbound, ChannelThreadContext,
        InteractiveMessage, StreamEvent, StreamReceiver, ThreadMessage,
    },
};

use moltis_common::types::ReplyPayload;

use crate::{
    config::StreamMode,
    markdown::{SLACK_MAX_MESSAGE_LEN, chunk_message, markdown_to_slack},
    state::AccountStateMap,
};

/// Minimum chars before the first message is sent during streaming.
const STREAM_MIN_INITIAL_CHARS: usize = 30;

/// Slack outbound message sender.
pub struct SlackOutbound {
    pub(crate) accounts: AccountStateMap,
}

impl SlackOutbound {
    /// Get a Slack client session for the given account.
    fn get_session(
        &self,
        account_id: &str,
    ) -> ChannelResult<(SlackClient<SlackClientHyperHttpsConnector>, SlackApiToken)> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;

        let token_str = state.config.bot_token.expose_secret().clone();
        let token = SlackApiToken::new(SlackApiTokenValue::from(token_str));

        let client = SlackClient::new(
            SlackClientHyperConnector::new()
                .map_err(|e| ChannelError::unavailable(format!("hyper connector: {e}")))?,
        );

        Ok((client, token))
    }

    /// Get the thread_ts for reply threading.
    fn get_thread_ts(&self, account_id: &str, to: &str, reply_to: Option<&str>) -> Option<String> {
        // If we have an explicit reply_to (message_id), use that as thread_ts.
        if let Some(ts) = reply_to {
            return Some(ts.to_string());
        }

        // Check if thread_replies is enabled and we have a stored thread_ts.
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts.get(account_id)?;
        if !state.config.thread_replies {
            return None;
        }
        // Look up by channel_id (any user).
        state
            .pending_threads
            .iter()
            .find(|(k, _)| k.starts_with(&format!("{to}:")))
            .map(|(_, ts)| ts.clone())
    }

    /// Get the edit throttle duration for streaming.
    fn get_edit_throttle(&self, account_id: &str) -> Duration {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| Duration::from_millis(s.config.edit_throttle_ms))
            .unwrap_or(Duration::from_millis(500))
    }

    /// Get the stream mode for the given account.
    fn get_stream_mode(&self, account_id: &str) -> StreamMode {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| s.config.stream_mode.clone())
            .unwrap_or_default()
    }

    /// Get the raw bot token string for API calls not covered by slack-morphism.
    fn get_bot_token(&self, account_id: &str) -> ChannelResult<String> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;
        Ok(state.config.bot_token.expose_secret().clone())
    }

    /// Native Slack streaming using chat.startStream/appendStream/stopStream.
    async fn send_stream_native(
        &self,
        account_id: &str,
        to: &str,
        thread_ts: Option<&str>,
        stream: &mut StreamReceiver,
    ) -> ChannelResult<()> {
        let bot_token = self.get_bot_token(account_id)?;
        let http = reqwest::Client::new();
        let throttle = self.get_edit_throttle(account_id);

        let stream_id = start_native_stream(&http, &bot_token, to, thread_ts).await?;

        let mut pending = String::new();
        let mut last_append = tokio::time::Instant::now();

        loop {
            match stream.recv().await {
                Some(StreamEvent::Delta(chunk)) => {
                    pending.push_str(&chunk);

                    // Throttle appends to avoid rate limits.
                    if last_append.elapsed() >= throttle {
                        let text = markdown_to_slack(&std::mem::take(&mut pending));
                        if !text.is_empty()
                            && let Err(e) =
                                append_native_stream(&http, &bot_token, &stream_id, &text).await
                        {
                            debug!(account_id, to, "chat.appendStream failed (will retry): {e}");
                            // Put the text back for next attempt.
                            pending = text;
                        }
                        last_append = tokio::time::Instant::now();
                    }
                },
                Some(StreamEvent::Done) => break,
                Some(StreamEvent::Error(e)) => {
                    pending.push_str(&format!("\n\n:warning: {e}"));
                    break;
                },
                None => break,
            }
        }

        // Flush any remaining text.
        if !pending.is_empty() {
            let text = markdown_to_slack(&pending);
            if let Err(e) = append_native_stream(&http, &bot_token, &stream_id, &text).await {
                warn!(account_id, to, "final chat.appendStream failed: {e}");
            }
        }

        // Finalize the stream.
        if let Err(e) = stop_native_stream(&http, &bot_token, &stream_id).await {
            warn!(account_id, to, "chat.stopStream failed: {e}");
        }

        Ok(())
    }

    /// Edit-in-place streaming: post → throttled edits → final update.
    async fn send_stream_edit_in_place(
        &self,
        account_id: &str,
        to: &str,
        thread_ts: Option<&str>,
        stream: &mut StreamReceiver,
    ) -> ChannelResult<()> {
        let (client, token) = self.get_session(account_id)?;
        let throttle = self.get_edit_throttle(account_id);

        let mut accumulated = String::new();
        let mut sent_ts: Option<SlackTs> = None;
        let mut last_edit = tokio::time::Instant::now();

        loop {
            match stream.recv().await {
                Some(StreamEvent::Delta(chunk)) => {
                    accumulated.push_str(&chunk);

                    match &sent_ts {
                        None => {
                            if accumulated.len() >= STREAM_MIN_INITIAL_CHARS {
                                let slack_text = markdown_to_slack(&accumulated);
                                match post_message(
                                    &client,
                                    &token,
                                    to,
                                    &format!("{slack_text}..."),
                                    thread_ts,
                                )
                                .await
                                {
                                    Ok(ts) => {
                                        sent_ts = Some(ts);
                                        last_edit = tokio::time::Instant::now();
                                    },
                                    Err(e) => {
                                        warn!(
                                            account_id,
                                            to, "failed to send initial stream message: {e}"
                                        );
                                    },
                                }
                            }
                        },
                        Some(ts) => {
                            if last_edit.elapsed() >= throttle {
                                let slack_text = markdown_to_slack(&accumulated);
                                let display = if slack_text.len() > SLACK_MAX_MESSAGE_LEN - 3 {
                                    format!(
                                        "{}...",
                                        &slack_text[..slack_text
                                            .floor_char_boundary(SLACK_MAX_MESSAGE_LEN - 3)]
                                    )
                                } else {
                                    format!("{slack_text}...")
                                };

                                if let Err(e) =
                                    update_message(&client, &token, to, ts, &display).await
                                {
                                    debug!(
                                        account_id,
                                        to, "stream edit-in-place failed (will retry): {e}"
                                    );
                                }
                                last_edit = tokio::time::Instant::now();
                            }
                        },
                    }
                },
                Some(StreamEvent::Done) => break,
                Some(StreamEvent::Error(e)) => {
                    accumulated.push_str(&format!("\n\n:warning: {e}"));
                    break;
                },
                None => break,
            }
        }

        if accumulated.is_empty() {
            return Ok(());
        }

        let final_text = markdown_to_slack(&accumulated);
        let chunks = chunk_message(&final_text, SLACK_MAX_MESSAGE_LEN);

        match &sent_ts {
            Some(ts) => {
                if let Some(first) = chunks.first()
                    && let Err(e) = update_message(&client, &token, to, ts, first).await
                {
                    warn!(account_id, to, "failed to finalize stream message: {e}");
                }
                for chunk in chunks.iter().skip(1) {
                    if let Err(e) = post_message(&client, &token, to, chunk, thread_ts).await {
                        warn!(account_id, to, "failed to send overflow chunk: {e}");
                    }
                }
            },
            None => {
                for chunk in &chunks {
                    if let Err(e) = post_message(&client, &token, to, chunk, thread_ts).await {
                        warn!(account_id, to, "failed to send stream message: {e}");
                    }
                }
            },
        }

        Ok(())
    }
}

/// Post a message to a Slack channel.
async fn post_message(
    client: &SlackClient<SlackClientHyperHttpsConnector>,
    token: &SlackApiToken,
    channel: &str,
    text: &str,
    thread_ts: Option<&str>,
) -> ChannelResult<SlackTs> {
    let session = client.open_session(token);
    let channel_id: SlackChannelId = channel.into();

    let mut req = SlackApiChatPostMessageRequest::new(
        channel_id,
        SlackMessageContent::new().with_text(text.to_string()),
    );

    if let Some(ts) = thread_ts {
        req = req.with_thread_ts(ts.into());
    }

    let resp = session
        .chat_post_message(&req)
        .await
        .map_err(|e| ChannelError::unavailable(format!("chat.postMessage failed: {e}")))?;

    Ok(resp.ts)
}

/// Update an existing message.
async fn update_message(
    client: &SlackClient<SlackClientHyperHttpsConnector>,
    token: &SlackApiToken,
    channel: &str,
    ts: &SlackTs,
    text: &str,
) -> ChannelResult<()> {
    let session = client.open_session(token);
    let channel_id: SlackChannelId = channel.into();

    let req = SlackApiChatUpdateRequest::new(
        channel_id,
        SlackMessageContent::new().with_text(text.to_string()),
        ts.clone(),
    );

    session
        .chat_update(&req)
        .await
        .map_err(|e| ChannelError::unavailable(format!("chat.update failed: {e}")))?;

    Ok(())
}

/// Decode a `data:<mime>;base64,<payload>` URI into raw bytes.
fn decode_data_url(url: &str) -> ChannelResult<(Vec<u8>, String)> {
    let comma = url
        .find(',')
        .ok_or_else(|| ChannelError::invalid_input("malformed data URL: no comma"))?;
    let header = &url[..comma];
    let mime = header
        .strip_prefix("data:")
        .and_then(|s| s.strip_suffix(";base64"))
        .unwrap_or("application/octet-stream");
    let payload = &url[comma + 1..];
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| ChannelError::invalid_input(format!("base64 decode error: {e}")))?;
    Ok((bytes, mime.to_string()))
}

/// Map MIME type to a file extension.
fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "audio/ogg" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "video/mp4" => "mp4",
        "application/pdf" => "pdf",
        _ => "bin",
    }
}

/// Upload a file to Slack using the V2 upload flow.
async fn upload_file(
    client: &SlackClient<SlackClientHyperHttpsConnector>,
    token: &SlackApiToken,
    channel: &str,
    filename: &str,
    content_type: &str,
    data: &[u8],
    caption: Option<&str>,
    thread_ts: Option<&str>,
) -> ChannelResult<()> {
    let session = client.open_session(token);

    // Step 1: Get the upload URL.
    let upload_req =
        SlackApiFilesGetUploadUrlExternalRequest::new(filename.to_string(), data.len());
    let upload_resp = session
        .get_upload_url_external(&upload_req)
        .await
        .map_err(|e| ChannelError::unavailable(format!("getUploadURLExternal failed: {e}")))?;

    // Step 2: Upload file bytes.
    let via_req = SlackApiFilesUploadViaUrlRequest::new(
        upload_resp.upload_url,
        data.to_vec(),
        content_type.to_string(),
    );
    session
        .files_upload_via_url(&via_req)
        .await
        .map_err(|e| ChannelError::unavailable(format!("file upload PUT failed: {e}")))?;

    // Step 3: Complete the upload — attach to channel.
    let file_complete = SlackApiFilesComplete::new(upload_resp.file_id);
    let mut complete_req = SlackApiFilesCompleteUploadExternalRequest::new(vec![file_complete])
        .with_channel_id(channel.into());
    if let Some(comment) = caption {
        complete_req = complete_req.with_initial_comment(comment.to_string());
    }
    if let Some(ts) = thread_ts {
        complete_req = complete_req.with_thread_ts(ts.into());
    }
    session
        .files_complete_upload_external(&complete_req)
        .await
        .map_err(|e| ChannelError::unavailable(format!("completeUploadExternal failed: {e}")))?;

    Ok(())
}

/// Add or remove a reaction on a Slack message using the Web API.
async fn modify_reaction(
    client: &SlackClient<SlackClientHyperHttpsConnector>,
    token: &SlackApiToken,
    channel: &str,
    timestamp: &str,
    emoji: &str,
    add: bool,
) -> ChannelResult<()> {
    let session = client.open_session(token);
    let channel_id: SlackChannelId = channel.into();
    let ts: SlackTs = timestamp.into();
    let reaction = SlackReactionName::new(emoji.to_string());

    if add {
        let req = SlackApiReactionsAddRequest::new(channel_id, reaction, ts);
        session
            .reactions_add(&req)
            .await
            .map_err(|e| ChannelError::unavailable(format!("reactions.add failed: {e}")))?;
    } else {
        let req = SlackApiReactionsRemoveRequest::new(reaction)
            .with_channel(channel_id)
            .with_timestamp(ts);
        session
            .reactions_remove(&req)
            .await
            .map_err(|e| ChannelError::unavailable(format!("reactions.remove failed: {e}")))?;
    }

    Ok(())
}

#[async_trait]
impl ChannelOutbound for SlackOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let (client, token) = self.get_session(account_id)?;
        let thread_ts = self.get_thread_ts(account_id, to, reply_to);
        let slack_text = markdown_to_slack(text);

        let chunks = chunk_message(&slack_text, SLACK_MAX_MESSAGE_LEN);
        for chunk in chunks {
            post_message(&client, &token, to, chunk, thread_ts.as_deref()).await?;
        }

        #[cfg(feature = "metrics")]
        moltis_metrics::counter!(
            moltis_metrics::channels::MESSAGES_SENT_TOTAL,
            moltis_metrics::labels::CHANNEL => "slack"
        )
        .increment(1);

        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let media_url = payload.media.as_ref().map(|m| m.url.as_str());

        match media_url {
            Some(url) if url.starts_with("data:") => {
                let (data, mime) = decode_data_url(url)?;
                let ext = extension_for_mime(&mime);
                let filename = format!("file.{ext}");
                let caption = if payload.text.is_empty() {
                    None
                } else {
                    Some(payload.text.as_str())
                };

                let (client, token) = self.get_session(account_id)?;
                let thread_ts = self.get_thread_ts(account_id, to, reply_to);

                upload_file(
                    &client,
                    &token,
                    to,
                    &filename,
                    &mime,
                    &data,
                    caption,
                    thread_ts.as_deref(),
                )
                .await
            },
            Some(url) => {
                // Regular URL — append to text and send.
                let text = if payload.text.is_empty() {
                    url.to_string()
                } else {
                    format!("{}\n{url}", payload.text)
                };
                self.send_text(account_id, to, &text, reply_to).await
            },
            None => {
                // No media — send text only.
                let text = if payload.text.is_empty() {
                    "(media attachment)".to_string()
                } else {
                    payload.text.clone()
                };
                self.send_text(account_id, to, &text, reply_to).await
            },
        }
    }

    async fn send_typing(&self, _account_id: &str, _to: &str) -> ChannelResult<()> {
        // Slack bots cannot show typing indicators.
        Ok(())
    }

    async fn send_interactive(
        &self,
        account_id: &str,
        to: &str,
        message: &InteractiveMessage,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let (client, token) = self.get_session(account_id)?;
        let thread_ts = self.get_thread_ts(account_id, to, reply_to);
        let session = client.open_session(&token);
        let channel_id: SlackChannelId = to.into();

        // Build Block Kit blocks: text section + actions per row.
        let mut blocks = vec![serde_json::json!({
            "type": "section",
            "text": { "type": "mrkdwn", "text": message.text },
        })];

        for row in &message.button_rows {
            let elements: Vec<serde_json::Value> = row
                .iter()
                .map(|btn| {
                    let mut button = serde_json::json!({
                        "type": "button",
                        "text": { "type": "plain_text", "text": btn.label },
                        "action_id": btn.callback_data,
                    });
                    match btn.style {
                        ButtonStyle::Primary => {
                            button["style"] = serde_json::json!("primary");
                        },
                        ButtonStyle::Danger => {
                            button["style"] = serde_json::json!("danger");
                        },
                        ButtonStyle::Default => {},
                    }
                    button
                })
                .collect();

            blocks.push(serde_json::json!({
                "type": "actions",
                "elements": elements,
            }));
        }

        let content = SlackMessageContent::new().with_text(message.text.clone());

        let mut req = SlackApiChatPostMessageRequest::new(channel_id, content);

        if let Some(ts) = thread_ts.as_deref() {
            req = req.with_thread_ts(ts.into());
        }

        // Attach blocks via raw JSON since slack-morphism's typed Block Kit
        // builders don't cover all action element styles easily.
        let mut body = serde_json::to_value(&req)
            .map_err(|e| ChannelError::unavailable(format!("serialize failed: {e}")))?;
        body["blocks"] = serde_json::json!(blocks);

        // Use the raw post approach.  Fall back to text-only if it fails.
        let raw_resp: serde_json::Value = session
            .http_session_api
            .http_post("chat.postMessage", &body, None)
            .await
            .map_err(|e| {
                ChannelError::unavailable(format!("chat.postMessage (interactive) failed: {e}"))
            })?;

        if raw_resp.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let err = raw_resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(ChannelError::unavailable(format!(
                "chat.postMessage (interactive) error: {err}"
            )));
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
        let (client, token) = self.get_session(account_id)?;
        modify_reaction(&client, &token, channel_id, message_id, emoji, true).await
    }

    async fn remove_reaction(
        &self,
        account_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> ChannelResult<()> {
        let (client, token) = self.get_session(account_id)?;
        modify_reaction(&client, &token, channel_id, message_id, emoji, false).await
    }
}

/// Start a native Slack stream via `chat.startStream`.
///
/// Returns `(stream_id, channel)` on success.
async fn start_native_stream(
    http: &reqwest::Client,
    bot_token: &str,
    channel: &str,
    thread_ts: Option<&str>,
) -> ChannelResult<String> {
    let mut body = serde_json::json!({ "channel": channel });
    if let Some(ts) = thread_ts {
        body["thread_ts"] = serde_json::json!(ts);
    }

    let resp = http
        .post("https://slack.com/api/chat.startStream")
        .bearer_auth(bot_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::external("chat.startStream", e))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ChannelError::external("chat.startStream parse", e))?;

    if json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = json
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(ChannelError::unavailable(format!(
            "chat.startStream failed: {err}"
        )));
    }

    json.get("stream_id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| ChannelError::unavailable("chat.startStream: missing stream_id"))
}

/// Append text to a native Slack stream via `chat.appendStream`.
async fn append_native_stream(
    http: &reqwest::Client,
    bot_token: &str,
    stream_id: &str,
    text: &str,
) -> ChannelResult<()> {
    let body = serde_json::json!({
        "stream_id": stream_id,
        "text": text,
    });

    let resp = http
        .post("https://slack.com/api/chat.appendStream")
        .bearer_auth(bot_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::external("chat.appendStream", e))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ChannelError::external("chat.appendStream parse", e))?;

    if json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = json
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(ChannelError::unavailable(format!(
            "chat.appendStream failed: {err}"
        )));
    }

    Ok(())
}

/// Finalize a native Slack stream via `chat.stopStream`.
async fn stop_native_stream(
    http: &reqwest::Client,
    bot_token: &str,
    stream_id: &str,
) -> ChannelResult<()> {
    let body = serde_json::json!({ "stream_id": stream_id });

    let resp = http
        .post("https://slack.com/api/chat.stopStream")
        .bearer_auth(bot_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::external("chat.stopStream", e))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ChannelError::external("chat.stopStream parse", e))?;

    if json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = json
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(ChannelError::unavailable(format!(
            "chat.stopStream failed: {err}"
        )));
    }

    Ok(())
}

#[async_trait]
impl ChannelStreamOutbound for SlackOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        let stream_mode = self.get_stream_mode(account_id);
        let thread_ts = self.get_thread_ts(account_id, to, reply_to);

        match stream_mode {
            StreamMode::Native => {
                self.send_stream_native(account_id, to, thread_ts.as_deref(), &mut stream)
                    .await
            },
            StreamMode::EditInPlace => {
                self.send_stream_edit_in_place(account_id, to, thread_ts.as_deref(), &mut stream)
                    .await
            },
            StreamMode::Off => {
                // Streaming disabled — accumulate and send once.
                let mut accumulated = String::new();
                loop {
                    match stream.recv().await {
                        Some(StreamEvent::Delta(chunk)) => accumulated.push_str(&chunk),
                        Some(StreamEvent::Error(e)) => {
                            accumulated.push_str(&format!("\n\n:warning: {e}"));
                            break;
                        },
                        Some(StreamEvent::Done) | None => break,
                    }
                }
                if !accumulated.is_empty() {
                    let (client, token) = self.get_session(account_id)?;
                    let final_text = markdown_to_slack(&accumulated);
                    for chunk in chunk_message(&final_text, SLACK_MAX_MESSAGE_LEN) {
                        if let Err(e) =
                            post_message(&client, &token, to, chunk, thread_ts.as_deref()).await
                        {
                            warn!(account_id, to, "failed to send stream message: {e}");
                        }
                    }
                }
                Ok(())
            },
        }
    }

    async fn is_stream_enabled(&self, account_id: &str) -> bool {
        self.get_stream_mode(account_id) != StreamMode::Off
    }
}

#[async_trait]
impl ChannelThreadContext for SlackOutbound {
    async fn fetch_thread_messages(
        &self,
        account_id: &str,
        channel_id: &str,
        thread_id: &str,
        limit: usize,
    ) -> ChannelResult<Vec<ThreadMessage>> {
        let (client, token) = self.get_session(account_id)?;
        let session = client.open_session(&token);

        let req = SlackApiConversationsRepliesRequest::new(channel_id.into(), thread_id.into())
            .with_limit(limit.min(200) as u16);

        let resp = session
            .conversations_replies(&req)
            .await
            .map_err(|e| ChannelError::unavailable(format!("conversations.replies failed: {e}")))?;

        let messages = resp
            .messages
            .into_iter()
            .map(|msg| {
                let sender_id = msg
                    .sender
                    .user
                    .as_ref()
                    .map(|u| u.to_string())
                    .unwrap_or_default();
                let is_bot = msg.sender.bot_id.is_some() || msg.sender.display_as_bot == Some(true);
                let text = msg.content.text.unwrap_or_default();
                let timestamp = msg.origin.ts.to_string();

                ThreadMessage {
                    sender_id,
                    is_bot,
                    text,
                    timestamp,
                }
            })
            .collect();

        Ok(messages)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn get_thread_ts_from_reply_to() {
        let accounts =
            std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));
        let outbound = SlackOutbound {
            accounts: accounts.clone(),
        };
        // reply_to takes precedence.
        let ts = outbound.get_thread_ts("acct", "C123", Some("1234567.890"));
        assert_eq!(ts, Some("1234567.890".to_string()));
    }

    #[test]
    fn get_thread_ts_no_account() {
        let accounts =
            std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));
        let outbound = SlackOutbound { accounts };
        let ts = outbound.get_thread_ts("acct", "C123", None);
        assert!(ts.is_none());
    }

    #[test]
    fn decode_data_url_png() {
        // Minimal 1x1 red PNG encoded as base64 data URL.
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"fakepng");
        let url = format!("data:image/png;base64,{b64}");
        let (bytes, mime) = decode_data_url(&url).unwrap();
        assert_eq!(bytes, b"fakepng");
        assert_eq!(mime, "image/png");
    }

    #[test]
    fn decode_data_url_no_comma_fails() {
        assert!(decode_data_url("data:image/pngbase64abc").is_err());
    }

    #[test]
    fn extension_for_known_mimes() {
        assert_eq!(extension_for_mime("image/png"), "png");
        assert_eq!(extension_for_mime("image/jpeg"), "jpg");
        assert_eq!(extension_for_mime("application/pdf"), "pdf");
        assert_eq!(extension_for_mime("text/plain"), "bin");
    }
}
