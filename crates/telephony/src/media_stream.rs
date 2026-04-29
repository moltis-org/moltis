//! Twilio Media Streams WebSocket handler.
//!
//! Receives real-time audio from phone calls via WebSocket, transcribes speech,
//! dispatches to the agent loop, and streams TTS responses back to the caller.
//!
//! Protocol: <https://www.twilio.com/docs/voice/media-streams/websocket-messages>
//!
//! Audio format: mu-law 8kHz mono, 160 bytes per 20ms frame, base64-encoded in JSON.

use {
    base64::Engine,
    bytes::Bytes,
    futures::{SinkExt, StreamExt},
    serde::{Deserialize, Serialize},
    std::{
        collections::HashMap,
        sync::Arc,
        time::{Duration, Instant},
    },
    tokio::sync::{RwLock, mpsc},
    tokio_tungstenite::tungstenite::Message,
    tracing::{debug, info, warn},
};

use crate::audio;

// ── Twilio WebSocket message types ──────────────────────────────────

/// Inbound message from Twilio.
#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
enum TwilioInbound {
    Connected {},
    Start {
        #[serde(rename = "streamSid")]
        stream_sid: String,
        start: StartPayload,
    },
    Media {
        #[serde(rename = "streamSid")]
        stream_sid: String,
        media: MediaPayload,
    },
    Mark {
        #[serde(rename = "streamSid")]
        stream_sid: String,
        mark: MarkPayload,
    },
    Stop {
        #[serde(rename = "streamSid")]
        stream_sid: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartPayload {
    #[serde(rename = "callSid")]
    call_sid: Option<String>,
    #[serde(default)]
    custom_parameters: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct MediaPayload {
    payload: String,
    #[allow(dead_code)]
    timestamp: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MarkPayload {
    name: String,
}

/// Outbound message to Twilio.
#[derive(Debug, Serialize)]
#[serde(tag = "event")]
enum TwilioOutbound {
    #[serde(rename = "media")]
    Media {
        #[serde(rename = "streamSid")]
        stream_sid: String,
        media: OutboundMedia,
    },
    #[serde(rename = "mark")]
    Mark {
        #[serde(rename = "streamSid")]
        stream_sid: String,
        mark: OutboundMark,
    },
    #[serde(rename = "clear")]
    Clear {
        #[serde(rename = "streamSid")]
        stream_sid: String,
    },
}

#[derive(Debug, Serialize)]
struct OutboundMedia {
    payload: String,
}

#[derive(Debug, Serialize)]
struct OutboundMark {
    name: String,
}

// ── Stream session state ────────────────────────────────────────────

/// State for an active media stream session.
pub struct MediaStreamSession {
    pub stream_sid: String,
    pub call_sid: String,
    pub account_id: String,
    pub call_id: String,
    /// Accumulated audio buffer (mu-law bytes).
    audio_buffer: Vec<u8>,
    /// When the last audio frame was received.
    last_audio_at: Instant,
    /// Whether the bot is currently speaking (suppress barge-in echo).
    bot_speaking: bool,
    /// Counter for mark events.
    mark_counter: u64,
}

impl MediaStreamSession {
    fn new(stream_sid: String, call_sid: String, account_id: String, call_id: String) -> Self {
        Self {
            stream_sid,
            call_sid,
            account_id,
            call_id,
            audio_buffer: Vec::with_capacity(16_000), // ~2s at 8kHz
            last_audio_at: Instant::now(),
            bot_speaking: false,
            mark_counter: 0,
        }
    }

    fn next_mark_name(&mut self) -> String {
        self.mark_counter += 1;
        format!("tts-{}", self.mark_counter)
    }
}

// ── Audio chunking for outbound TTS ─────────────────────────────────

/// Chunk size: 160 bytes = 20ms at 8kHz mu-law.
const MULAW_CHUNK_SIZE: usize = 160;

/// Silence threshold: if no audio for this duration, treat as end-of-utterance.
const SILENCE_THRESHOLD: Duration = Duration::from_millis(800);

/// Minimum audio buffer to trigger transcription (avoid transcribing silence/noise).
const MIN_AUDIO_BYTES: usize = 3200; // 400ms at 8kHz

/// Callback for dispatching transcribed speech to the agent.
pub type SpeechCallback = Box<
    dyn Fn(String, String, String, String) -> futures::future::BoxFuture<'static, Option<String>>
        + Send
        + Sync,
>;

/// Handle a Twilio Media Stream WebSocket connection.
///
/// `speech_callback` is called with (account_id, call_id, caller, text) and
/// should return the agent's text response (or None to skip TTS).
///
/// `tts_fn` converts text to mu-law audio bytes for playback.
pub async fn handle_media_stream<S>(
    ws_stream: S,
    account_id: String,
    call_id: String,
    caller: String,
    speech_callback: Arc<SpeechCallback>,
    tts_fn: Arc<dyn Fn(&str) -> futures::future::BoxFuture<'static, Option<Vec<u8>>> + Send + Sync>,
) where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message>
        + Send
        + Unpin
        + 'static,
{
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // Channel for sending outbound messages to the WebSocket.
    let (out_tx, mut out_rx) = mpsc::channel::<String>(64);

    // Spawn the outbound writer.
    let write_handle = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_tx.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    let mut session: Option<MediaStreamSession> = None;
    let out = out_tx.clone();

    // Spawn a silence detector that checks periodically.
    let silence_audio_buf: Arc<RwLock<Vec<u8>>> = Arc::new(RwLock::new(Vec::new()));
    let silence_last_audio: Arc<RwLock<Instant>> = Arc::new(RwLock::new(Instant::now()));
    let silence_out = out_tx.clone();
    let silence_buf_clone = Arc::clone(&silence_audio_buf);
    let silence_time_clone = Arc::clone(&silence_last_audio);
    let silence_account = account_id.clone();
    let silence_call = call_id.clone();
    let silence_caller = caller.clone();
    let silence_cb = Arc::clone(&speech_callback);
    let silence_tts = Arc::clone(&tts_fn);
    let stream_sid_holder: Arc<RwLock<String>> = Arc::new(RwLock::new(String::new()));
    let stream_sid_for_silence = Arc::clone(&stream_sid_holder);

    let silence_handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;

            let elapsed = silence_time_clone.read().await.elapsed();
            let buf_len = silence_buf_clone.read().await.len();

            if elapsed >= SILENCE_THRESHOLD && buf_len >= MIN_AUDIO_BYTES {
                // End of utterance detected — transcribe and respond.
                let audio_data = {
                    let mut buf = silence_buf_clone.write().await;
                    std::mem::take(&mut *buf)
                };

                let ssid = stream_sid_for_silence.read().await.clone();
                if ssid.is_empty() {
                    continue;
                }

                // Clear Twilio's audio buffer (barge-in: stop any playing TTS).
                let clear_msg = serde_json::to_string(&TwilioOutbound::Clear {
                    stream_sid: ssid.clone(),
                })
                .unwrap_or_default();
                let _ = silence_out.send(clear_msg).await;

                // Convert mu-law to PCM for STT providers.
                // (Most STT providers accept mu-law directly, but we pass raw bytes.)
                debug!(
                    bytes = audio_data.len(),
                    "silence detected, dispatching audio for transcription"
                );

                // Call the speech callback with the transcribed text.
                // For now, we pass the audio to the callback which handles STT + agent.
                // In a full implementation, STT would be streaming.
                let response = silence_cb(
                    silence_account.clone(),
                    silence_call.clone(),
                    silence_caller.clone(),
                    // Pass audio as base64 for the callback to handle.
                    base64::engine::general_purpose::STANDARD.encode(&audio_data),
                )
                .await;

                // If we got a response, synthesize TTS and stream back.
                if let Some(text) = response {
                    if let Some(mulaw_audio) = silence_tts(&text).await {
                        // Stream audio back in 20ms chunks.
                        for chunk in mulaw_audio.chunks(MULAW_CHUNK_SIZE) {
                            let payload = base64::engine::general_purpose::STANDARD.encode(chunk);
                            let msg = serde_json::to_string(&TwilioOutbound::Media {
                                stream_sid: ssid.clone(),
                                media: OutboundMedia { payload },
                            })
                            .unwrap_or_default();
                            let _ = silence_out.send(msg).await;
                            // Pace at real-time (20ms per 160 bytes).
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        // Send a mark to know when playback finishes.
                        let mark_msg = serde_json::to_string(&TwilioOutbound::Mark {
                            stream_sid: ssid.clone(),
                            mark: OutboundMark {
                                name: format!("tts-response"),
                            },
                        })
                        .unwrap_or_default();
                        let _ = silence_out.send(mark_msg).await;
                    }
                }
            }
        }
    });

    // Main message loop.
    while let Some(msg_result) = ws_rx.next().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(e) => {
                warn!("media stream WebSocket error: {e}");
                break;
            },
        };

        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };

        let inbound: TwilioInbound = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(_) => continue,
        };

        match inbound {
            TwilioInbound::Connected {} => {
                debug!("media stream connected");
            },
            TwilioInbound::Start { stream_sid, start } => {
                let call_sid = start.call_sid.unwrap_or_default();
                info!(
                    stream_sid = %stream_sid,
                    call_sid = %call_sid,
                    "media stream started"
                );
                *stream_sid_holder.write().await = stream_sid.clone();
                session = Some(MediaStreamSession::new(
                    stream_sid,
                    call_sid,
                    account_id.clone(),
                    call_id.clone(),
                ));
            },
            TwilioInbound::Media { media, .. } => {
                if let Ok(audio_bytes) =
                    base64::engine::general_purpose::STANDARD.decode(&media.payload)
                {
                    silence_audio_buf
                        .write()
                        .await
                        .extend_from_slice(&audio_bytes);
                    *silence_last_audio.write().await = Instant::now();
                }
            },
            TwilioInbound::Mark { mark, .. } => {
                debug!(mark_name = %mark.name, "mark received");
                if let Some(ref mut s) = session {
                    s.bot_speaking = false;
                }
            },
            TwilioInbound::Stop { stream_sid } => {
                info!(stream_sid = %stream_sid, "media stream stopped");
                break;
            },
        }
    }

    // Clean up.
    silence_handle.abort();
    write_handle.abort();
    drop(out);
    info!(call_id = %call_id, "media stream session ended");
}

/// Generate a stream token for authenticating WebSocket connections.
pub fn generate_stream_token() -> String {
    let bytes: [u8; 16] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Build TwiML that connects to a Media Stream WebSocket.
pub fn build_stream_twiml(ws_url: &str, greeting: Option<&str>) -> Bytes {
    let mut twiml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><Response>"#);

    // Optionally speak a greeting before connecting the stream.
    if let Some(msg) = greeting {
        twiml.push_str(&format!(
            r#"<Say voice="Polly.Joanna">{}</Say>"#,
            msg.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        ));
    }

    twiml.push_str(&format!(r#"<Connect><Stream url="{ws_url}" /></Connect>"#));
    twiml.push_str("</Response>");
    Bytes::from(twiml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_start_event() {
        let json = r#"{
            "event": "start",
            "streamSid": "MZ123",
            "start": {
                "callSid": "CA456",
                "customParameters": {"token": "abc"},
                "mediaFormat": {"encoding": "audio/x-mulaw", "sampleRate": 8000, "channels": 1}
            }
        }"#;
        let msg: TwilioInbound = serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}"));
        match msg {
            TwilioInbound::Start { stream_sid, start } => {
                assert_eq!(stream_sid, "MZ123");
                assert_eq!(start.call_sid.as_deref(), Some("CA456"));
                assert_eq!(
                    start.custom_parameters.get("token").map(|s| s.as_str()),
                    Some("abc")
                );
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_media_event() {
        let json = r#"{
            "event": "media",
            "streamSid": "MZ123",
            "media": {
                "payload": "dGVzdA==",
                "timestamp": "1000"
            }
        }"#;
        let msg: TwilioInbound = serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}"));
        match msg {
            TwilioInbound::Media { media, .. } => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&media.payload)
                    .unwrap_or_default();
                assert_eq!(decoded, b"test");
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_stop_event() {
        let json = r#"{"event": "stop", "streamSid": "MZ123"}"#;
        let msg: TwilioInbound = serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(msg, TwilioInbound::Stop { .. }));
    }

    #[test]
    fn serialize_outbound_media() {
        let msg = TwilioOutbound::Media {
            stream_sid: "MZ123".into(),
            media: OutboundMedia {
                payload: "dGVzdA==".into(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap_or_default();
        assert!(json.contains("\"event\":\"media\""));
        assert!(json.contains("\"streamSid\":\"MZ123\""));
    }

    #[test]
    fn serialize_clear_event() {
        let msg = TwilioOutbound::Clear {
            stream_sid: "MZ123".into(),
        };
        let json = serde_json::to_string(&msg).unwrap_or_default();
        assert!(json.contains("\"event\":\"clear\""));
    }

    #[test]
    fn build_stream_twiml_with_greeting() {
        let twiml = build_stream_twiml("wss://example.com/stream", Some("Hello"));
        let s = std::str::from_utf8(&twiml).unwrap_or("");
        assert!(s.contains("<Say"));
        assert!(s.contains("Hello"));
        assert!(s.contains("<Connect><Stream url=\"wss://example.com/stream\""));
    }

    #[test]
    fn build_stream_twiml_without_greeting() {
        let twiml = build_stream_twiml("wss://example.com/stream", None);
        let s = std::str::from_utf8(&twiml).unwrap_or("");
        assert!(!s.contains("<Say"));
        assert!(s.contains("<Connect><Stream"));
    }

    #[test]
    fn generate_token_is_unique() {
        let t1 = generate_stream_token();
        let t2 = generate_stream_token();
        assert_ne!(t1, t2);
        assert!(!t1.is_empty());
    }

    #[test]
    fn parse_connected_event() {
        let json = r#"{"event": "connected"}"#;
        let msg: TwilioInbound = serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(msg, TwilioInbound::Connected {}));
    }

    #[test]
    fn parse_mark_event() {
        let json = r#"{"event": "mark", "streamSid": "MZ123", "mark": {"name": "tts-1"}}"#;
        let msg: TwilioInbound = serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}"));
        match msg {
            TwilioInbound::Mark { stream_sid, mark } => {
                assert_eq!(stream_sid, "MZ123");
                assert_eq!(mark.name, "tts-1");
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn serialize_mark_event() {
        let msg = TwilioOutbound::Mark {
            stream_sid: "MZ123".into(),
            mark: OutboundMark {
                name: "tts-42".into(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap_or_default();
        assert!(json.contains("\"event\":\"mark\""));
        assert!(json.contains("\"name\":\"tts-42\""));
    }

    #[test]
    fn stream_token_length() {
        let token = generate_stream_token();
        // 16 random bytes base64url-encoded = 22 chars (no padding)
        assert!(token.len() >= 20);
        // Should be URL-safe
        assert!(!token.contains('+'));
        assert!(!token.contains('/'));
    }
}
