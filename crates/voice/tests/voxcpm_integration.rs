//! Wire-contract tests for the VoxCPM TTS provider.
//!
//! These run against a `wiremock` stand-in for a vLLM-Omni speech server, so
//! they assert the exact request shape Moltis puts on the wire and the exact
//! responses it accepts. That matters because VoxCPM validates `voice` against
//! the speakers the server knows and rejects unknown names, so sending a
//! placeholder the way the OpenAI provider does would fail at runtime.
//!
//! Run with:
//!   cargo test -p moltis-voice --test voxcpm_integration

#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    moltis_voice::{
        VoxCpmTtsConfig,
        tts::{AudioFormat, SynthesizeRequest, TtsProvider, VoxCpmTts},
    },
    serde_json::{Value, json},
    wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    },
};

/// Build a provider pointed at the mock server.
fn provider(server: &MockServer, model: Option<&str>, voice: Option<&str>) -> VoxCpmTts {
    VoxCpmTts::new(&VoxCpmTtsConfig {
        endpoint: format!("{}/v1", server.uri()),
        model: model.map(ToString::to_string),
        voice: voice.map(ToString::to_string),
        voice_design: true,
    })
}

/// Capture the JSON body of the single recorded request.
async fn recorded_body(server: &MockServer) -> Value {
    let requests = server
        .received_requests()
        .await
        .expect("mock server should record requests");
    assert_eq!(requests.len(), 1, "expected exactly one request");
    serde_json::from_slice(&requests[0].body).expect("request body should be JSON")
}

// ── Synthesis ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn synthesize_omits_voice_for_zero_shot() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"RIFFfake-wav".to_vec()))
        .mount(&server)
        .await;

    let output = provider(&server, Some("openbmb/VoxCPM2"), None)
        .synthesize(SynthesizeRequest {
            text: "Hello from Moltis.".into(),
            output_format: AudioFormat::Wav,
            ..Default::default()
        })
        .await
        .expect("synthesis should succeed");

    assert_eq!(output.format, AudioFormat::Wav);
    assert_eq!(&output.data[..], b"RIFFfake-wav");

    let body = recorded_body(&server).await;
    assert_eq!(body["input"], "Hello from Moltis.");
    assert_eq!(body["model"], "openbmb/VoxCPM2");
    assert_eq!(body["response_format"], "wav");
    // The absence of `voice` is what selects zero-shot synthesis. A default
    // like "alloy" would be rejected by VoxCPM's speaker validation.
    assert!(
        body.get("voice").is_none(),
        "voice must be omitted when no speaker is configured, got {body}"
    );
    assert!(
        body.get("speed").is_none(),
        "speed must be omitted when unset"
    );
}

#[tokio::test]
async fn synthesize_sends_configured_speaker() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"audio".to_vec()))
        .mount(&server)
        .await;

    provider(&server, None, Some("alice"))
        .synthesize(SynthesizeRequest {
            text: "Cloned voice.".into(),
            output_format: AudioFormat::Mp3,
            ..Default::default()
        })
        .await
        .expect("synthesis should succeed");

    let body = recorded_body(&server).await;
    assert_eq!(body["voice"], "alice");
    assert_eq!(body["response_format"], "mp3");
}

#[tokio::test]
async fn request_voice_id_overrides_configured_speaker() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"audio".to_vec()))
        .mount(&server)
        .await;

    provider(&server, None, Some("alice"))
        .synthesize(SynthesizeRequest {
            text: "Override.".into(),
            voice_id: Some("bob".into()),
            ..Default::default()
        })
        .await
        .expect("synthesis should succeed");

    assert_eq!(recorded_body(&server).await["voice"], "bob");
}

#[tokio::test]
async fn persona_instructions_become_a_voice_design_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"audio".to_vec()))
        .mount(&server)
        .await;

    provider(&server, None, None)
        .synthesize(SynthesizeRequest {
            text: "Good evening, sir.".into(),
            // Shape produced by `VoicePersonaPrompt::render`.
            instructions: Some("Persona: Alfred\nStyle: Dry wit".into()),
            ..Default::default()
        })
        .await
        .expect("synthesis should succeed");

    let body = recorded_body(&server).await;
    // Field labels are stripped: measured against a real VoxCPM2 server, a
    // surviving `Key:` label collapses the effect back to the no-prefix
    // baseline, and `Persona:` carries an internal name rather than voice
    // direction.
    assert_eq!(
        body["input"], "(Dry wit)Good evening, sir.",
        "personas must arrive as a plain VoxCPM voice-design prefix"
    );
    // VoxCPM2's serving adapter ignores `instructions`; sending it would be
    // dead weight at best and a 400 on a strict server at worst.
    assert!(body.get("instructions").is_none());
}

#[tokio::test]
async fn voice_design_can_be_disabled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"audio".to_vec()))
        .mount(&server)
        .await;

    let tts = VoxCpmTts::new(&VoxCpmTtsConfig {
        endpoint: format!("{}/v1", server.uri()),
        voice_design: false,
        ..Default::default()
    });
    tts.synthesize(SynthesizeRequest {
        text: "Plain text.".into(),
        instructions: Some("Style: Dry wit".into()),
        ..Default::default()
    })
    .await
    .expect("synthesis should succeed");

    assert_eq!(recorded_body(&server).await["input"], "Plain text.");
}

#[tokio::test]
async fn unsupported_format_falls_back_and_reports_honestly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"audio".to_vec()))
        .mount(&server)
        .await;

    // vLLM-Omni serves no AAC, so the provider must ask for something it does
    // serve and report the format it actually received.
    let output = provider(&server, None, None)
        .synthesize(SynthesizeRequest {
            text: "Hi.".into(),
            output_format: AudioFormat::Aac,
            ..Default::default()
        })
        .await
        .expect("synthesis should succeed");

    assert_eq!(output.format, AudioFormat::Mp3);
    assert_eq!(recorded_body(&server).await["response_format"], "mp3");
}

#[tokio::test]
async fn server_error_body_is_surfaced() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string("Invalid voice 'alloy'. Supported: alice, bob"),
        )
        .mount(&server)
        .await;

    let err = provider(&server, None, Some("alloy"))
        .synthesize(SynthesizeRequest {
            text: "Hi.".into(),
            ..Default::default()
        })
        .await
        .expect_err("a 400 must not be reported as success");

    let message = err.to_string();
    assert!(
        message.contains("400"),
        "status should be surfaced: {message}"
    );
    assert!(
        message.contains("Supported: alice, bob"),
        "server explanation should be surfaced: {message}"
    );
}

#[tokio::test]
async fn empty_audio_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(Vec::new()))
        .mount(&server)
        .await;

    let err = provider(&server, None, None)
        .synthesize(SynthesizeRequest {
            text: "Hi.".into(),
            ..Default::default()
        })
        .await
        .expect_err("an empty 200 must not be treated as audio");

    assert!(err.to_string().contains("empty"), "got: {err}");
}

#[tokio::test]
async fn unreachable_server_explains_how_to_start_it() {
    // Port 1 is reserved and will refuse the connection immediately.
    let tts = VoxCpmTts::new(&VoxCpmTtsConfig {
        endpoint: "http://127.0.0.1:1/v1".into(),
        ..Default::default()
    });

    let err = tts
        .synthesize(SynthesizeRequest {
            text: "Hi.".into(),
            ..Default::default()
        })
        .await
        .expect_err("an unreachable server must error");

    let message = err.to_string();
    assert!(
        message.contains("vllm serve openbmb/VoxCPM2"),
        "got: {message}"
    );
}

// ── Voice listing ───────────────────────────────────────────────────────────

#[tokio::test]
async fn voices_are_read_from_the_server() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/audio/voices"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "voices": ["alice", "preset_one"],
            "uploaded_voices": [
                {"name": "alice", "speaker_description": "warm narrator"}
            ]
        })))
        .mount(&server)
        .await;

    let voices = provider(&server, None, None)
        .voices()
        .await
        .expect("voices should be listed");

    assert_eq!(voices.len(), 2);
    let alice = voices.iter().find(|v| v.id == "alice").expect("alice");
    assert_eq!(alice.description.as_deref(), Some("warm narrator"));
    let preset = voices
        .iter()
        .find(|v| v.id == "preset_one")
        .expect("preset");
    assert!(preset.description.is_none());
}

#[tokio::test]
async fn voices_error_is_not_silently_swallowed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/audio/voices"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let err = provider(&server, None, None)
        .voices()
        .await
        .expect_err("a 500 must not be reported as an empty voice list");
    assert!(err.to_string().contains("500"), "got: {err}");
}

#[tokio::test]
async fn trailing_slash_in_endpoint_does_not_double_up_the_path() {
    let server = MockServer::start().await;
    // Matching on the exact path proves no `//audio/speech` is emitted.
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"audio".to_vec()))
        .mount(&server)
        .await;

    let tts = VoxCpmTts::new(&VoxCpmTtsConfig {
        endpoint: format!("{}/v1/", server.uri()),
        ..Default::default()
    });
    tts.synthesize(SynthesizeRequest {
        text: "Hi.".into(),
        ..Default::default()
    })
    .await
    .expect("synthesis should succeed against a trailing-slash endpoint");
}
