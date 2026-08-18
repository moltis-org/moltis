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
    client::slack_client_for_base_url,
    config::StreamMode,
    markdown::{SLACK_MAX_MESSAGE_LEN, chunk_message, markdown_to_slack},
    native_stream::{HttpNativeStreamApi, send_native_stream},
    state::{AccountStateMap, StreamRecipient},
};

/// Minimum chars before the first message is sent during streaming.
const STREAM_MIN_INITIAL_CHARS: usize = 30;

/// Shared HTTP client for raw Slack Web API calls (native streaming, assistant
/// status). `reqwest::Client` pools connections and is cheap to clone, so build
/// it once instead of per call — `send_typing` in particular is invoked on a
/// repeating loop while a turn runs.
fn shared_http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(moltis_common::http_client::build_default_http_client)
        .clone()
}

fn validate_response_url(url: &str) -> ChannelResult<reqwest::Url> {
    let uri = url.parse::<http::Uri>().map_err(|error| {
        ChannelError::invalid_input(format!("invalid Slack response_url: {error}"))
    })?;
    let url = reqwest::Url::parse(url).map_err(|error| {
        ChannelError::invalid_input(format!("invalid Slack response_url: {error}"))
    })?;
    if url.scheme() != "https" {
        return Err(ChannelError::invalid_input(
            "Slack response_url must use HTTPS",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ChannelError::invalid_input(
            "Slack response_url must not contain credentials",
        ));
    }
    if uri
        .authority()
        .and_then(http::uri::Authority::port_u16)
        .is_some()
    {
        return Err(ChannelError::invalid_input(
            "Slack response_url must not contain a port",
        ));
    }
    if !matches!(
        url.host_str(),
        Some("hooks.slack.com" | "hooks.slack-gov.com")
    ) {
        return Err(ChannelError::invalid_input(
            "Slack response_url host is not approved",
        ));
    }
    Ok(url)
}

fn response_url_http_client() -> ChannelResult<&'static reqwest::Client> {
    static CLIENT: std::sync::OnceLock<Result<reqwest::Client, String>> =
        std::sync::OnceLock::new();
    match CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| error.to_string())
    }) {
        Ok(client) => Ok(client),
        Err(error) => Err(ChannelError::unavailable(format!(
            "failed to build Slack response_url client: {error}"
        ))),
    }
}

pub(crate) async fn post_response_url(url: &str, text: &str) -> ChannelResult<()> {
    let url = validate_response_url(url)?;
    let response = response_url_http_client()?
        .post(url)
        .json(&serde_json::json!({
            "response_type": "ephemeral",
            "replace_original": false,
            "text": text,
        }))
        .send()
        .await
        .map_err(|error| ChannelError::external("Slack response_url", error))?;
    if !response.status().is_success() {
        return Err(ChannelError::unavailable(format!(
            "Slack response_url returned HTTP {}",
            response.status()
        )));
    }
    Ok(())
}

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

        let client = slack_client_for_base_url(&state.config.api_base_url)?;

        Ok((client, token))
    }

    /// Apply the account's thread reply preference to a normal outbound reply.
    /// The exact inbound root remains available separately for context and the
    /// native streaming API, which requires a thread identity.
    fn get_reply_thread_ts(&self, account_id: &str, reply_to: Option<&str>) -> Option<String> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .is_some_and(|state| state.config.thread_replies)
            .then(|| reply_to.map(String::from))
            .flatten()
    }

    /// Whether Block Kit rich rendering is enabled for the given account.
    fn get_rich_blocks(&self, account_id: &str) -> bool {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| s.config.rich_blocks)
            .unwrap_or(false)
    }

    /// Get the stream mode for the given account.
    fn get_stream_mode(&self, account_id: &str) -> StreamMode {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| s.config.stream_mode.clone())
            .unwrap_or_default()
    }

    /// Get the edit throttle duration for edit-in-place streaming.
    fn get_edit_throttle(&self, account_id: &str) -> Duration {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| Duration::from_millis(s.config.edit_throttle_ms))
            .unwrap_or(Duration::from_millis(500))
    }

    /// Get native streaming settings in one account lookup.
    fn get_native_stream_config(
        &self,
        account_id: &str,
        channel: &str,
        thread_ts: &str,
    ) -> ChannelResult<(String, String, Duration, Option<StreamRecipient>)> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;
        Ok((
            state.config.bot_token.expose_secret().clone(),
            state.config.api_base_url.clone(),
            Duration::from_millis(state.config.edit_throttle_ms),
            state.stream_recipient(channel, thread_ts).cloned(),
        ))
    }

    /// Native Slack streaming using chat.startStream/appendStream/stopStream.
    async fn send_stream_native(
        &self,
        account_id: &str,
        to: &str,
        thread_ts: &str,
        stream: &mut StreamReceiver,
    ) -> ChannelResult<Vec<String>> {
        let (bot_token, api_base_url, throttle, recipient) =
            self.get_native_stream_config(account_id, to, thread_ts)?;
        let api = HttpNativeStreamApi::new(shared_http_client(), api_base_url, bot_token);
        send_native_stream(&api, to, thread_ts, recipient.as_ref(), throttle, stream).await
    }

    /// Edit-in-place streaming: post → throttled edits → final update.
    async fn send_stream_edit_in_place(
        &self,
        account_id: &str,
        to: &str,
        thread_ts: Option<&str>,
        stream: &mut StreamReceiver,
    ) -> ChannelResult<Vec<String>> {
        let (client, token) = self.get_session(account_id)?;
        let throttle = self.get_edit_throttle(account_id);

        let mut accumulated = String::new();
        let mut sent_ts: Option<SlackTs> = None;
        let mut last_edit = tokio::time::Instant::now();

        loop {
            match stream.recv().await {
                Some(StreamEvent::Delta(chunk) | StreamEvent::ProgressDelta(chunk)) => {
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
                Some(StreamEvent::TaskUpdate(_)) => {},
                Some(StreamEvent::Done) => break,
                Some(StreamEvent::Error(e)) => {
                    accumulated.push_str(&format!("\n\n:warning: {e}"));
                    break;
                },
                None => break,
            }
        }

        if accumulated.is_empty() {
            return Ok(Vec::new());
        }

        let final_text = markdown_to_slack(&accumulated);
        let chunks = chunk_message(&final_text, SLACK_MAX_MESSAGE_LEN);
        let mut ids = Vec::with_capacity(chunks.len());

        // Final delivery failures are returned, not swallowed: the caller uses
        // the result to decide whether the reply actually landed.
        match &sent_ts {
            Some(ts) => {
                if let Some(first) = chunks.first() {
                    update_message(&client, &token, to, ts, first)
                        .await
                        .inspect_err(|e| {
                            warn!(account_id, to, "failed to finalize stream message: {e}");
                        })?;
                    ids.push(ts.to_string());
                }
                for chunk in chunks.iter().skip(1) {
                    let ts = post_message(&client, &token, to, chunk, thread_ts)
                        .await
                        .inspect_err(|e| {
                            warn!(account_id, to, "failed to send overflow chunk: {e}")
                        })?;
                    ids.push(ts.to_string());
                }
            },
            None => {
                for chunk in &chunks {
                    let ts = post_message(&client, &token, to, chunk, thread_ts)
                        .await
                        .inspect_err(|e| {
                            warn!(account_id, to, "failed to send stream message: {e}")
                        })?;
                    ids.push(ts.to_string());
                }
            },
        }

        Ok(ids)
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

/// Post a message rendered as Block Kit blocks, with `fallback_text` used for
/// notifications and clients that cannot render blocks.
async fn post_message_with_blocks(
    client: &SlackClient<SlackClientHyperHttpsConnector>,
    token: &SlackApiToken,
    channel: &str,
    fallback_text: &str,
    blocks: &[serde_json::Value],
    thread_ts: Option<&str>,
) -> ChannelResult<SlackTs> {
    let session = client.open_session(token);
    let mut body = serde_json::json!({
        "channel": channel,
        "text": fallback_text,
        "blocks": blocks,
    });
    if let Some(ts) = thread_ts {
        body["thread_ts"] = serde_json::json!(ts);
    }

    let resp: serde_json::Value = session
        .http_session_api
        .http_post("chat.postMessage", &body, None)
        .await
        .map_err(|e| ChannelError::unavailable(format!("chat.postMessage (blocks) failed: {e}")))?;

    if resp.get("ok") == Some(&serde_json::Value::Bool(false)) {
        let err = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(ChannelError::unavailable(format!(
            "chat.postMessage (blocks) error: {err}"
        )));
    }
    let ts = resp
        .get("ts")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ChannelError::unavailable("chat.postMessage (blocks) did not return ts"))?;
    Ok(SlackTs(ts.to_string()))
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
#[allow(dead_code)]
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
    upload_file_reporting_ids(
        client,
        token,
        channel,
        filename,
        content_type,
        data,
        caption,
        thread_ts,
    )
    .await?;
    Ok(())
}

/// Upload a file to Slack and report identifiers that Slack reactions can carry.
async fn upload_file_reporting_ids(
    client: &SlackClient<SlackClientHyperHttpsConnector>,
    token: &SlackApiToken,
    channel: &str,
    filename: &str,
    content_type: &str,
    data: &[u8],
    caption: Option<&str>,
    thread_ts: Option<&str>,
) -> ChannelResult<Vec<String>> {
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
    let complete_resp = session
        .files_complete_upload_external(&complete_req)
        .await
        .map_err(|e| ChannelError::unavailable(format!("completeUploadExternal failed: {e}")))?;

    Ok(complete_resp
        .files
        .into_iter()
        .map(|file| file.id.0)
        .collect())
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
    // Slack expects shortcodes (no colons, no raw glyphs, no skin-tone suffix).
    let reaction =
        SlackReactionName::new(crate::emoji::normalize_reaction_name(emoji).into_owned());

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
        let thread_ts = self.get_reply_thread_ts(account_id, reply_to);
        let slack_text = markdown_to_slack(text);

        let chunks = chunk_message(&slack_text, SLACK_MAX_MESSAGE_LEN);

        // Rich Block Kit rendering (opt-in). Only used when the whole reply fits
        // a single message: the `text` fallback must carry the *entire* content
        // for notifications and clients that cannot render blocks, so a reply
        // that needs chunking is sent as plain text instead of silently
        // delivering only its first chunk.
        let rendered_blocks = (self.get_rich_blocks(account_id) && chunks.len() == 1)
            .then(|| crate::blocks::markdown_to_blocks(text))
            .flatten();

        if let Some(blocks) = rendered_blocks {
            let fallback = chunks.first().copied().unwrap_or(&slack_text);
            post_message_with_blocks(&client, &token, to, fallback, &blocks, thread_ts.as_deref())
                .await?;
        } else {
            for chunk in chunks {
                post_message(&client, &token, to, chunk, thread_ts.as_deref()).await?;
            }
        }

        #[cfg(feature = "metrics")]
        moltis_metrics::counter!(
            moltis_metrics::channels::MESSAGES_SENT_TOTAL,
            moltis_metrics::labels::CHANNEL => "slack"
        )
        .increment(1);

        Ok(())
    }

    async fn send_text_reporting_ids(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<Vec<String>> {
        let (client, token) = self.get_session(account_id)?;
        let thread_ts = self.get_reply_thread_ts(account_id, reply_to);
        let slack_text = markdown_to_slack(text);
        let chunks = chunk_message(&slack_text, SLACK_MAX_MESSAGE_LEN);
        let rendered_blocks = (self.get_rich_blocks(account_id) && chunks.len() == 1)
            .then(|| crate::blocks::markdown_to_blocks(text))
            .flatten();

        let mut ids = Vec::new();
        if let Some(blocks) = rendered_blocks {
            let fallback = chunks.first().copied().unwrap_or(&slack_text);
            let ts = post_message_with_blocks(
                &client,
                &token,
                to,
                fallback,
                &blocks,
                thread_ts.as_deref(),
            )
            .await?;
            ids.push(ts.to_string());
        } else {
            for chunk in chunks {
                let ts = post_message(&client, &token, to, chunk, thread_ts.as_deref()).await?;
                ids.push(ts.to_string());
            }
        }

        #[cfg(feature = "metrics")]
        moltis_metrics::counter!(
            moltis_metrics::channels::MESSAGES_SENT_TOTAL,
            moltis_metrics::labels::CHANNEL => "slack"
        )
        .increment(1);

        Ok(ids)
    }

    async fn send_text_with_suffix(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.send_text_with_suffix_reporting_ids(account_id, to, text, suffix_html, reply_to)
            .await?;
        Ok(())
    }

    async fn send_text_with_suffix_reporting_ids(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _suffix_html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<Vec<String>> {
        self.send_text_reporting_ids(account_id, to, text, reply_to)
            .await
    }

    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.send_html_reporting_ids(account_id, to, html, reply_to)
            .await?;
        Ok(())
    }

    async fn send_html_reporting_ids(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<Vec<String>> {
        self.send_text_reporting_ids(account_id, to, html, reply_to)
            .await
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.send_media_reporting_ids(account_id, to, payload, reply_to)
            .await?;
        Ok(())
    }

    async fn send_media_reporting_ids(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<Vec<String>> {
        let media_url = payload.media.as_ref().map(|m| m.url.as_str());

        match media_url {
            Some(url) if url.starts_with("data:") => {
                let (data, mime) = decode_data_url(url)?;
                let filename = payload
                    .media
                    .as_ref()
                    .and_then(|m| m.filename.clone())
                    .unwrap_or_else(|| {
                        let ext = extension_for_mime(&mime);
                        format!("file.{ext}")
                    });
                let caption = if payload.text.is_empty() {
                    None
                } else {
                    Some(payload.text.as_str())
                };

                let (client, token) = self.get_session(account_id)?;
                let thread_ts = self.get_reply_thread_ts(account_id, reply_to);

                upload_file_reporting_ids(
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
                self.send_text_reporting_ids(account_id, to, &text, reply_to)
                    .await
            },
            None => {
                // No media — send text only.
                let text = if payload.text.is_empty() {
                    "(media attachment)".to_string()
                } else {
                    payload.text.clone()
                };
                self.send_text_reporting_ids(account_id, to, &text, reply_to)
                    .await
            },
        }
    }

    async fn send_typing(&self, _account_id: &str, _to: &str) -> ChannelResult<()> {
        // Slack bots have no typing indicator. `assistant.threads.setStatus`
        // can show a live status, but only for apps configured as Slack AI/
        // Assistant apps and only inside assistant threads; that needs an exact
        // thread identity per turn, so it is intentionally not wired here.
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
        let thread_ts = self.get_reply_thread_ts(account_id, reply_to);
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

#[async_trait]
impl ChannelStreamOutbound for SlackOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        stream: StreamReceiver,
    ) -> ChannelResult<()> {
        self.send_stream_reporting_ids(account_id, to, reply_to, stream)
            .await?;
        Ok(())
    }

    async fn send_stream_reporting_ids(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<Vec<String>> {
        let stream_mode = self.get_stream_mode(account_id);
        let thread_ts = self.get_reply_thread_ts(account_id, reply_to);

        match stream_mode {
            StreamMode::Native => match thread_ts.as_deref() {
                Some(thread_ts) => {
                    self.send_stream_native(account_id, to, thread_ts, &mut stream)
                        .await
                },
                None => {
                    self.send_stream_edit_in_place(account_id, to, None, &mut stream)
                        .await
                },
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
                        Some(StreamEvent::Delta(chunk) | StreamEvent::ProgressDelta(chunk)) => {
                            accumulated.push_str(&chunk)
                        },
                        Some(StreamEvent::TaskUpdate(_)) => {},
                        Some(StreamEvent::Error(e)) => {
                            accumulated.push_str(&format!("\n\n:warning: {e}"));
                            break;
                        },
                        Some(StreamEvent::Done) | None => break,
                    }
                }
                let mut ids = Vec::new();
                if !accumulated.is_empty() {
                    let (client, token) = self.get_session(account_id)?;
                    let final_text = markdown_to_slack(&accumulated);
                    for chunk in chunk_message(&final_text, SLACK_MAX_MESSAGE_LEN) {
                        match post_message(&client, &token, to, chunk, thread_ts.as_deref()).await {
                            Ok(ts) => ids.push(ts.to_string()),
                            Err(e) => {
                                warn!(account_id, to, "failed to send stream message: {e}");
                            },
                        }
                    }
                }
                Ok(ids)
            },
        }
    }

    async fn is_stream_enabled(&self, account_id: &str) -> bool {
        // Streaming sends incremental plain text, which cannot carry Block Kit.
        // When rich rendering is requested it wins: the reply is delivered once,
        // complete, through `send_text` so it actually renders as blocks.
        self.get_stream_mode(account_id) != StreamMode::Off && !self.get_rich_blocks(account_id)
    }

    async fn receives_task_updates(&self, account_id: &str) -> bool {
        self.get_stream_mode(account_id) == StreamMode::Native && !self.get_rich_blocks(account_id)
    }

    async fn claims_stream_delivery(&self, account_id: &str, reply_to: Option<&str>) -> bool {
        self.get_stream_mode(account_id) == StreamMode::Native
            && !self.get_rich_blocks(account_id)
            && self.get_reply_thread_ts(account_id, reply_to).is_some()
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
                    message_id: timestamp.clone(),
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

    fn outbound_with_thread_replies(thread_replies: bool) -> SlackOutbound {
        outbound_with_config(thread_replies, StreamMode::EditInPlace, false)
    }

    fn outbound_with_config(
        thread_replies: bool,
        stream_mode: StreamMode,
        rich_blocks: bool,
    ) -> SlackOutbound {
        let accounts =
            std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));
        let config = crate::config::SlackAccountConfig {
            thread_replies,
            stream_mode,
            rich_blocks,
            ..Default::default()
        };
        accounts
            .write()
            .unwrap()
            .insert("acct".to_string(), crate::state::AccountState {
                account_id: "acct".to_string(),
                config,
                message_log: None,
                event_sink: None,
                cancel: tokio_util::sync::CancellationToken::new(),
                bot_user_id: Some("UBOT".to_string()),
                stream_recipients: Default::default(),
                otp: std::sync::Mutex::new(moltis_channels::otp::OtpState::new(300)),
                dedup: std::sync::Mutex::new(crate::state::EventDedup::default()),
            });
        SlackOutbound { accounts }
    }

    #[test]
    fn configured_thread_replies_use_exact_inbound_root() {
        let outbound = outbound_with_thread_replies(true);
        let ts = outbound.get_reply_thread_ts("acct", Some("1234567.890"));
        assert_eq!(ts, Some("1234567.890".to_string()));
    }

    #[test]
    fn disabled_thread_replies_send_normal_replies_to_channel() {
        let outbound = outbound_with_thread_replies(false);
        assert!(
            outbound
                .get_reply_thread_ts("acct", Some("1234567.890"))
                .is_none()
        );
    }

    #[tokio::test]
    async fn task_updates_require_native_streaming_without_rich_blocks() {
        let native = outbound_with_config(true, StreamMode::Native, false);
        assert!(native.receives_task_updates("acct").await);
        assert!(native.claims_stream_delivery("acct", Some("1.0")).await);
        assert!(!native.claims_stream_delivery("acct", None).await);

        let edit = outbound_with_config(true, StreamMode::EditInPlace, false);
        assert!(!edit.receives_task_updates("acct").await);
        assert!(!edit.claims_stream_delivery("acct", Some("1.0")).await);

        let rich = outbound_with_config(true, StreamMode::Native, true);
        assert!(!rich.receives_task_updates("acct").await);
        assert!(!rich.claims_stream_delivery("acct", Some("1.0")).await);

        let top_level = outbound_with_config(false, StreamMode::Native, false);
        assert!(!top_level.claims_stream_delivery("acct", Some("1.0")).await);
    }

    #[test]
    fn response_url_accepts_only_slack_https_hook_hosts() {
        assert!(validate_response_url("https://hooks.slack.com/commands/T1/123/secret").is_ok());
        assert!(
            validate_response_url("https://hooks.slack-gov.com/commands/T1/123/secret").is_ok()
        );

        for url in [
            "http://hooks.slack.com/commands/T1/123/secret",
            "https://user@hooks.slack.com/commands/T1/123/secret",
            "https://hooks.slack.com:443/commands/T1/123/secret",
            "https://hooks.slack.com.evil.example/commands/T1/123/secret",
            "https://slack.com/commands/T1/123/secret",
            "https://127.0.0.1/commands/T1/123/secret",
        ] {
            assert!(validate_response_url(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn response_url_client_is_reused() {
        let first = response_url_http_client().unwrap();
        let second = response_url_http_client().unwrap();
        assert!(std::ptr::eq(first, second));
    }

    #[tokio::test]
    async fn response_url_client_does_not_follow_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let _ = socket.read(&mut request).await.unwrap();
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{address}/redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            tokio::time::timeout(Duration::from_millis(200), listener.accept())
                .await
                .is_ok()
        });

        let response = response_url_http_client()
            .unwrap()
            .post(format!("http://{address}/initial"))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_redirection());
        assert!(!server.await.unwrap(), "redirect target was requested");
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

    #[test]
    fn trace_link_delivery_modes_override_reporting_contracts() {
        let implementation = include_str!("outbound.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        for mode in ["text", "media", "text_with_suffix", "html", "stream"] {
            let method = format!("async fn send_{mode}_reporting_ids(");
            assert!(
                implementation.contains(&method),
                "Slack must override {method} so reactions retain trace links"
            );
        }
    }
}
