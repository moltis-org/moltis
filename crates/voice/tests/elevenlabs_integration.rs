//! Live integration tests for the ElevenLabs TTS provider.
//!
//! These tests hit the real ElevenLabs API and require `ELEVENLABS_API_KEY` in
//! the environment. They are `#[ignore]`d by default so `cargo test` skips them.
//!
//! Run with:
//!   cargo test -p moltis-voice --test elevenlabs_integration -- --ignored
//!
//! Covers regressions for:
//! - #735: custom ElevenLabs voices don't work

#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    moltis_voice::tts::{AudioFormat, ElevenLabsTts, SynthesizeRequest, TtsProvider},
    secrecy::Secret,
};

fn api_key() -> Secret<String> {
    let key = std::env::var("ELEVENLABS_API_KEY")
        .expect("ELEVENLABS_API_KEY must be set for integration tests");
    Secret::new(key)
}

fn make_provider() -> ElevenLabsTts {
    ElevenLabsTts::new(Some(api_key()))
}

// ── Basic TTS synthesis ──────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn synthesize_premade_voice() {
    let provider = make_provider();

    let request = SynthesizeRequest {
        text: "Hello, this is a test.".into(),
        voice_id: Some("21m00Tcm4TlvDq8ikWAM".into()), // Rachel (premade)
        model: Some("eleven_flash_v2_5".into()),
        output_format: AudioFormat::Mp3,
        ..Default::default()
    };

    let output = provider
        .synthesize(request)
        .await
        .expect("premade voice synthesis should succeed");

    assert!(
        output.data.len() > 100,
        "audio data should be non-trivial: {} bytes",
        output.data.len()
    );
    assert_eq!(output.format, AudioFormat::Mp3);
}

#[tokio::test]
#[ignore]
async fn synthesize_custom_voice_from_catalog() {
    let provider = make_provider();

    // List voices from the account to find custom (non-premade) voices.
    let voices = match provider.voices().await {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "Skipping custom voice test: cannot list voices (API key may lack voices_read): {e}"
            );
            return;
        },
    };

    eprintln!("Account has {} voice(s):", voices.len());
    for v in &voices {
        eprintln!("  {} — {}", v.id, v.name);
    }

    if voices.is_empty() {
        eprintln!("Skipping: no voices available on account");
        return;
    }

    // Try every voice — the bug is that custom voices fail while premade ones work.
    for voice in &voices {
        let request = SynthesizeRequest {
            text: "Testing this voice.".into(),
            voice_id: Some(voice.id.clone()),
            model: Some("eleven_flash_v2_5".into()),
            output_format: AudioFormat::Mp3,
            ..Default::default()
        };

        let result = provider.synthesize(request).await;
        match &result {
            Ok(output) => {
                assert!(
                    output.data.len() > 100,
                    "voice {} ({}) returned too few bytes: {}",
                    voice.id,
                    voice.name,
                    output.data.len()
                );
                eprintln!(
                    "  ✓ {} ({}) — {} bytes",
                    voice.id,
                    voice.name,
                    output.data.len()
                );
            },
            Err(e) => {
                panic!(
                    "Voice {} ({}) failed to synthesize: {e}\n\
                     This confirms the bug: custom voices fail while premade ones work.",
                    voice.id, voice.name
                );
            },
        }
    }
}

#[tokio::test]
#[ignore]
async fn synthesize_falls_back_to_configured_default_voice() {
    // Verifies the voice_id: None → default_voice_id fallback path used by
    // the UI "Test" button (tts.convert is called without a voiceId param).
    let configured_voice_id = "21m00Tcm4TlvDq8ikWAM"; // Rachel (premade)

    let provider = ElevenLabsTts::with_defaults(
        Some(api_key()),
        Some(configured_voice_id.into()),
        Some("eleven_flash_v2_5".into()),
    );

    // No voice_id in request — mirrors how testTts() works in the UI.
    let request = SynthesizeRequest {
        text: "Testing default voice from config.".into(),
        voice_id: None,
        model: None,
        output_format: AudioFormat::Mp3,
        ..Default::default()
    };

    let output = provider
        .synthesize(request)
        .await
        .expect("synthesis with default voice from config should succeed");

    assert!(
        output.data.len() > 100,
        "audio should be non-trivial: {} bytes",
        output.data.len()
    );
}

// ── Voice listing ────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn list_voices_returns_non_empty() {
    let provider = make_provider();

    match provider.voices().await {
        Ok(voices) => {
            eprintln!("Listed {} voice(s)", voices.len());
            for v in &voices {
                eprintln!("  {} — {} (desc: {:?})", v.id, v.name, v.description);
            }
            // If the key has voices_read permission, should return voices.
            assert!(!voices.is_empty(), "voices list should not be empty");
        },
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("401") || msg.contains("missing_permissions") {
                eprintln!("Skipping: API key lacks voices_read permission: {msg}");
            } else {
                panic!("Unexpected error listing voices: {e}");
            }
        },
    }
}

// ── Error handling ───────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn synthesize_invalid_voice_returns_error() {
    let provider = make_provider();

    let request = SynthesizeRequest {
        text: "This should fail.".into(),
        voice_id: Some("nonexistent_voice_id_00000".into()),
        output_format: AudioFormat::Mp3,
        ..Default::default()
    };

    let result = provider.synthesize(request).await;
    assert!(
        result.is_err(),
        "synthesis with nonexistent voice_id should fail"
    );
    let err = result.unwrap_err().to_string();
    eprintln!("Expected error for invalid voice: {err}");
}

// ── Model compatibility ──────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn synthesize_with_multilingual_model() {
    let provider = make_provider();

    let request = SynthesizeRequest {
        text: "Bonjour, comment allez-vous?".into(),
        voice_id: Some("21m00Tcm4TlvDq8ikWAM".into()),
        model: Some("eleven_multilingual_v2".into()),
        output_format: AudioFormat::Mp3,
        ..Default::default()
    };

    let output = provider
        .synthesize(request)
        .await
        .expect("multilingual model synthesis should succeed");

    assert!(
        output.data.len() > 100,
        "audio should be non-trivial: {} bytes",
        output.data.len()
    );
}
