use std::{collections::BTreeMap, time::Duration};

use {
    async_trait::async_trait,
    moltis_channels::{
        Error as ChannelError, Result as ChannelResult,
        plugin::{ChannelTaskStatus, ChannelTaskUpdate},
    },
    tracing::warn,
};

use crate::{client::slack_api_method_url, state::StreamRecipient};

/// Slack's documented limit for `markdown_text` on native stream methods.
const MAX_MARKDOWN_CHARS: usize = 12_000;
const MAX_TASK_CHARS: usize = 256;
const PLAN_TITLE: &str = "Working on your request";
const STREAM_FAILURE_NOTICE: &str =
    ":warning: Slack streaming failed before the response completed.";

#[derive(Debug, serde::Deserialize)]
struct NativeStreamResponse {
    ok: bool,
    channel: Option<String>,
    ts: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeStream {
    channel: String,
    ts: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiscardOutcome {
    Deleted,
    Retained,
    Unrecovered,
}

#[derive(Debug)]
enum AppendOutcome {
    Appended,
    NeedsRecovery(StreamRecovery),
}

#[derive(Debug)]
struct StreamRecovery {
    error: ChannelError,
    cleanup: Option<NativeStreamContent>,
    stopped: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum NativeStreamChunk {
    PlanUpdate {
        title: String,
    },
    TaskUpdate {
        id: String,
        title: String,
        status: NativeTaskStatus,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum NativeTaskStatus {
    InProgress,
    Complete,
    Error,
}

impl From<ChannelTaskStatus> for NativeTaskStatus {
    fn from(status: ChannelTaskStatus) -> Self {
        match status {
            ChannelTaskStatus::InProgress => Self::InProgress,
            ChannelTaskStatus::Complete => Self::Complete,
            ChannelTaskStatus::Error => Self::Error,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct NativeStreamContent {
    markdown_text: Option<String>,
    chunks: Vec<NativeStreamChunk>,
}

impl NativeStreamContent {
    fn is_empty(&self) -> bool {
        self.markdown_text.as_deref().is_none_or(str::is_empty) && self.chunks.is_empty()
    }
}

fn add_content(body: &mut serde_json::Value, content: &NativeStreamContent) {
    if let Some(text) = content.markdown_text.as_deref()
        && !text.is_empty()
    {
        body["markdown_text"] = serde_json::json!(text);
    }
    if !content.chunks.is_empty() {
        body["chunks"] = serde_json::json!(content.chunks);
    }
}

fn native_stream_body(
    stream: &NativeStream,
    content: Option<&NativeStreamContent>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "channel": stream.channel,
        "ts": stream.ts,
    });
    if let Some(content) = content {
        add_content(&mut body, content);
    }
    body
}

fn update_message_body(stream: &NativeStream, markdown_text: &str) -> serde_json::Value {
    serde_json::json!({
        "channel": stream.channel,
        "ts": stream.ts,
        "markdown_text": markdown_text,
    })
}

fn start_native_stream_body(
    channel: &str,
    thread_ts: &str,
    content: &NativeStreamContent,
    recipient: Option<&StreamRecipient>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "channel": channel,
        "thread_ts": thread_ts,
        "task_display_mode": "plan",
    });
    add_content(&mut body, content);
    if !channel.starts_with('D')
        && let Some(recipient) = recipient
    {
        body["recipient_user_id"] = serde_json::json!(recipient.user_id);
        body["recipient_team_id"] = serde_json::json!(recipient.team_id);
    }
    body
}

#[async_trait]
trait NativeStreamApi: Send + Sync {
    async fn start(
        &self,
        channel: &str,
        thread_ts: &str,
        content: &NativeStreamContent,
        recipient: Option<&StreamRecipient>,
    ) -> ChannelResult<NativeStream>;

    async fn append(
        &self,
        stream: &NativeStream,
        content: &NativeStreamContent,
    ) -> ChannelResult<()>;

    async fn stop(
        &self,
        stream: &NativeStream,
        content: Option<&NativeStreamContent>,
    ) -> ChannelResult<()>;

    async fn delete(&self, stream: &NativeStream) -> ChannelResult<()>;

    async fn update(&self, stream: &NativeStream, text: &str) -> ChannelResult<()>;
}

pub(crate) struct HttpNativeStreamApi {
    http: reqwest::Client,
    api_base_url: String,
    bot_token: String,
}

impl HttpNativeStreamApi {
    pub(crate) fn new(http: reqwest::Client, api_base_url: String, bot_token: String) -> Self {
        Self {
            http,
            api_base_url,
            bot_token,
        }
    }
}

#[async_trait]
impl NativeStreamApi for HttpNativeStreamApi {
    async fn start(
        &self,
        channel: &str,
        thread_ts: &str,
        content: &NativeStreamContent,
        recipient: Option<&StreamRecipient>,
    ) -> ChannelResult<NativeStream> {
        let body = start_native_stream_body(channel, thread_ts, content, recipient);
        let response: NativeStreamResponse = self
            .http
            .post(slack_api_method_url(
                &self.api_base_url,
                "chat.startStream",
            )?)
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await
            .map_err(|error| ChannelError::external("chat.startStream", error))?
            .json()
            .await
            .map_err(|error| ChannelError::external("chat.startStream parse", error))?;

        if !response.ok {
            let error = response.error.as_deref().unwrap_or("unknown");
            return Err(ChannelError::unavailable(format!(
                "chat.startStream failed: {error}"
            )));
        }

        Ok(NativeStream {
            channel: response
                .channel
                .ok_or_else(|| ChannelError::unavailable("chat.startStream: missing channel"))?,
            ts: response
                .ts
                .ok_or_else(|| ChannelError::unavailable("chat.startStream: missing ts"))?,
        })
    }

    async fn append(
        &self,
        stream: &NativeStream,
        content: &NativeStreamContent,
    ) -> ChannelResult<()> {
        let response: serde_json::Value = self
            .http
            .post(slack_api_method_url(
                &self.api_base_url,
                "chat.appendStream",
            )?)
            .bearer_auth(&self.bot_token)
            .json(&native_stream_body(stream, Some(content)))
            .send()
            .await
            .map_err(|error| ChannelError::external("chat.appendStream", error))?
            .json()
            .await
            .map_err(|error| ChannelError::external("chat.appendStream parse", error))?;
        check_ok(&response, "chat.appendStream")
    }

    async fn stop(
        &self,
        stream: &NativeStream,
        content: Option<&NativeStreamContent>,
    ) -> ChannelResult<()> {
        let response: serde_json::Value = self
            .http
            .post(slack_api_method_url(&self.api_base_url, "chat.stopStream")?)
            .bearer_auth(&self.bot_token)
            .json(&native_stream_body(stream, content))
            .send()
            .await
            .map_err(|error| ChannelError::external("chat.stopStream", error))?
            .json()
            .await
            .map_err(|error| ChannelError::external("chat.stopStream parse", error))?;
        check_ok(&response, "chat.stopStream")
    }

    async fn delete(&self, stream: &NativeStream) -> ChannelResult<()> {
        let response: serde_json::Value = self
            .http
            .post(slack_api_method_url(&self.api_base_url, "chat.delete")?)
            .bearer_auth(&self.bot_token)
            .json(&native_stream_body(stream, None))
            .send()
            .await
            .map_err(|error| ChannelError::external("chat.delete", error))?
            .json()
            .await
            .map_err(|error| ChannelError::external("chat.delete parse", error))?;
        check_ok(&response, "chat.delete")
    }

    async fn update(&self, stream: &NativeStream, markdown_text: &str) -> ChannelResult<()> {
        let response: serde_json::Value = self
            .http
            .post(slack_api_method_url(&self.api_base_url, "chat.update")?)
            .bearer_auth(&self.bot_token)
            .json(&update_message_body(stream, markdown_text))
            .send()
            .await
            .map_err(|error| ChannelError::external("chat.update", error))?
            .json()
            .await
            .map_err(|error| ChannelError::external("chat.update parse", error))?;
        check_ok(&response, "chat.update")
    }
}

fn check_ok(response: &serde_json::Value, method: &str) -> ChannelResult<()> {
    if response.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(());
    }
    let error = response
        .get("error")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    Err(ChannelError::unavailable(format!(
        "{method} failed: {error}"
    )))
}

pub(crate) async fn send_native_stream(
    api: &HttpNativeStreamApi,
    channel: &str,
    thread_ts: &str,
    recipient: Option<&StreamRecipient>,
    throttle: Duration,
    stream: &mut moltis_channels::plugin::StreamReceiver,
) -> ChannelResult<Vec<String>> {
    send_native_stream_with_api(api, channel, thread_ts, recipient, throttle, stream).await
}

async fn send_native_stream_with_api<A: NativeStreamApi>(
    api: &A,
    channel: &str,
    thread_ts: &str,
    recipient: Option<&StreamRecipient>,
    throttle: Duration,
    stream: &mut moltis_channels::plugin::StreamReceiver,
) -> ChannelResult<Vec<String>> {
    use moltis_channels::plugin::StreamEvent;

    let mut pending_text = String::new();
    let mut full_text = String::new();
    let mut recovery_overflowed = false;
    let mut pending_chunks = Vec::new();
    let mut native_stream = None;
    let mut plan_started = false;
    let mut active_tasks = BTreeMap::new();
    let mut last_append = tokio::time::Instant::now();
    let interval_duration = throttle.max(Duration::from_millis(1));
    let mut flush_interval = tokio::time::interval(interval_duration);
    flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    flush_interval.tick().await;

    loop {
        tokio::select! {
            event = stream.recv() => {
                match event {
                    Some(StreamEvent::Delta(chunk) | StreamEvent::ProgressDelta(chunk)) => {
                        recovery_overflowed |= append_recovery_text(&mut full_text, &chunk);
                        pending_text.push_str(&chunk);
                    },
                    Some(StreamEvent::TaskUpdate(update)) => {
                        if !plan_started {
                            pending_chunks.push(NativeStreamChunk::PlanUpdate {
                                title: PLAN_TITLE.to_string(),
                            });
                            plan_started = true;
                        }
                        pending_chunks.push(task_update_chunk(update));
                    },
                    Some(StreamEvent::Done) | None => break,
                    Some(StreamEvent::Error(error)) => {
                        let notice = format!("\n\n:warning: {error}");
                        recovery_overflowed |= append_recovery_text(&mut full_text, &notice);
                        pending_text.push_str(&notice);
                        break;
                    },
                }

                if native_stream.is_none() {
                    let Some(initial) = take_native_content(&mut pending_text, &mut pending_chunks) else {
                        continue;
                    };
                    native_stream = Some(api.start(channel, thread_ts, &initial, recipient).await?);
                    apply_task_updates(&mut active_tasks, &initial);
                    last_append = tokio::time::Instant::now();
                } else if last_append.elapsed() >= throttle {
                    if let Some(recovery) = flush_appends(
                        api,
                        native_stream.as_ref(),
                        &mut pending_text,
                        &mut pending_chunks,
                        &mut active_tasks,
                    )
                    .await?
                    {
                        let native_stream = native_stream.as_ref().ok_or_else(|| {
                            ChannelError::unavailable("native Slack stream was not started")
                        })?;
                        let updated = recover_stream_message(
                            api,
                            native_stream,
                            &mut full_text,
                            &mut recovery_overflowed,
                            &recovery,
                            stream,
                        )
                        .await;
                        if updated {
                            return Ok(vec![native_stream.ts.clone()]);
                        }
                        return Err(recovery.error);
                    }
                    last_append = tokio::time::Instant::now();
                }
            }
            _ = flush_interval.tick(), if native_stream.is_some() => {
                if let Some(recovery) = flush_appends(
                    api,
                    native_stream.as_ref(),
                    &mut pending_text,
                    &mut pending_chunks,
                    &mut active_tasks,
                )
                .await?
                {
                    let native_stream = native_stream.as_ref().ok_or_else(|| {
                        ChannelError::unavailable("native Slack stream was not started")
                    })?;
                    let updated = recover_stream_message(
                        api,
                        native_stream,
                        &mut full_text,
                        &mut recovery_overflowed,
                        &recovery,
                        stream,
                    )
                    .await;
                    if updated {
                        return Ok(vec![native_stream.ts.clone()]);
                    }
                    return Err(recovery.error);
                }
                last_append = tokio::time::Instant::now();
            }
        }
    }

    if native_stream.is_none() {
        let Some(initial) = take_native_content(&mut pending_text, &mut pending_chunks) else {
            return Ok(Vec::new());
        };
        native_stream = Some(api.start(channel, thread_ts, &initial, recipient).await?);
        apply_task_updates(&mut active_tasks, &initial);
    }
    let native_stream = native_stream
        .as_ref()
        .ok_or_else(|| ChannelError::unavailable("native Slack stream was not started"))?;

    while exceeds_markdown_limit(&pending_text) {
        let content =
            take_native_content(&mut pending_text, &mut pending_chunks).unwrap_or_default();
        match append_or_stop(api, native_stream, &content, &mut active_tasks).await? {
            AppendOutcome::Appended => {},
            AppendOutcome::NeedsRecovery(recovery) => {
                let updated = finish_recovery(
                    api,
                    native_stream,
                    &full_text,
                    recovery_overflowed,
                    &recovery,
                )
                .await;
                if updated {
                    return Ok(vec![native_stream.ts.clone()]);
                }
                return Err(recovery.error);
            },
        }
    }

    let final_content = take_native_content(&mut pending_text, &mut pending_chunks);
    if let Err(error) = api.stop(native_stream, final_content.as_ref()).await {
        warn!(channel, "chat.stopStream failed: {error}");
        let cleanup = error_cleanup_content(final_content.as_ref(), &active_tasks);
        match discard_stream(api, native_stream, cleanup.as_ref()).await {
            DiscardOutcome::Deleted => return Err(error),
            DiscardOutcome::Retained => {
                if update_recovered_message(api, native_stream, &full_text, recovery_overflowed)
                    .await
                {
                    return Ok(vec![native_stream.ts.clone()]);
                }
                return Err(error);
            },
            DiscardOutcome::Unrecovered => {
                let recovery = StreamRecovery {
                    error,
                    cleanup,
                    stopped: false,
                };
                if finish_recovery(
                    api,
                    native_stream,
                    &full_text,
                    recovery_overflowed,
                    &recovery,
                )
                .await
                {
                    return Ok(vec![native_stream.ts.clone()]);
                }
                return Err(recovery.error);
            },
        }
    }
    Ok(vec![native_stream.ts.clone()])
}

async fn flush_appends<A: NativeStreamApi>(
    api: &A,
    stream: Option<&NativeStream>,
    pending_text: &mut String,
    pending_chunks: &mut Vec<NativeStreamChunk>,
    active_tasks: &mut BTreeMap<String, String>,
) -> ChannelResult<Option<StreamRecovery>> {
    let stream =
        stream.ok_or_else(|| ChannelError::unavailable("native Slack stream was not started"))?;
    while let Some(content) = take_native_content(pending_text, pending_chunks) {
        match append_or_stop(api, stream, &content, active_tasks).await? {
            AppendOutcome::Appended => {},
            AppendOutcome::NeedsRecovery(recovery) => return Ok(Some(recovery)),
        }
    }
    Ok(None)
}

async fn append_or_stop<A: NativeStreamApi>(
    api: &A,
    stream: &NativeStream,
    content: &NativeStreamContent,
    active_tasks: &mut BTreeMap<String, String>,
) -> ChannelResult<AppendOutcome> {
    if let Err(error) = api.append(stream, content).await {
        let cleanup = error_cleanup_content(Some(content), active_tasks);
        match discard_stream(api, stream, cleanup.as_ref()).await {
            DiscardOutcome::Deleted => return Err(error),
            DiscardOutcome::Retained => {
                return Ok(AppendOutcome::NeedsRecovery(StreamRecovery {
                    error,
                    cleanup,
                    stopped: true,
                }));
            },
            DiscardOutcome::Unrecovered => {
                return Ok(AppendOutcome::NeedsRecovery(StreamRecovery {
                    error,
                    cleanup,
                    stopped: false,
                }));
            },
        }
    }
    apply_task_updates(active_tasks, content);
    Ok(AppendOutcome::Appended)
}

async fn discard_stream<A: NativeStreamApi>(
    api: &A,
    stream: &NativeStream,
    cleanup: Option<&NativeStreamContent>,
) -> DiscardOutcome {
    let stopped = match api.stop(stream, cleanup).await {
        Ok(()) => true,
        Err(error) => {
            warn!("fallback chat.stopStream failed while discarding stream: {error}");
            false
        },
    };
    match api.delete(stream).await {
        Ok(()) => DiscardOutcome::Deleted,
        Err(error) => {
            warn!("chat.delete failed while discarding native stream: {error}");
            if stopped {
                return DiscardOutcome::Retained;
            }
            match api.stop(stream, cleanup).await {
                Ok(()) => DiscardOutcome::Retained,
                Err(retry_error) => {
                    warn!("retry chat.stopStream failed while retaining stream: {retry_error}");
                    DiscardOutcome::Unrecovered
                },
            }
        },
    }
}

async fn recover_stream_message<A: NativeStreamApi>(
    api: &A,
    native_stream: &NativeStream,
    full_text: &mut String,
    recovery_overflowed: &mut bool,
    recovery: &StreamRecovery,
    stream: &mut moltis_channels::plugin::StreamReceiver,
) -> bool {
    use moltis_channels::plugin::StreamEvent;

    while let Some(event) = stream.recv().await {
        match event {
            StreamEvent::Delta(chunk) | StreamEvent::ProgressDelta(chunk) => {
                *recovery_overflowed |= append_recovery_text(full_text, &chunk);
            },
            StreamEvent::Error(error) => {
                *recovery_overflowed |=
                    append_recovery_text(full_text, &format!("\n\n:warning: {error}"));
                break;
            },
            StreamEvent::Done => break,
            StreamEvent::TaskUpdate(_) => {},
        }
    }
    finish_recovery(
        api,
        native_stream,
        full_text,
        *recovery_overflowed,
        recovery,
    )
    .await
}

async fn finish_recovery<A: NativeStreamApi>(
    api: &A,
    stream: &NativeStream,
    full_text: &str,
    recovery_overflowed: bool,
    recovery: &StreamRecovery,
) -> bool {
    if !recovery.stopped
        && let Err(error) = api.stop(stream, recovery.cleanup.as_ref()).await
    {
        warn!("final chat.stopStream failed while recovering native stream: {error}");
        if let Err(delete_error) = api.delete(stream).await {
            warn!("final chat.delete failed while recovering native stream: {delete_error}");
        }
        return false;
    }
    update_recovered_message(api, stream, full_text, recovery_overflowed).await
}

async fn update_recovered_message<A: NativeStreamApi>(
    api: &A,
    stream: &NativeStream,
    full_text: &str,
    recovery_overflowed: bool,
) -> bool {
    let text = if recovery_overflowed || full_text.is_empty() {
        STREAM_FAILURE_NOTICE
    } else {
        full_text
    };
    match api.update(stream, text).await {
        Ok(()) => true,
        Err(error) => {
            warn!("chat.update failed while recovering native stream: {error}");
            match api.delete(stream).await {
                Ok(()) => false,
                Err(delete_error) => {
                    warn!("chat.delete failed after recovery update failure: {delete_error}");
                    true
                },
            }
        },
    }
}

fn append_recovery_text(buffer: &mut String, text: &str) -> bool {
    let remaining = MAX_MARKDOWN_CHARS.saturating_sub(buffer.chars().count());
    let mut chars = text.chars();
    buffer.extend(chars.by_ref().take(remaining));
    chars.next().is_some()
}

fn apply_task_updates(active_tasks: &mut BTreeMap<String, String>, content: &NativeStreamContent) {
    for chunk in &content.chunks {
        let NativeStreamChunk::TaskUpdate { id, title, status } = chunk else {
            continue;
        };
        match status {
            NativeTaskStatus::InProgress => {
                active_tasks.insert(id.clone(), title.clone());
            },
            NativeTaskStatus::Complete | NativeTaskStatus::Error => {
                active_tasks.remove(id);
            },
        }
    }
}

fn error_cleanup_content(
    failed: Option<&NativeStreamContent>,
    active_tasks: &BTreeMap<String, String>,
) -> Option<NativeStreamContent> {
    let mut tasks = active_tasks.clone();
    let mut chunks = Vec::new();
    if let Some(content) = failed {
        for chunk in &content.chunks {
            match chunk {
                NativeStreamChunk::PlanUpdate { .. } => chunks.push(chunk.clone()),
                NativeStreamChunk::TaskUpdate { id, title, .. } => {
                    tasks.insert(id.clone(), title.clone());
                },
            }
        }
    }
    chunks.extend(
        tasks
            .into_iter()
            .map(|(id, title)| NativeStreamChunk::TaskUpdate {
                id,
                title,
                status: NativeTaskStatus::Error,
            }),
    );
    let content = NativeStreamContent {
        markdown_text: Some(STREAM_FAILURE_NOTICE.to_string()),
        chunks,
    };
    (!content.is_empty()).then_some(content)
}

fn task_update_chunk(update: ChannelTaskUpdate) -> NativeStreamChunk {
    NativeStreamChunk::TaskUpdate {
        id: truncate_chars(&update.id, MAX_TASK_CHARS),
        title: truncate_chars(&update.title, MAX_TASK_CHARS),
        status: update.status.into(),
    }
}

fn exceeds_markdown_limit(text: &str) -> bool {
    text.char_indices().nth(MAX_MARKDOWN_CHARS).is_some()
}

fn take_markdown_chunk(buffer: &mut String) -> Option<String> {
    if buffer.is_empty() {
        return None;
    }
    let split_at = buffer
        .char_indices()
        .nth(MAX_MARKDOWN_CHARS)
        .map_or(buffer.len(), |(index, _)| index);
    Some(buffer.drain(..split_at).collect())
}

fn take_native_content(
    pending_text: &mut String,
    pending_chunks: &mut Vec<NativeStreamChunk>,
) -> Option<NativeStreamContent> {
    let content = NativeStreamContent {
        markdown_text: take_markdown_chunk(pending_text),
        chunks: std::mem::take(pending_chunks),
    };
    (!content.is_empty()).then_some(content)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {
        moltis_channels::plugin::{StreamEvent, StreamReceiver},
        std::sync::Mutex,
    };

    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Call {
        Start(NativeStreamContent),
        Append(NativeStreamContent),
        Stop(Option<NativeStreamContent>),
        Delete,
        Update(String),
    }

    #[derive(Default)]
    struct FakeApi {
        calls: Mutex<Vec<Call>>,
        fail_append: bool,
        fail_update: bool,
        delete_failures: Mutex<usize>,
        stop_failures: Mutex<usize>,
    }

    #[async_trait]
    impl NativeStreamApi for FakeApi {
        async fn start(
            &self,
            _channel: &str,
            _thread_ts: &str,
            content: &NativeStreamContent,
            _recipient: Option<&StreamRecipient>,
        ) -> ChannelResult<NativeStream> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Start(content.clone()));
            Ok(NativeStream {
                channel: "C1".into(),
                ts: "1.0".into(),
            })
        }

        async fn append(
            &self,
            _stream: &NativeStream,
            content: &NativeStreamContent,
        ) -> ChannelResult<()> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Append(content.clone()));
            if self.fail_append {
                Err(ChannelError::unavailable("append failed"))
            } else {
                Ok(())
            }
        }

        async fn stop(
            &self,
            _stream: &NativeStream,
            content: Option<&NativeStreamContent>,
        ) -> ChannelResult<()> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Stop(content.cloned()));
            let mut failures = self.stop_failures.lock().unwrap();
            if *failures == 0 {
                return Ok(());
            }
            *failures -= 1;
            Err(ChannelError::unavailable("stop failed"))
        }

        async fn delete(&self, _stream: &NativeStream) -> ChannelResult<()> {
            self.calls.lock().unwrap().push(Call::Delete);
            let mut failures = self.delete_failures.lock().unwrap();
            if *failures == 0 {
                return Ok(());
            }
            *failures -= 1;
            Err(ChannelError::unavailable("delete failed"))
        }

        async fn update(&self, _stream: &NativeStream, text: &str) -> ChannelResult<()> {
            self.calls.lock().unwrap().push(Call::Update(text.into()));
            if self.fail_update {
                Err(ChannelError::unavailable("update failed"))
            } else {
                Ok(())
            }
        }
    }

    async fn stream(events: Vec<StreamEvent>) -> StreamReceiver {
        let (sender, receiver) = tokio::sync::mpsc::channel(events.len().max(1));
        for event in events {
            sender.send(event).await.unwrap();
        }
        drop(sender);
        receiver
    }

    #[tokio::test]
    async fn append_failure_with_failed_delete_suppresses_duplicate_fallback() {
        let api = FakeApi {
            fail_append: true,
            delete_failures: Mutex::new(1),
            stop_failures: Mutex::new(1),
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::Delta("first".into()),
            StreamEvent::Delta("second".into()),
            StreamEvent::Done,
        ])
        .await;

        let result =
            send_native_stream_with_api(&api, "C1", "1.0", None, Duration::ZERO, &mut receiver)
                .await;

        assert_eq!(result.unwrap(), vec!["1.0"]);
        assert_eq!(*api.calls.lock().unwrap(), vec![
            Call::Start(text_content("first")),
            Call::Append(text_content("second")),
            Call::Stop(Some(text_content(STREAM_FAILURE_NOTICE))),
            Call::Delete,
            Call::Stop(Some(text_content(STREAM_FAILURE_NOTICE))),
            Call::Update("firstsecond".into()),
        ]);
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn total_cleanup_failure_recovers_in_place() {
        let api = FakeApi {
            fail_append: true,
            delete_failures: Mutex::new(1),
            stop_failures: Mutex::new(2),
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::Delta("first".into()),
            StreamEvent::Delta("second".into()),
            StreamEvent::Done,
        ])
        .await;

        let result =
            send_native_stream_with_api(&api, "C1", "1.0", None, Duration::ZERO, &mut receiver)
                .await;

        assert_eq!(result.unwrap(), vec!["1.0"]);
        assert_eq!(*api.calls.lock().unwrap(), vec![
            Call::Start(text_content("first")),
            Call::Append(text_content("second")),
            Call::Stop(Some(text_content(STREAM_FAILURE_NOTICE))),
            Call::Delete,
            Call::Stop(Some(text_content(STREAM_FAILURE_NOTICE))),
            Call::Stop(Some(text_content(STREAM_FAILURE_NOTICE))),
            Call::Update("firstsecond".into()),
        ]);
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn total_cleanup_and_update_failure_preserves_fallback() {
        let api = FakeApi {
            fail_append: true,
            delete_failures: Mutex::new(1),
            fail_update: true,
            stop_failures: Mutex::new(2),
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::Delta("first".into()),
            StreamEvent::Delta("second".into()),
            StreamEvent::Done,
        ])
        .await;

        let result =
            send_native_stream_with_api(&api, "C1", "1.0", None, Duration::ZERO, &mut receiver)
                .await;

        assert!(result.unwrap_err().to_string().contains("append failed"));
        assert_eq!(api.calls.lock().unwrap().last(), Some(&Call::Delete));
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn failed_final_recovery_stop_does_not_update_active_stream() {
        let api = FakeApi {
            fail_append: true,
            delete_failures: Mutex::new(1),
            stop_failures: Mutex::new(3),
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::Delta("first".into()),
            StreamEvent::Delta("second".into()),
            StreamEvent::Done,
        ])
        .await;

        let result =
            send_native_stream_with_api(&api, "C1", "1.0", None, Duration::ZERO, &mut receiver)
                .await;

        assert!(result.unwrap_err().to_string().contains("append failed"));
        let calls = api.calls.lock().unwrap();
        assert_eq!(calls.last(), Some(&Call::Delete));
        assert!(!calls.iter().any(|call| matches!(call, Call::Update(_))));
    }

    #[tokio::test]
    async fn oversized_recovery_replaces_partial_answer_with_bounded_notice() {
        let api = FakeApi {
            fail_append: true,
            delete_failures: Mutex::new(1),
            stop_failures: Mutex::new(2),
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::Delta("first".into()),
            StreamEvent::Delta("second".into()),
            StreamEvent::Delta("é".repeat(MAX_MARKDOWN_CHARS)),
            StreamEvent::Done,
        ])
        .await;

        let result =
            send_native_stream_with_api(&api, "C1", "1.0", None, Duration::ZERO, &mut receiver)
                .await;

        assert_eq!(result.unwrap(), vec!["1.0"]);
        assert_eq!(
            api.calls.lock().unwrap().last(),
            Some(&Call::Update(STREAM_FAILURE_NOTICE.into()))
        );
        assert!(STREAM_FAILURE_NOTICE.chars().count() <= MAX_MARKDOWN_CHARS);
    }

    #[tokio::test]
    async fn failed_terminal_task_append_stops_with_an_error_card() {
        let api = FakeApi {
            fail_append: true,
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::TaskUpdate(ChannelTaskUpdate {
                id: "task-1".into(),
                title: "web_search".into(),
                status: ChannelTaskStatus::InProgress,
            }),
            StreamEvent::TaskUpdate(ChannelTaskUpdate {
                id: "task-1".into(),
                title: "web_search".into(),
                status: ChannelTaskStatus::Complete,
            }),
            StreamEvent::Done,
        ])
        .await;

        let result =
            send_native_stream_with_api(&api, "C1", "1.0", None, Duration::ZERO, &mut receiver)
                .await;

        assert!(result.is_err());
        let calls = api.calls.lock().unwrap();
        let Some(Call::Stop(Some(cleanup))) = calls.iter().rev().nth(1) else {
            panic!("expected cleanup stop");
        };
        assert_eq!(calls.last(), Some(&Call::Delete));
        assert_eq!(cleanup.chunks, vec![NativeStreamChunk::TaskUpdate {
            id: "task-1".into(),
            title: "web_search".into(),
            status: NativeTaskStatus::Error,
        }]);
    }

    #[tokio::test]
    async fn native_markdown_is_unchanged_and_every_request_is_unicode_bounded() {
        let api = FakeApi::default();
        let markdown = "**bold** [link](https://example.com)";
        let long = "🦀".repeat(MAX_MARKDOWN_CHARS * 2 + 1);
        let mut receiver = stream(vec![
            StreamEvent::Delta(markdown.into()),
            StreamEvent::Delta(long.clone()),
            StreamEvent::Done,
        ])
        .await;

        let ids =
            send_native_stream_with_api(&api, "C1", "1.0", None, Duration::ZERO, &mut receiver)
                .await
                .unwrap();

        assert_eq!(ids, ["1.0"]);
        let calls = api.calls.lock().unwrap();
        assert_eq!(calls.first(), Some(&Call::Start(text_content(markdown))));
        let delivered = calls
            .iter()
            .filter_map(|call| match call {
                Call::Start(content) | Call::Append(content) => content.markdown_text.as_deref(),
                Call::Stop(Some(content)) => content.markdown_text.as_deref(),
                Call::Stop(None) | Call::Delete => None,
                Call::Update(text) => Some(text.as_str()),
            })
            .collect::<String>();
        assert_eq!(delivered, format!("{markdown}{long}"));
        assert!(calls.iter().all(|call| {
            match call {
                Call::Start(content) | Call::Append(content) => content
                    .markdown_text
                    .as_deref()
                    .is_none_or(|text| text.chars().count() <= MAX_MARKDOWN_CHARS),
                Call::Stop(content) => content
                    .as_ref()
                    .and_then(|content| content.markdown_text.as_deref())
                    .is_none_or(|text| text.chars().count() <= MAX_MARKDOWN_CHARS),
                Call::Delete => true,
                Call::Update(text) => text.chars().count() <= MAX_MARKDOWN_CHARS,
            }
        }));
    }

    #[tokio::test]
    async fn final_stop_markdown_is_unicode_bounded() {
        let api = FakeApi::default();
        let tail = "é".repeat(MAX_MARKDOWN_CHARS + 1);
        let mut receiver = stream(vec![
            StreamEvent::Delta("start".into()),
            StreamEvent::Delta(tail.clone()),
            StreamEvent::Done,
        ])
        .await;

        send_native_stream_with_api(
            &api,
            "C1",
            "1.0",
            None,
            Duration::from_secs(60),
            &mut receiver,
        )
        .await
        .unwrap();

        assert_eq!(*api.calls.lock().unwrap(), vec![
            Call::Start(text_content("start")),
            Call::Append(text_content(&"é".repeat(MAX_MARKDOWN_CHARS))),
            Call::Stop(Some(text_content("é"))),
        ]);
    }

    #[tokio::test]
    async fn failed_final_stop_retries_without_markdown_and_returns_error() {
        let api = FakeApi {
            stop_failures: Mutex::new(1),
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::Delta("start".into()),
            StreamEvent::Delta("tail".into()),
            StreamEvent::Done,
        ])
        .await;

        let result = send_native_stream_with_api(
            &api,
            "C1",
            "1.0",
            None,
            Duration::from_secs(60),
            &mut receiver,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(*api.calls.lock().unwrap(), vec![
            Call::Start(text_content("start")),
            Call::Stop(Some(text_content("tail"))),
            Call::Stop(Some(text_content(STREAM_FAILURE_NOTICE))),
            Call::Delete,
        ]);
    }

    #[tokio::test]
    async fn failed_final_stop_with_retained_notice_suppresses_fallback() {
        let api = FakeApi {
            delete_failures: Mutex::new(1),
            stop_failures: Mutex::new(1),
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::Delta("start".into()),
            StreamEvent::Delta("tail".into()),
            StreamEvent::Done,
        ])
        .await;

        let result = send_native_stream_with_api(
            &api,
            "C1",
            "1.0",
            None,
            Duration::from_secs(60),
            &mut receiver,
        )
        .await;

        assert_eq!(result.unwrap(), vec!["1.0"]);
        assert_eq!(*api.calls.lock().unwrap(), vec![
            Call::Start(text_content("start")),
            Call::Stop(Some(text_content("tail"))),
            Call::Stop(Some(text_content(STREAM_FAILURE_NOTICE))),
            Call::Delete,
            Call::Update("starttail".into()),
        ]);
    }

    #[tokio::test]
    async fn retained_notice_update_failure_depends_on_final_delete() {
        for (delete_failures, expects_delivery) in [(1, false), (2, true)] {
            let api = FakeApi {
                delete_failures: Mutex::new(delete_failures),
                fail_update: true,
                stop_failures: Mutex::new(1),
                ..Default::default()
            };
            let mut receiver = stream(vec![
                StreamEvent::Delta("start".into()),
                StreamEvent::Delta("tail".into()),
                StreamEvent::Done,
            ])
            .await;

            let result = send_native_stream_with_api(
                &api,
                "C1",
                "1.0",
                None,
                Duration::from_secs(60),
                &mut receiver,
            )
            .await;

            assert_eq!(result.is_ok(), expects_delivery);
            assert_eq!(api.calls.lock().unwrap().last(), Some(&Call::Delete));
        }
    }

    #[tokio::test]
    async fn total_final_stop_cleanup_failure_recovers_in_place() {
        let api = FakeApi {
            delete_failures: Mutex::new(1),
            stop_failures: Mutex::new(3),
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::Delta("start".into()),
            StreamEvent::Delta("tail".into()),
            StreamEvent::Done,
        ])
        .await;

        let result = send_native_stream_with_api(
            &api,
            "C1",
            "1.0",
            None,
            Duration::from_secs(60),
            &mut receiver,
        )
        .await;

        assert_eq!(result.unwrap(), vec!["1.0"]);
        assert_eq!(*api.calls.lock().unwrap(), vec![
            Call::Start(text_content("start")),
            Call::Stop(Some(text_content("tail"))),
            Call::Stop(Some(text_content(STREAM_FAILURE_NOTICE))),
            Call::Delete,
            Call::Stop(Some(text_content(STREAM_FAILURE_NOTICE))),
            Call::Stop(Some(text_content(STREAM_FAILURE_NOTICE))),
            Call::Update("starttail".into()),
        ]);
    }

    #[tokio::test]
    async fn total_final_stop_cleanup_and_update_failure_preserves_fallback() {
        let api = FakeApi {
            delete_failures: Mutex::new(1),
            fail_update: true,
            stop_failures: Mutex::new(3),
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::Delta("start".into()),
            StreamEvent::Delta("tail".into()),
            StreamEvent::Done,
        ])
        .await;

        let result = send_native_stream_with_api(
            &api,
            "C1",
            "1.0",
            None,
            Duration::from_secs(60),
            &mut receiver,
        )
        .await;

        assert!(result.unwrap_err().to_string().contains("stop failed"));
        assert_eq!(api.calls.lock().unwrap().last(), Some(&Call::Delete));
    }

    #[tokio::test]
    async fn failed_final_stop_retries_with_active_tasks_marked_error() {
        let api = FakeApi {
            stop_failures: Mutex::new(1),
            ..Default::default()
        };
        let mut receiver = stream(vec![
            StreamEvent::TaskUpdate(ChannelTaskUpdate {
                id: "task-1".into(),
                title: "web_search".into(),
                status: ChannelTaskStatus::InProgress,
            }),
            StreamEvent::TaskUpdate(ChannelTaskUpdate {
                id: "task-1".into(),
                title: "web_search".into(),
                status: ChannelTaskStatus::Complete,
            }),
            StreamEvent::Done,
        ])
        .await;

        let result = send_native_stream_with_api(
            &api,
            "C1",
            "1.0",
            None,
            Duration::from_secs(60),
            &mut receiver,
        )
        .await;

        assert!(result.is_err());
        let calls = api.calls.lock().unwrap();
        let Some(Call::Stop(Some(cleanup))) = calls.iter().rev().nth(1) else {
            panic!("expected cleanup stop");
        };
        assert_eq!(calls.last(), Some(&Call::Delete));
        assert_eq!(cleanup.chunks, vec![NativeStreamChunk::TaskUpdate {
            id: "task-1".into(),
            title: "web_search".into(),
            status: NativeTaskStatus::Error,
        }]);
    }

    #[tokio::test]
    async fn task_updates_share_the_native_response_stream() {
        let api = FakeApi::default();
        let mut receiver = stream(vec![
            StreamEvent::TaskUpdate(ChannelTaskUpdate {
                id: "call-1".into(),
                title: "web_search".into(),
                status: ChannelTaskStatus::InProgress,
            }),
            StreamEvent::TaskUpdate(ChannelTaskUpdate {
                id: "call-1".into(),
                title: "web_search".into(),
                status: ChannelTaskStatus::Complete,
            }),
            StreamEvent::Delta("Here is the answer".into()),
            StreamEvent::Done,
        ])
        .await;

        send_native_stream_with_api(&api, "C1", "1.0", None, Duration::ZERO, &mut receiver)
            .await
            .unwrap();

        assert_eq!(*api.calls.lock().unwrap(), vec![
            Call::Start(NativeStreamContent {
                markdown_text: None,
                chunks: vec![
                    NativeStreamChunk::PlanUpdate {
                        title: PLAN_TITLE.into(),
                    },
                    NativeStreamChunk::TaskUpdate {
                        id: "call-1".into(),
                        title: "web_search".into(),
                        status: NativeTaskStatus::InProgress,
                    },
                ],
            }),
            Call::Append(NativeStreamContent {
                markdown_text: None,
                chunks: vec![NativeStreamChunk::TaskUpdate {
                    id: "call-1".into(),
                    title: "web_search".into(),
                    status: NativeTaskStatus::Complete,
                }],
            }),
            Call::Append(text_content("Here is the answer")),
            Call::Stop(None),
        ]);
    }

    #[test]
    fn task_update_fields_are_unicode_bounded() {
        let chunk = task_update_chunk(ChannelTaskUpdate {
            id: "🦀".repeat(MAX_TASK_CHARS + 1),
            title: "é".repeat(MAX_TASK_CHARS + 1),
            status: ChannelTaskStatus::Error,
        });
        let NativeStreamChunk::TaskUpdate { id, title, status } = chunk else {
            panic!("expected task update");
        };
        assert_eq!(id.chars().count(), MAX_TASK_CHARS);
        assert_eq!(title.chars().count(), MAX_TASK_CHARS);
        assert_eq!(status, NativeTaskStatus::Error);
    }

    #[test]
    fn official_payloads_use_channel_ts_and_markdown_text() {
        let recipient = StreamRecipient {
            user_id: "U123".into(),
            team_id: "T123".into(),
        };
        assert_eq!(
            start_native_stream_body("C123", "1.0", &text_content("**hello**"), Some(&recipient),),
            serde_json::json!({
                "channel": "C123",
                "thread_ts": "1.0",
                "markdown_text": "**hello**",
                "task_display_mode": "plan",
                "recipient_user_id": "U123",
                "recipient_team_id": "T123",
            })
        );
        let stream = NativeStream {
            channel: "C123".into(),
            ts: "2.0".into(),
        };
        assert_eq!(
            native_stream_body(&stream, Some(&text_content("tail"))),
            serde_json::json!({
                "channel": "C123",
                "ts": "2.0",
                "markdown_text": "tail",
            })
        );
        assert_eq!(
            native_stream_body(&stream, None),
            serde_json::json!({"channel": "C123", "ts": "2.0"})
        );
        assert_eq!(
            update_message_body(&stream, "complete"),
            serde_json::json!({
                "channel": "C123",
                "ts": "2.0",
                "markdown_text": "complete",
            })
        );
        assert_eq!(
            native_stream_body(
                &stream,
                Some(&NativeStreamContent {
                    markdown_text: None,
                    chunks: vec![NativeStreamChunk::TaskUpdate {
                        id: "call-1".into(),
                        title: "web_search".into(),
                        status: NativeTaskStatus::Complete,
                    }],
                }),
            ),
            serde_json::json!({
                "channel": "C123",
                "ts": "2.0",
                "chunks": [{
                    "type": "task_update",
                    "id": "call-1",
                    "title": "web_search",
                    "status": "complete",
                }],
            })
        );
    }

    fn text_content(text: &str) -> NativeStreamContent {
        NativeStreamContent {
            markdown_text: Some(text.to_string()),
            chunks: Vec::new(),
        }
    }
}
