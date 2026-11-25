use super::*;

#[test]
fn unknown_voice_tts_list_provider_warned() {
    let toml = r#"
[voice.tts]
providers = ["openai-tts", "not-a-provider"]
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "voice.tts.providers[1]");
    assert!(
        warning.is_some(),
        "expected warning for unknown voice.tts.providers entry"
    );
}

#[test]
fn unknown_voice_stt_list_provider_warned() {
    let toml = r#"
[voice.stt]
providers = ["whisper", "not-a-provider"]
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "voice.stt.providers[1]");
    assert!(
        warning.is_some(),
        "expected warning for unknown voice.stt.providers entry"
    );
}

#[test]
fn known_voice_provider_list_entries_not_warned() {
    let toml = r#"
[voice.tts]
providers = ["openai", "google-tts", "coqui"]

[voice.stt]
providers = ["elevenlabs", "whisper-cli", "sherpa-onnx"]
"#;
    let result = validate_toml_str(toml);
    let warnings: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| {
            d.category == "unknown-field"
                && (d.path.starts_with("voice.tts.providers")
                    || d.path.starts_with("voice.stt.providers"))
        })
        .collect();
    assert!(
        warnings.is_empty(),
        "known voice provider list values should not warn: {warnings:?}"
    );
}

#[test]
fn gemini_stt_section_is_known() {
    let toml = r#"
[voice.stt]
provider = "gemini"
providers = ["gemini"]

[voice.stt.gemini]
enabled = true
model = "gemini-3.5-transcribe"
language = "fr-FR"
"#;
    let result = validate_toml_str(toml);
    let gemini_diagnostics: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.path.starts_with("voice.stt"))
        .collect();
    assert!(
        gemini_diagnostics.is_empty(),
        "gemini STT config should validate cleanly: {gemini_diagnostics:?}"
    );

    let config: MoltisConfig = toml::from_str(toml).unwrap();
    assert_eq!(
        config.voice.stt.provider,
        Some(crate::schema::VoiceSttProvider::Gemini)
    );
    assert_eq!(
        config.voice.stt.gemini.model.as_deref(),
        Some("gemini-3.5-transcribe")
    );
}
