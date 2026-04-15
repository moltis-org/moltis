use super::*;

#[test]
fn strip_mention_at_start() {
    assert_eq!(strip_bot_mention("<@123> hello world", 123), "hello world");
    assert_eq!(strip_bot_mention("<@!123> hello world", 123), "hello world");
}

#[test]
fn strip_mention_no_match() {
    assert_eq!(
        strip_bot_mention("hello <@123> world", 123),
        "hello <@123> world"
    );
    assert_eq!(strip_bot_mention("hello world", 123), "hello world");
}

#[test]
fn strip_mention_different_bot() {
    assert_eq!(strip_bot_mention("<@999> hello", 123), "<@999> hello");
}

#[test]
fn extract_location_coordinates_from_plain_pair() {
    let coords = extract_location_coordinates("48.8566, 2.3522")
        .unwrap_or_else(|| panic!("expected coordinates"));
    assert!((coords.0 - 48.8566).abs() < 1e-6);
    assert!((coords.1 - 2.3522).abs() < 1e-6);
}

#[test]
fn extract_location_coordinates_from_google_query() {
    let coords = extract_location_coordinates("https://www.google.com/maps?q=37.7749,-122.4194")
        .unwrap_or_else(|| panic!("expected coordinates"));
    assert!((coords.0 - 37.7749).abs() < 1e-6);
    assert!((coords.1 + 122.4194).abs() < 1e-6);
}

#[test]
fn extract_location_coordinates_from_google_path_marker() {
    let coords = extract_location_coordinates(
        "https://www.google.com/maps/place/test/@48.8566,2.3522,14z/data=!3m1!4b1",
    )
    .unwrap_or_else(|| panic!("expected coordinates"));
    assert!((coords.0 - 48.8566).abs() < 1e-6);
    assert!((coords.1 - 2.3522).abs() < 1e-6);
}

#[test]
fn extract_location_coordinates_from_apple_maps() {
    let coords = extract_location_coordinates("https://maps.apple.com/?ll=34.0522,-118.2437&z=12")
        .unwrap_or_else(|| panic!("expected coordinates"));
    assert!((coords.0 - 34.0522).abs() < 1e-6);
    assert!((coords.1 + 118.2437).abs() < 1e-6);
}

#[test]
fn extract_location_coordinates_rejects_non_location_text() {
    assert!(extract_location_coordinates("hey what's up?").is_none());
    assert!(extract_location_coordinates("my score is 1,2 today").is_none());
}

#[test]
fn chunk_short_message() {
    let chunks = chunk_message("hello", 2000);
    assert_eq!(chunks, vec!["hello"]);
}

#[test]
fn chunk_long_message() {
    let text = "a".repeat(4500);
    let chunks = chunk_message(&text, 2000);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].len(), 2000);
    assert_eq!(chunks[1].len(), 2000);
    assert_eq!(chunks[2].len(), 500);
}

#[test]
fn chunk_splits_at_newline() {
    let mut text = String::new();
    text.push_str(&"a".repeat(1500));
    text.push('\n');
    text.push_str(&"b".repeat(1000));
    let chunks = chunk_message(&text, 2000);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 1501); // 1500 + newline
    assert_eq!(chunks[1].len(), 1000);
}

#[test]
fn chunk_avoids_splitting_inside_code_fence() {
    // The code fence fits within max_len, so the split should land before
    // or after the fence — not inside it.
    let mut text = String::new();
    text.push_str(&"a".repeat(80));
    text.push('\n');
    text.push_str("```\n");
    text.push_str("code line 1\ncode line 2\n");
    text.push_str("```\n");
    text.push_str(&"b".repeat(80));
    // max_len = 120: the 80+newline prefix is 81 chars, which fits.
    // The code fence block is ~30 chars. A naive newline split at ~120
    // would land inside the fence. The markdown-aware splitter should
    // split at the newline before the fence (position 81).
    let chunks = chunk_message(&text, 120);
    for chunk in &chunks {
        let opens = chunk.matches("```").count();
        assert_eq!(opens % 2, 0, "unbalanced code fence in chunk: {chunk:?}");
    }
}

#[test]
fn chunk_message_handles_multibyte_boundary() {
    let text = format!("{} tail", "😀".repeat(600));
    let chunks = chunk_message(&text, 2001);
    assert!(chunks.len() >= 2);
    assert_eq!(chunks.concat(), text);
    for chunk in chunks {
        assert!(chunk.is_char_boundary(chunk.len()));
    }
}

/// Security: the OTP challenge message sent to the Discord user must
/// NEVER contain the verification code. The code should only be visible
/// to the admin in the web UI. If this test fails, unauthenticated users
/// can self-approve without admin involvement.
#[test]
fn security_otp_challenge_message_does_not_contain_code() {
    let msg = OTP_CHALLENGE_MSG;

    // Must not contain any 6-digit numeric sequences (OTP codes are 6 digits).
    let has_six_digits = msg
        .as_bytes()
        .windows(6)
        .any(|w| w.iter().all(|b| b.is_ascii_digit()));
    assert!(
        !has_six_digits,
        "SECURITY: OTP_CHALLENGE_MSG must not contain numeric codes"
    );
}

#[test]
fn chunk_code_fence_too_large_falls_back() {
    // When the code fence itself exceeds max_len, we must still split
    // (graceful degradation — can't avoid unbalanced fences here).
    let mut text = String::from("```\n");
    text.push_str(&"x".repeat(300));
    text.push_str("\n```\n");
    let chunks = chunk_message(&text, 100);
    assert!(chunks.len() >= 2, "should split oversized code fence");
    let reassembled: String = chunks.iter().copied().collect();
    assert_eq!(reassembled, text);
}

// ── OTP code detection tests ─────────────────────────────────────

#[test]
fn looks_like_otp_code_valid() {
    assert!(looks_like_otp_code("123456"));
    assert!(looks_like_otp_code("000000"));
    assert!(looks_like_otp_code("999999"));
}

#[test]
fn looks_like_otp_code_rejects_non_codes() {
    assert!(!looks_like_otp_code("hello"));
    assert!(!looks_like_otp_code("12345")); // too short
    assert!(!looks_like_otp_code("1234567")); // too long
    assert!(!looks_like_otp_code("12345a")); // not all digits
    assert!(!looks_like_otp_code("")); // empty
    assert!(!looks_like_otp_code("abcdef")); // no digits
    assert!(!looks_like_otp_code("12 345")); // space
}

#[test]
fn looks_like_otp_code_rejects_unicode_digits() {
    // Arabic-Indic digits (U+0660..U+0669) should not be accepted.
    assert!(!looks_like_otp_code(
        "\u{0660}\u{0661}\u{0662}\u{0663}\u{0664}\u{0665}"
    ));
}

// ── OTP message security tests ───────────────────────────────────

#[test]
fn security_otp_message_has_no_format_placeholders() {
    let msg = OTP_CHALLENGE_MSG;
    assert!(
        !msg.contains("{code}") && !msg.contains("{0}") && !msg.contains("%s"),
        "OTP challenge message must not contain format placeholders"
    );
}

#[test]
fn security_otp_message_points_to_web_ui() {
    let msg = OTP_CHALLENGE_MSG;
    assert!(
        msg.contains("Channels") && msg.contains("Senders"),
        "OTP message must tell user where to find the code"
    );
}

#[test]
fn otp_message_uses_discord_markdown_not_html() {
    let msg = OTP_CHALLENGE_MSG;
    // Discord uses ** for bold, not <b>.
    assert!(
        !msg.contains("<b>") && !msg.contains("<i>"),
        "OTP message should use Discord markdown, not HTML tags"
    );
    // Should use ** or nothing, but never HTML.
    assert!(
        !msg.contains("</"),
        "OTP message contains HTML closing tags"
    );
}

#[test]
fn otp_message_mentions_expiry() {
    let msg = OTP_CHALLENGE_MSG;
    assert!(
        msg.contains("5 minutes") || msg.contains("expires"),
        "OTP message should mention the expiry time"
    );
}

// ── Presence config mapping tests ────────────────────────────────

#[test]
fn required_intents_includes_reactions() {
    let intents = required_intents();
    assert!(
        intents.contains(GatewayIntents::GUILD_MESSAGE_REACTIONS),
        "must include GUILD_MESSAGE_REACTIONS"
    );
    assert!(
        intents.contains(GatewayIntents::DIRECT_MESSAGE_REACTIONS),
        "must include DIRECT_MESSAGE_REACTIONS"
    );
}

#[test]
fn required_intents_includes_message_content() {
    let intents = required_intents();
    assert!(
        intents.contains(GatewayIntents::MESSAGE_CONTENT),
        "must include MESSAGE_CONTENT for reading message text"
    );
    assert!(
        intents.contains(GatewayIntents::GUILD_MESSAGES),
        "must include GUILD_MESSAGES"
    );
    assert!(
        intents.contains(GatewayIntents::DIRECT_MESSAGES),
        "must include DIRECT_MESSAGES"
    );
    assert!(
        intents.contains(GatewayIntents::GUILDS),
        "must include GUILDS"
    );
}

#[test]
fn strip_mention_with_leading_whitespace() {
    assert_eq!(strip_bot_mention("  <@123> hello", 123), "hello");
}

#[test]
fn strip_mention_only_mention() {
    // When the message is just the mention, result should be empty after trim.
    assert_eq!(strip_bot_mention("<@123>", 123), "");
}

#[test]
fn discord_message_created_ms_from_snowflake() {
    // Example snowflake from Discord docs / Serenity tests.
    let id = MessageId::new(175_928_847_299_117_063);
    assert_eq!(discord_message_created_ms(id), 1_462_015_105_796);
}

// ── Inbound media attachment tests ───────────────────────────────

fn make_attachment(content_type: Option<&str>, filename: &str) -> Attachment {
    let content_type_json = match content_type {
        Some(ct) => format!("\"{ct}\""),
        None => "null".to_string(),
    };
    let json = format!(
        r#"{{
                "id": "1",
                "filename": "{filename}",
                "size": 1024,
                "url": "https://cdn.discordapp.com/attachments/1/2/{filename}",
                "proxy_url": "https://media.discordapp.net/attachments/1/2/{filename}",
                "content_type": {content_type_json}
            }}"#
    );
    match serde_json::from_str(&json) {
        Ok(attachment) => attachment,
        Err(error) => panic!("attachment json should deserialize: {error}"),
    }
}

#[test]
fn audio_format_maps_common_mime_types() {
    assert_eq!(
        audio_format_from_attachment(Some("audio/ogg"), "x.ogg"),
        "ogg"
    );
    assert_eq!(
        audio_format_from_attachment(Some("audio/ogg; codecs=opus"), "voice.ogg"),
        "ogg"
    );
    assert_eq!(
        audio_format_from_attachment(Some("audio/mpeg"), "x.mp3"),
        "mp3"
    );
    assert_eq!(
        audio_format_from_attachment(Some("audio/mp3"), "x.mp3"),
        "mp3"
    );
    assert_eq!(
        audio_format_from_attachment(Some("audio/mp4"), "x.m4a"),
        "m4a"
    );
    assert_eq!(
        audio_format_from_attachment(Some("audio/x-m4a"), "x.m4a"),
        "m4a"
    );
    assert_eq!(
        audio_format_from_attachment(Some("audio/wav"), "x.wav"),
        "wav"
    );
    assert_eq!(
        audio_format_from_attachment(Some("audio/webm"), "x.webm"),
        "webm"
    );
}

#[test]
fn audio_format_falls_back_to_extension() {
    assert_eq!(audio_format_from_attachment(None, "note.mp3"), "mp3");
    assert_eq!(audio_format_from_attachment(None, "note.Ogg"), "ogg");
    assert_eq!(audio_format_from_attachment(None, "note.opus"), "ogg");
    assert_eq!(audio_format_from_attachment(None, "note.webm"), "webm");
    // Unknown: default to ogg (Discord voice messages are OGG Opus).
    assert_eq!(audio_format_from_attachment(None, "mystery.bin"), "ogg");
}

#[test]
fn image_media_type_fallback_picks_extension() {
    assert_eq!(
        image_media_type_fallback(Some("image/png; charset=binary"), "x.png"),
        "image/png"
    );
    assert_eq!(
        image_media_type_fallback(None, "screenshot.PNG"),
        "image/png"
    );
    assert_eq!(image_media_type_fallback(None, "pic.gif"), "image/gif");
    assert_eq!(image_media_type_fallback(None, "pic.webp"), "image/webp");
    // Unknown: default to jpeg.
    assert_eq!(image_media_type_fallback(None, "pic.bin"), "image/jpeg");
}

#[test]
fn voice_stt_unavailable_body_uses_marker_and_caption() {
    assert_eq!(
        voice_stt_unavailable_log_body(""),
        "[Voice message - STT unavailable]"
    );
    assert_eq!(
        voice_stt_unavailable_log_body("caption"),
        "caption\n\n[Voice message - STT unavailable]"
    );
}

#[test]
fn attachment_is_audio_detects_mime_and_extension() {
    assert!(attachment_is_audio(&make_attachment(
        Some("audio/ogg"),
        "v.ogg"
    )));
    assert!(attachment_is_audio(&make_attachment(None, "v.opus")));
    assert!(attachment_is_audio(&make_attachment(None, "clip.MP3")));
    assert!(!attachment_is_audio(&make_attachment(
        Some("image/png"),
        "x.png"
    )));
    assert!(!attachment_is_audio(&make_attachment(None, "notes.txt")));
}

#[test]
fn attachment_is_image_detects_mime_and_extension() {
    assert!(attachment_is_image(&make_attachment(
        Some("image/jpeg"),
        "x.jpg"
    )));
    assert!(attachment_is_image(&make_attachment(None, "x.PNG")));
    assert!(attachment_is_image(&make_attachment(None, "x.webp")));
    assert!(!attachment_is_image(&make_attachment(
        Some("audio/ogg"),
        "v.ogg"
    )));
    assert!(!attachment_is_image(&make_attachment(None, "doc.pdf")));
}

#[test]
fn select_media_prefers_audio_over_image() {
    let image = make_attachment(Some("image/png"), "a.png");
    let audio = make_attachment(Some("audio/ogg"), "b.ogg");
    let attachments = vec![image, audio];
    let picked = match select_media_attachment(&attachments) {
        Some(attachment) => attachment,
        None => panic!("expected audio attachment to be selected"),
    };
    assert_eq!(picked.filename, "b.ogg");
}

#[test]
fn select_media_returns_none_for_unsupported() {
    let doc = make_attachment(Some("application/pdf"), "spec.pdf");
    assert!(select_media_attachment(&[doc]).is_none());
    assert!(select_media_attachment(&[]).is_none());
}

#[test]
fn select_media_picks_image_when_no_audio() {
    let doc = make_attachment(Some("application/pdf"), "spec.pdf");
    let image = make_attachment(Some("image/jpeg"), "pic.jpg");
    let attachments = [doc, image];
    let picked = match select_media_attachment(&attachments) {
        Some(attachment) => attachment,
        None => panic!("expected image attachment to be selected"),
    };
    assert_eq!(picked.filename, "pic.jpg");
}

// ── resolve_discord_inbound_media via mock sink ──────────────────
//
// These tests exercise the branching logic without hitting the network.
// We use a minimal sink mock that only implements the required trait
// methods; everything else falls through to trait defaults.

use {
    image::{DynamicImage, ImageBuffer, ImageFormat, Rgb},
    std::io::Cursor,
};

use moltis_channels::Result as ChannelsResult;

enum MockTranscription {
    Success(String),
    Failure(String),
}

enum MockDownload {
    Success(Vec<u8>),
    Failure(String),
}

struct MockDownloader {
    outcome: MockDownload,
}

#[async_trait]
impl InboundMediaDownloader for MockDownloader {
    async fn download_media(
        &self,
        _source: &InboundMediaSource,
        _max_bytes: usize,
    ) -> ChannelsResult<Vec<u8>> {
        match &self.outcome {
            MockDownload::Success(bytes) => Ok(bytes.clone()),
            MockDownload::Failure(message) => Err(ChannelError::unavailable(message)),
        }
    }
}

struct MockSink {
    stt_available: bool,
    transcription: MockTranscription,
}

#[async_trait]
impl ChannelEventSink for MockSink {
    async fn emit(&self, _event: ChannelEvent) {}

    async fn dispatch_to_chat(
        &self,
        _text: &str,
        _reply_to: ChannelReplyTarget,
        _meta: ChannelMessageMeta,
    ) {
    }

    async fn dispatch_command(
        &self,
        _command: &str,
        _reply_to: ChannelReplyTarget,
        _sender_id: Option<&str>,
    ) -> ChannelsResult<String> {
        Ok(String::new())
    }

    async fn transcribe_voice(&self, _audio_data: &[u8], _format: &str) -> ChannelsResult<String> {
        match &self.transcription {
            MockTranscription::Success(text) => Ok(text.clone()),
            MockTranscription::Failure(message) => Err(ChannelError::unavailable(message)),
        }
    }

    async fn request_disable_account(&self, _channel_type: &str, _account_id: &str, _reason: &str) {
    }

    async fn voice_stt_available(&self) -> bool {
        self.stt_available
    }
}

#[tokio::test]
async fn resolve_returns_no_media_when_attachments_empty() {
    let sink = MockSink {
        stt_available: true,
        transcription: MockTranscription::Success(String::new()),
    };
    let downloader = MockDownloader {
        outcome: MockDownload::Success(Vec::new()),
    };
    let outcome = resolve_discord_inbound_media(&[], "hello", &sink, "acct", &downloader).await;
    assert!(matches!(outcome, MediaResolveOutcome::NoMedia));
}

#[tokio::test]
async fn resolve_returns_no_media_when_only_unsupported() {
    let sink = MockSink {
        stt_available: true,
        transcription: MockTranscription::Success(String::new()),
    };
    let downloader = MockDownloader {
        outcome: MockDownload::Success(Vec::new()),
    };
    let doc = make_attachment(Some("application/pdf"), "spec.pdf");
    let outcome = resolve_discord_inbound_media(&[doc], "", &sink, "acct", &downloader).await;
    assert!(matches!(outcome, MediaResolveOutcome::NoMedia));
}

#[tokio::test]
async fn resolve_voice_without_stt_returns_unavailable() {
    let sink = MockSink {
        stt_available: false,
        transcription: MockTranscription::Success(String::new()),
    };
    let downloader = MockDownloader {
        outcome: MockDownload::Success(Vec::new()),
    };
    let voice = make_attachment(Some("audio/ogg"), "v.ogg");
    let outcome = resolve_discord_inbound_media(&[voice], "", &sink, "acct", &downloader).await;
    assert!(matches!(outcome, MediaResolveOutcome::VoiceSttUnavailable));
}

fn tiny_jpeg() -> Vec<u8> {
    let image = ImageBuffer::from_pixel(1, 1, Rgb([255, 0, 0]));
    let mut cursor = Cursor::new(Vec::new());
    let write_result = DynamicImage::ImageRgb8(image).write_to(&mut cursor, ImageFormat::Jpeg);
    match write_result {
        Ok(()) => cursor.into_inner(),
        Err(error) => panic!("tiny jpeg generation should succeed: {error}"),
    }
}

#[tokio::test]
async fn resolve_voice_success_combines_caption_and_transcript() {
    let sink = MockSink {
        stt_available: true,
        transcription: MockTranscription::Success("transcribed discord voice".to_string()),
    };
    let downloader = MockDownloader {
        outcome: MockDownload::Success(b"voice-bytes".to_vec()),
    };
    let voice = make_attachment(Some("audio/ogg"), "v.ogg");

    let outcome =
        resolve_discord_inbound_media(&[voice], "caption", &sink, "acct", &downloader).await;

    let media = match outcome {
        MediaResolveOutcome::Media(media) => media,
        _ => panic!("expected voice media outcome"),
    };
    assert_eq!(
        media.body,
        "caption\n\n[Voice message]: transcribed discord voice"
    );
    assert!(media.attachments.is_empty());
    assert!(matches!(media.kind, ChannelMessageKind::Voice));
    let (voice_audio, format) = match media.voice_audio {
        Some(audio) => audio,
        None => panic!("expected saved voice audio"),
    };
    assert_eq!(voice_audio, b"voice-bytes".to_vec());
    assert_eq!(format, "ogg");
}

#[tokio::test]
async fn resolve_voice_download_failure_falls_back_to_marker() {
    let sink = MockSink {
        stt_available: true,
        transcription: MockTranscription::Success(String::new()),
    };
    let downloader = MockDownloader {
        outcome: MockDownload::Failure("boom".to_string()),
    };
    let voice = make_attachment(Some("audio/ogg"), "v.ogg");

    let outcome = resolve_discord_inbound_media(&[voice], "", &sink, "acct", &downloader).await;

    let media = match outcome {
        MediaResolveOutcome::Media(media) => media,
        _ => panic!("expected voice media fallback outcome"),
    };
    assert_eq!(media.body, "[Voice message - download failed]");
    assert!(media.attachments.is_empty());
    assert!(media.voice_audio.is_none());
    assert!(matches!(media.kind, ChannelMessageKind::Voice));
}

#[tokio::test]
async fn resolve_voice_transcription_failure_falls_back_to_caption() {
    let sink = MockSink {
        stt_available: true,
        transcription: MockTranscription::Failure("stt offline".to_string()),
    };
    let downloader = MockDownloader {
        outcome: MockDownload::Success(b"voice-bytes".to_vec()),
    };
    let voice = make_attachment(Some("audio/ogg"), "v.ogg");

    let outcome =
        resolve_discord_inbound_media(&[voice], "caption", &sink, "acct", &downloader).await;

    let media = match outcome {
        MediaResolveOutcome::Media(media) => media,
        _ => panic!("expected voice media fallback outcome"),
    };
    assert_eq!(media.body, "caption");
    assert!(media.attachments.is_empty());
    assert!(matches!(media.kind, ChannelMessageKind::Voice));
    assert!(media.voice_audio.is_some());
}

#[tokio::test]
async fn resolve_image_success_builds_multimodal_attachment() {
    let sink = MockSink {
        stt_available: true,
        transcription: MockTranscription::Success(String::new()),
    };
    let downloader = MockDownloader {
        outcome: MockDownload::Success(tiny_jpeg()),
    };
    let image = make_attachment(Some("image/jpeg"), "pic.jpg");

    let outcome =
        resolve_discord_inbound_media(&[image], "diagram", &sink, "acct", &downloader).await;

    let media = match outcome {
        MediaResolveOutcome::Media(media) => media,
        _ => panic!("expected image media outcome"),
    };
    assert_eq!(media.body, "diagram");
    assert_eq!(media.attachments.len(), 1);
    assert!(matches!(media.kind, ChannelMessageKind::Photo));
    assert!(media.voice_audio.is_none());
    assert_eq!(media.attachments[0].media_type, "image/jpeg");
    assert!(!media.attachments[0].data.is_empty());
}
