use std::time::Duration;

use {
    async_trait::async_trait,
    moltis_channels::{Error as ChannelError, Result as ChannelResult},
    tracing::warn,
};

use crate::{client::slack_api_method_url, state::StreamRecipient};

/// Slack's documented limit for `markdown_text` on native stream methods.
const MAX_MARKDOWN_CHARS: usize = 12_000;

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

fn native_stream_body(stream: &NativeStream, markdown_text: Option<&str>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "channel": stream.channel,
        "ts": stream.ts,
    });
    if let Some(text) = markdown_text {
        body["markdown_text"] = serde_json::json!(text);
    }
    body
}

fn start_native_stream_body(
    channel: &str,
    thread_ts: &str,
    markdown_text: &str,
    recipient: Option<&StreamRecipient>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "channel": channel,
        "thread_ts": thread_ts,
        "markdown_text": markdown_text,
    });
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
        markdown_text: &str,
        recipient: Option<&StreamRecipient>,
    ) -> ChannelResult<NativeStream>;

    async fn append(&self, stream: &NativeStream, markdown_text: &str) -> ChannelResult<()>;

    async fn stop(&self, stream: &NativeStream, markdown_text: Option<&str>) -> ChannelResult<()>;
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
        markdown_text: &str,
        recipient: Option<&StreamRecipient>,
    ) -> ChannelResult<NativeStream> {
        let body = start_native_stream_body(channel, thread_ts, markdown_text, recipient);
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

    async fn append(&self, stream: &NativeStream, markdown_text: &str) -> ChannelResult<()> {
        let response: serde_json::Value = self
            .http
            .post(slack_api_method_url(
                &self.api_base_url,
                "chat.appendStream",
            )?)
            .bearer_auth(&self.bot_token)
            .json(&native_stream_body(stream, Some(markdown_text)))
            .send()
            .await
            .map_err(|error| ChannelError::external("chat.appendStream", error))?
            .json()
            .await
            .map_err(|error| ChannelError::external("chat.appendStream parse", error))?;
        check_ok(&response, "chat.appendStream")
    }

    async fn stop(&self, stream: &NativeStream, markdown_text: Option<&str>) -> ChannelResult<()> {
        let response: serde_json::Value = self
            .http
            .post(slack_api_method_url(&self.api_base_url, "chat.stopStream")?)
            .bearer_auth(&self.bot_token)
            .json(&native_stream_body(stream, markdown_text))
            .send()
            .await
            .map_err(|error| ChannelError::external("chat.stopStream", error))?
            .json()
            .await
            .map_err(|error| ChannelError::external("chat.stopStream parse", error))?;
        check_ok(&response, "chat.stopStream")
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
) -> ChannelResult<()> {
    send_native_stream_with_api(api, channel, thread_ts, recipient, throttle, stream).await
}

async fn send_native_stream_with_api<A: NativeStreamApi>(
    api: &A,
    channel: &str,
    thread_ts: &str,
    recipient: Option<&StreamRecipient>,
    throttle: Duration,
    stream: &mut moltis_channels::plugin::StreamReceiver,
) -> ChannelResult<()> {
    use moltis_channels::plugin::StreamEvent;

    let mut pending = String::new();
    let mut native_stream = None;
    let mut last_append = tokio::time::Instant::now();

    loop {
        match stream.recv().await {
            Some(StreamEvent::Delta(chunk) | StreamEvent::ProgressDelta(chunk)) => {
                pending.push_str(&chunk);
                if native_stream.is_none() {
                    let Some(initial) = take_markdown_chunk(&mut pending) else {
                        continue;
                    };
                    native_stream = Some(api.start(channel, thread_ts, &initial, recipient).await?);
                    last_append = tokio::time::Instant::now();
                }
                if last_append.elapsed() >= throttle {
                    flush_appends(api, native_stream.as_ref(), &mut pending).await?;
                    last_append = tokio::time::Instant::now();
                }
            },
            Some(StreamEvent::Done) | None => break,
            Some(StreamEvent::Error(error)) => {
                pending.push_str(&format!("\n\n:warning: {error}"));
                break;
            },
        }
    }

    if native_stream.is_none() {
        let Some(initial) = take_markdown_chunk(&mut pending) else {
            return Ok(());
        };
        native_stream = Some(api.start(channel, thread_ts, &initial, recipient).await?);
    }
    let native_stream = native_stream
        .as_ref()
        .ok_or_else(|| ChannelError::unavailable("native Slack stream was not started"))?;

    while exceeds_markdown_limit(&pending) {
        let chunk = take_markdown_chunk(&mut pending).unwrap_or_default();
        append_or_stop(api, native_stream, &chunk).await?;
    }

    let final_text = (!pending.is_empty()).then_some(pending.as_str());
    if let Err(error) = api.stop(native_stream, final_text).await {
        warn!(channel, "chat.stopStream failed: {error}");
        if let Err(fallback_error) = api.stop(native_stream, None).await {
            warn!(channel, "fallback chat.stopStream failed: {fallback_error}");
        }
        return Err(error);
    }
    Ok(())
}

async fn flush_appends<A: NativeStreamApi>(
    api: &A,
    stream: Option<&NativeStream>,
    pending: &mut String,
) -> ChannelResult<()> {
    let stream =
        stream.ok_or_else(|| ChannelError::unavailable("native Slack stream was not started"))?;
    while let Some(chunk) = take_markdown_chunk(pending) {
        append_or_stop(api, stream, &chunk).await?;
    }
    Ok(())
}

async fn append_or_stop<A: NativeStreamApi>(
    api: &A,
    stream: &NativeStream,
    markdown_text: &str,
) -> ChannelResult<()> {
    if let Err(error) = api.append(stream, markdown_text).await {
        if let Err(stop_error) = api.stop(stream, None).await {
            warn!("fallback chat.stopStream failed after append error: {stop_error}");
        }
        return Err(error);
    }
    Ok(())
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
        Start(String),
        Append(String),
        Stop(Option<String>),
    }

    #[derive(Default)]
    struct FakeApi {
        calls: Mutex<Vec<Call>>,
        fail_append: bool,
        stop_failures: Mutex<usize>,
    }

    #[async_trait]
    impl NativeStreamApi for FakeApi {
        async fn start(
            &self,
            _channel: &str,
            _thread_ts: &str,
            markdown_text: &str,
            _recipient: Option<&StreamRecipient>,
        ) -> ChannelResult<NativeStream> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Start(markdown_text.to_string()));
            Ok(NativeStream {
                channel: "C1".into(),
                ts: "1.0".into(),
            })
        }

        async fn append(&self, _stream: &NativeStream, markdown_text: &str) -> ChannelResult<()> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Append(markdown_text.to_string()));
            if self.fail_append {
                Err(ChannelError::unavailable("append failed"))
            } else {
                Ok(())
            }
        }

        async fn stop(
            &self,
            _stream: &NativeStream,
            markdown_text: Option<&str>,
        ) -> ChannelResult<()> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Stop(markdown_text.map(String::from)));
            let mut failures = self.stop_failures.lock().unwrap();
            if *failures == 0 {
                return Ok(());
            }
            *failures -= 1;
            Err(ChannelError::unavailable("stop failed"))
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
    async fn append_failure_returns_error_and_stops_stream() {
        let api = FakeApi {
            fail_append: true,
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

        assert!(result.is_err());
        assert_eq!(*api.calls.lock().unwrap(), vec![
            Call::Start("first".into()),
            Call::Append("second".into()),
            Call::Stop(None),
        ]);
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

        send_native_stream_with_api(&api, "C1", "1.0", None, Duration::ZERO, &mut receiver)
            .await
            .unwrap();

        let calls = api.calls.lock().unwrap();
        assert_eq!(calls.first(), Some(&Call::Start(markdown.into())));
        let delivered = calls
            .iter()
            .filter_map(|call| match call {
                Call::Start(text) | Call::Append(text) => Some(text.as_str()),
                Call::Stop(Some(text)) => Some(text.as_str()),
                Call::Stop(None) => None,
            })
            .collect::<String>();
        assert_eq!(delivered, format!("{markdown}{long}"));
        assert!(calls.iter().all(|call| {
            match call {
                Call::Start(text) | Call::Append(text) => {
                    text.chars().count() <= MAX_MARKDOWN_CHARS
                },
                Call::Stop(text) => text
                    .as_deref()
                    .is_none_or(|text| text.chars().count() <= MAX_MARKDOWN_CHARS),
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
            Call::Start("start".into()),
            Call::Append("é".repeat(MAX_MARKDOWN_CHARS)),
            Call::Stop(Some("é".into())),
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
            Call::Start("start".into()),
            Call::Stop(Some("tail".into())),
            Call::Stop(None),
        ]);
    }

    #[test]
    fn official_payloads_use_channel_ts_and_markdown_text() {
        let recipient = StreamRecipient {
            user_id: "U123".into(),
            team_id: "T123".into(),
        };
        assert_eq!(
            start_native_stream_body("C123", "1.0", "**hello**", Some(&recipient)),
            serde_json::json!({
                "channel": "C123",
                "thread_ts": "1.0",
                "markdown_text": "**hello**",
                "recipient_user_id": "U123",
                "recipient_team_id": "T123",
            })
        );
        let stream = NativeStream {
            channel: "C123".into(),
            ts: "2.0".into(),
        };
        assert_eq!(
            native_stream_body(&stream, Some("tail")),
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
    }
}
