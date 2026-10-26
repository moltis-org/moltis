//! VoxCPM TTS provider, served through a vLLM-Omni speech endpoint.
//!
//! [VoxCPM](https://github.com/OpenBMB/VoxCPM) is a tokenizer-free TTS model
//! released under Apache-2.0. It is served here through
//! [vLLM-Omni](https://github.com/vllm-project/vllm-omni), which exposes an
//! OpenAI-compatible speech API extended with VoxCPM-specific fields:
//!
//! ```bash
//! vllm serve openbmb/VoxCPM2 --omni --host 0.0.0.0 --port 8000
//! ```
//!
//! Two endpoints are used:
//!
//! - `POST {endpoint}/audio/speech` — synthesis
//! - `GET  {endpoint}/audio/voices` — the speakers the server actually has
//!
//! Unlike the OpenAI provider, `voice` is never defaulted: VoxCPM rejects
//! speaker names it does not know, and omitting the field selects zero-shot
//! synthesis. Voice personas are applied as a VoxCPM *voice design* prefix on
//! the input text, because the VoxCPM2 serving adapter reads the description
//! from the text rather than from a separate `instructions` field.

use {
    crate::{
        config::VoxCpmTtsConfig,
        tts::{AudioFormat, AudioOutput, SynthesizeRequest, TtsProvider, Voice},
    },
    anyhow::{Result, anyhow},
    async_trait::async_trait,
    bytes::Bytes,
    reqwest::Client,
    serde::{Deserialize, Serialize},
    std::{borrow::Cow, time::Duration},
};

/// Default vLLM-Omni OpenAI-compatible base URL.
const DEFAULT_ENDPOINT: &str = "http://localhost:8000/v1";

/// Synthesis can be slow on CPU-bound or cold servers.
const SYNTHESIZE_TIMEOUT: Duration = Duration::from_secs(120);

/// Listing voices is a cheap metadata call.
const VOICES_TIMEOUT: Duration = Duration::from_secs(5);

/// VoxCPM TTS provider backed by a vLLM-Omni server.
#[derive(Clone, Debug)]
pub struct VoxCpmTts {
    client: Client,
    endpoint: String,
    model: Option<String>,
    voice: Option<String>,
    voice_design: bool,
}

impl VoxCpmTts {
    /// Create a new VoxCPM provider from config.
    #[must_use]
    pub fn new(config: &VoxCpmTtsConfig) -> Self {
        Self {
            client: Client::new(),
            endpoint: normalize_endpoint(&config.endpoint),
            model: non_empty(config.model.as_deref()),
            voice: non_empty(config.voice.as_deref()),
            voice_design: config.voice_design,
        }
    }

    /// Map a moltis audio format onto a vLLM-Omni `response_format`.
    ///
    /// vLLM-Omni supports `wav`, `mp3`, `flac`, `pcm` and `opus`. Formats it
    /// does not serve fall back to the closest one it does, and the format
    /// actually returned is reported back so callers never mislabel the bytes.
    fn negotiate_format(requested: AudioFormat) -> (&'static str, AudioFormat) {
        match requested {
            AudioFormat::Mp3 | AudioFormat::Aac => ("mp3", AudioFormat::Mp3),
            AudioFormat::Opus | AudioFormat::Webm => ("opus", AudioFormat::Opus),
            AudioFormat::Pcm => ("pcm", AudioFormat::Pcm),
            AudioFormat::Wav => ("wav", AudioFormat::Wav),
        }
    }
}

/// Trim a trailing slash so `{endpoint}/audio/speech` never doubles up.
fn normalize_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_ENDPOINT.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Treat blank config strings as unset.
fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

/// Flatten a persona instruction block into a VoxCPM voice-design clause.
///
/// `VoicePersonaPrompt::render` emits newline-separated `Key: value` lines.
/// VoxCPM reads a voice description from a `(...)` prefix on the text, but it
/// only responds to a *plain* description: measuring F0 against a real
/// `openbmb/VoxCPM2` server showed that any surviving `Key:` label collapses
/// the effect back to the no-prefix baseline, while the same words without
/// labels shift pitch substantially. A single `Profile:` is enough to break it.
///
/// So the field labels are stripped and only the values are kept. The
/// `Persona:` line is dropped outright — it carries the persona's internal
/// name, which describes the identity rather than how the voice should sound.
/// Parentheses are removed so they cannot terminate the prefix early.
fn flatten_voice_design(instructions: &str) -> String {
    /// Labels emitted by `VoicePersonaPrompt::render`.
    const VALUE_LABELS: [&str; 6] = [
        "Profile",
        "Style",
        "Accent",
        "Pacing",
        "Scene",
        "Constraints",
    ];
    /// Identity, not voice direction: never reaches the model.
    const DROPPED_LABELS: [&str; 1] = ["Persona"];

    let mut parts: Vec<String> = Vec::new();

    for line in instructions.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let label = line.split_once(':').map(|(head, _)| head.trim());
        if label.is_some_and(|l| DROPPED_LABELS.iter().any(|d| l.eq_ignore_ascii_case(d))) {
            continue;
        }

        // Keep the value only when the line is one of the known persona
        // fields; anything else is already free-form description.
        let value = match (label, line.split_once(':')) {
            (Some(l), Some((_, tail)))
                if VALUE_LABELS.iter().any(|k| l.eq_ignore_ascii_case(k)) =>
            {
                tail
            },
            _ => line,
        };

        let cleaned = normalize_clause(value);
        if !cleaned.is_empty() {
            parts.push(cleaned);
        }
    }

    parts.join(", ")
}

/// Collapse whitespace and drop parentheses from one description fragment.
fn normalize_clause(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_space = false;

    for ch in value.chars() {
        match ch {
            '(' | ')' => {},
            c if c.is_whitespace() => {
                if !out.is_empty() {
                    pending_space = true;
                }
            },
            c => {
                if pending_space {
                    out.push(' ');
                    pending_space = false;
                }
                out.push(c);
            },
        }
    }

    while out.ends_with(',') || out.ends_with('.') {
        out.pop();
    }
    out
}

/// Prefix `text` with a VoxCPM voice-design clause built from `instructions`.
///
/// Returns the text unchanged when voice design is disabled, when there are no
/// instructions, or when the text already carries its own `(...)` prefix.
fn apply_voice_design<'a>(
    text: &'a str,
    instructions: Option<&str>,
    enabled: bool,
) -> Cow<'a, str> {
    if !enabled {
        return Cow::Borrowed(text);
    }
    let Some(instructions) = instructions else {
        return Cow::Borrowed(text);
    };
    if text.trim_start().starts_with('(') {
        // Caller (or a `[[tts:...]]` directive) already supplied a design prefix.
        return Cow::Borrowed(text);
    }
    let design = flatten_voice_design(instructions);
    if design.is_empty() {
        return Cow::Borrowed(text);
    }
    Cow::Owned(format!("({design}){text}"))
}

#[async_trait]
impl TtsProvider for VoxCpmTts {
    fn id(&self) -> &'static str {
        "voxcpm"
    }

    fn name(&self) -> &'static str {
        "VoxCPM"
    }

    fn is_configured(&self) -> bool {
        // Nothing here needs configuring: VoxCPM takes no credential, and
        // vLLM-Omni serves a single speech model whose name it reports itself,
        // so `model` is optional. The only real gate is whether the server is
        // up, which the gateway probes separately via `check_voxcpm_server`.
        // Requiring a non-default endpoint (the Coqui pattern) would reject the
        // documented default deployment and fail with "provider not
        // configured" for anyone who followed the setup docs verbatim.
        true
    }

    async fn voices(&self) -> Result<Vec<Voice>> {
        let url = format!("{}/audio/voices", self.endpoint);
        let resp = self
            .client
            .get(&url)
            .timeout(VOICES_TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                anyhow!(
                    "failed to reach VoxCPM server at {}: {e}. Start it with: vllm serve openbmb/VoxCPM2 --omni --port 8000",
                    self.endpoint
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("VoxCPM voices request failed: {status} - {body}"));
        }

        let listing: VoicesResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("failed to parse VoxCPM voices response: {e}"))?;

        Ok(listing.into_voices())
    }

    async fn synthesize(&self, request: SynthesizeRequest) -> Result<AudioOutput> {
        let (wire_format, actual_format) = Self::negotiate_format(request.output_format);
        let input = apply_voice_design(
            &request.text,
            request.instructions.as_deref(),
            self.voice_design,
        );

        // `voice` is deliberately not defaulted: VoxCPM validates the name
        // against its registered speakers and rejects unknown ones, while
        // omitting it selects zero-shot synthesis.
        let voice = request.voice_id.as_deref().or(self.voice.as_deref());

        let body = SpeechRequest {
            model: request.model.as_deref().or(self.model.as_deref()),
            input: input.as_ref(),
            voice,
            response_format: wire_format,
            speed: request.speed,
        };

        let url = format!("{}/audio/speech", self.endpoint);
        let resp = self
            .client
            .post(&url)
            .timeout(SYNTHESIZE_TIMEOUT)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                anyhow!(
                    "failed to reach VoxCPM server at {}: {e}. Start it with: vllm serve openbmb/VoxCPM2 --omni --port 8000",
                    self.endpoint
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("VoxCPM synthesis failed: {status} - {body}"));
        }

        let data = resp.bytes().await?;
        if data.is_empty() {
            return Err(anyhow!("VoxCPM returned an empty audio response"));
        }

        Ok(AudioOutput {
            data: Bytes::from(data.to_vec()),
            format: actual_format,
            duration_ms: None,
        })
    }
}

// ── Wire types ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct SpeechRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    input: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice: Option<&'a str>,
    response_format: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
struct VoicesResponse {
    #[serde(default)]
    voices: Vec<String>,
    #[serde(default)]
    uploaded_voices: Vec<UploadedVoice>,
}

#[derive(Debug, Deserialize)]
struct UploadedVoice {
    name: String,
    #[serde(default)]
    speaker_description: Option<String>,
    #[serde(default)]
    ref_text: Option<String>,
}

impl VoicesResponse {
    /// Merge the preset and uploaded listings, preferring uploaded metadata.
    fn into_voices(self) -> Vec<Voice> {
        let described: std::collections::HashMap<String, UploadedVoice> = self
            .uploaded_voices
            .into_iter()
            .map(|v| (v.name.clone(), v))
            .collect();

        self.voices
            .into_iter()
            .map(|id| {
                let description = described.get(&id).and_then(|u| {
                    u.speaker_description
                        .clone()
                        .or_else(|| u.ref_text.as_ref().map(|t| format!("Cloned from: {t}")))
                });
                Voice {
                    name: id.clone(),
                    id,
                    description,
                    preview_url: None,
                }
            })
            .collect()
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn provider(config: VoxCpmTtsConfig) -> VoxCpmTts {
        VoxCpmTts::new(&config)
    }

    #[test]
    fn metadata_is_stable() {
        let tts = provider(VoxCpmTtsConfig::default());
        assert_eq!(tts.id(), "voxcpm");
        assert_eq!(tts.name(), "VoxCPM");
        assert!(!tts.supports_ssml());
    }

    #[test]
    fn default_deployment_is_configured() {
        // The documented setup is `vllm serve ... --port 8000` with no extra
        // config, which leaves endpoint at the default and model unset. That
        // must still count as configured or `tts.enable` refuses the provider.
        let tts = provider(VoxCpmTtsConfig::default());
        assert!(tts.is_configured());
    }

    #[test]
    fn explicit_model_and_custom_endpoint_stay_configured() {
        let tts = provider(VoxCpmTtsConfig {
            endpoint: "http://10.0.0.5:8000/v1".into(),
            model: Some("openbmb/VoxCPM2".into()),
            ..Default::default()
        });
        assert!(tts.is_configured());
    }

    #[test]
    fn endpoint_trailing_slash_is_trimmed() {
        let tts = provider(VoxCpmTtsConfig {
            endpoint: "http://localhost:8000/v1/".into(),
            ..Default::default()
        });
        // Request URLs are built as `{endpoint}/audio/speech`, so a trailing
        // slash would produce a double slash in every path.
        assert_eq!(tts.endpoint, DEFAULT_ENDPOINT);
    }

    #[test]
    fn blank_endpoint_falls_back_to_default() {
        let tts = provider(VoxCpmTtsConfig {
            endpoint: "   ".into(),
            ..Default::default()
        });
        assert_eq!(tts.endpoint, DEFAULT_ENDPOINT);
    }

    #[test]
    fn blank_voice_and_model_are_treated_as_unset() {
        let tts = provider(VoxCpmTtsConfig {
            model: Some("  ".into()),
            voice: Some(String::new()),
            ..Default::default()
        });
        assert!(tts.model.is_none());
        assert!(tts.voice.is_none());
    }

    #[test]
    fn unsupported_formats_report_what_was_actually_returned() {
        // AAC and WebM are not served by vLLM-Omni; the reported format must
        // match the bytes we asked for, not the bytes we wanted.
        assert_eq!(
            VoxCpmTts::negotiate_format(AudioFormat::Aac),
            ("mp3", AudioFormat::Mp3)
        );
        assert_eq!(
            VoxCpmTts::negotiate_format(AudioFormat::Webm),
            ("opus", AudioFormat::Opus)
        );
        assert_eq!(
            VoxCpmTts::negotiate_format(AudioFormat::Wav),
            ("wav", AudioFormat::Wav)
        );
        assert_eq!(
            VoxCpmTts::negotiate_format(AudioFormat::Pcm),
            ("pcm", AudioFormat::Pcm)
        );
    }

    #[test]
    fn persona_block_keeps_values_and_drops_field_labels() {
        // Measured against a real VoxCPM2 server: a surviving `Key:` label
        // collapses the voice-design effect back to the no-prefix baseline,
        // so only the values may reach the model.
        let rendered = "Persona: Alfred\nProfile: A wise British butler\nStyle: Dry wit";
        assert_eq!(
            flatten_voice_design(rendered),
            "A wise British butler, Dry wit"
        );
    }

    #[test]
    fn persona_name_never_reaches_the_model() {
        // `Persona: <label>` is an internal identifier, not voice direction.
        assert_eq!(flatten_voice_design("Persona: Alfred"), "");
        assert!(!flatten_voice_design("Persona: Alfred\nStyle: Dry wit").contains("Alfred"));
    }

    #[test]
    fn all_rendered_persona_fields_are_unlabelled() {
        let rendered =
            "Persona: N\nProfile: p\nStyle: s\nAccent: a\nPacing: c\nScene: e\nConstraints: x. y";
        let out = flatten_voice_design(rendered);
        assert_eq!(out, "p, s, a, c, e, x. y");
        assert!(!out.contains(':'), "no field label may survive: {out}");
    }

    #[test]
    fn free_form_instructions_pass_through() {
        // Instructions that are already a plain description keep their words.
        assert_eq!(
            flatten_voice_design("A young woman, gentle and sweet voice"),
            "A young woman, gentle and sweet voice"
        );
    }

    #[test]
    fn design_clause_drops_inner_parentheses() {
        // Unbalanced parentheses would terminate VoxCPM's prefix early.
        assert_eq!(
            flatten_voice_design("Profile: A butler (retired)"),
            "A butler retired"
        );
    }

    #[test]
    fn voice_design_prefixes_the_text() {
        let out = apply_voice_design("Good evening.", Some("Style: Dry wit"), true);
        assert_eq!(out, "(Dry wit)Good evening.");
    }

    #[test]
    fn voice_design_can_be_disabled() {
        let out = apply_voice_design("Good evening.", Some("Style: Dry wit"), false);
        assert_eq!(out, "Good evening.");
    }

    #[test]
    fn voice_design_is_skipped_without_instructions() {
        assert_eq!(
            apply_voice_design("Good evening.", None, true),
            "Good evening."
        );
    }

    #[test]
    fn existing_prefix_is_not_double_wrapped() {
        let out = apply_voice_design("(A calm narrator)Hello.", Some("Style: Dry wit"), true);
        assert_eq!(out, "(A calm narrator)Hello.");
    }

    #[test]
    fn whitespace_only_instructions_do_not_prefix() {
        assert_eq!(apply_voice_design("Hello.", Some("  \n "), true), "Hello.");
    }

    #[test]
    fn a_persona_with_only_a_name_does_not_prefix() {
        // Dropping `Persona:` can empty the clause; no empty `()` may be sent.
        assert_eq!(
            apply_voice_design("Hello.", Some("Persona: Alfred"), true),
            "Hello."
        );
    }

    #[test]
    fn voices_response_merges_uploaded_metadata() {
        let raw = r#"{
            "voices": ["alice", "preset_one"],
            "uploaded_voices": [
                {"name": "alice", "speaker_description": "warm narrator"}
            ]
        }"#;
        let parsed: VoicesResponse =
            serde_json::from_str(raw).expect("voices payload should deserialize");
        let voices = parsed.into_voices();

        assert_eq!(voices.len(), 2);
        assert_eq!(voices[0].id, "alice");
        assert_eq!(voices[0].description.as_deref(), Some("warm narrator"));
        assert_eq!(voices[1].id, "preset_one");
        assert!(voices[1].description.is_none());
    }

    #[test]
    fn voices_response_falls_back_to_reference_transcript() {
        let raw = r#"{
            "voices": ["bob"],
            "uploaded_voices": [{"name": "bob", "ref_text": "hello there"}]
        }"#;
        let parsed: VoicesResponse =
            serde_json::from_str(raw).expect("voices payload should deserialize");
        let voices = parsed.into_voices();
        assert_eq!(
            voices[0].description.as_deref(),
            Some("Cloned from: hello there")
        );
    }

    #[test]
    fn voices_response_tolerates_missing_uploaded_list() {
        let parsed: VoicesResponse = serde_json::from_str(r#"{"voices":["x"]}"#)
            .expect("uploaded_voices should be optional");
        assert_eq!(parsed.into_voices().len(), 1);
    }

    #[test]
    fn request_omits_voice_and_model_when_unset() {
        let body = SpeechRequest {
            model: None,
            input: "hi",
            voice: None,
            response_format: "wav",
            speed: None,
        };
        let json = serde_json::to_string(&body).expect("request should serialize");
        // Omitting `voice` is what selects zero-shot synthesis; sending a
        // placeholder like "alloy" would be rejected by VoxCPM.
        assert_eq!(json, r#"{"input":"hi","response_format":"wav"}"#);
    }
}
