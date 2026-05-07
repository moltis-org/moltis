//! Shared OpenAI-compatible transcription logic.
//!
//! Used by providers that speak the `/v1/audio/transcriptions` protocol:
//! `voxtral-local`, `whisper-local`, and any future OpenAI-compat STT server.

use {
    anyhow::{Context, Result, anyhow},
    reqwest::{
        Client,
        multipart::{Form, Part},
    },
    serde::Deserialize,
};

use {
    super::{Transcript, Word},
    crate::tts::AudioFormat,
};

/// Send an OpenAI-compatible `/v1/audio/transcriptions` multipart request
/// and return a [`Transcript`].
pub async fn transcribe_openai_compat(
    client: &Client,
    endpoint: &str,
    audio: &[u8],
    format: AudioFormat,
    model: Option<&str>,
    language: Option<&str>,
    server_label: &str,
) -> Result<Transcript> {
    let filename = format!("audio.{}", format.extension());
    let mime_type = format.mime_type();

    let file_part = Part::bytes(audio.to_vec())
        .file_name(filename)
        .mime_str(mime_type)
        .context("failed to create file part")?;

    let mut form = Form::new()
        .part("file", file_part)
        .text("response_format", "verbose_json");

    if let Some(model) = model {
        form = form.text("model", model.to_owned());
    }

    if let Some(language) = language {
        form = form.text("language", language.to_owned());
    }

    let url = format!("{}/v1/audio/transcriptions", endpoint.trim_end_matches('/'));
    let response = client
        .post(&url)
        .multipart(form)
        .send()
        .await
        .with_context(|| format!("failed to send request to {server_label} server"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "{server_label} transcription request failed: {status} - {body}",
        ));
    }

    let compat_response: OpenAiCompatResponse = response
        .json()
        .await
        .with_context(|| format!("failed to parse {server_label} response"))?;

    Ok(Transcript {
        text: compat_response.text,
        language: compat_response.language,
        confidence: None,
        duration_seconds: compat_response.duration,
        words: compat_response.words.map(|words| {
            words
                .into_iter()
                .map(|w| Word {
                    word: w.word,
                    start: w.start,
                    end: w.end,
                })
                .collect()
        }),
    })
}

/// Check if an OpenAI-compatible server is reachable via its `/health` endpoint.
pub async fn check_server_health(client: &Client, endpoint: &str) -> bool {
    let health_url = format!("{}/health", endpoint.trim_end_matches('/'));
    client
        .get(&health_url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

// ── API Types (OpenAI-compatible verbose_json response) ─────────────────────

#[derive(Debug, Deserialize)]
struct OpenAiCompatResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f32>,
    #[serde(default)]
    words: Option<Vec<OpenAiCompatWord>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCompatWord {
    word: String,
    start: f32,
    end: f32,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_parsing_full() {
        let json = r#"{
            "text": "Hello, how are you?",
            "language": "en",
            "duration": 2.5,
            "words": [
                {"word": "Hello", "start": 0.0, "end": 0.5},
                {"word": "how", "start": 0.6, "end": 0.8}
            ]
        }"#;

        let response: OpenAiCompatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello, how are you?");
        assert_eq!(response.language, Some("en".into()));
        assert_eq!(response.duration, Some(2.5));
        assert_eq!(response.words.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_response_parsing_minimal() {
        let json = r#"{"text": "Hello"}"#;
        let response: OpenAiCompatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello");
        assert!(response.language.is_none());
        assert!(response.words.is_none());
    }
}
