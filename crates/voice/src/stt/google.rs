//! Google Cloud Speech-to-Text provider implementation.
//!
//! Google Cloud provides speech recognition with support for over 125 languages
//! and variants. This implementation uses the REST API with API key authentication.

use {
    anyhow::{Context, Result, anyhow},
    async_trait::async_trait,
    base64::Engine,
    reqwest::Client,
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

use super::{SttProvider, TranscribeRequest, Transcript, Word};

/// Google Cloud Speech-to-Text API base URL.
const API_BASE: &str = "https://speech.googleapis.com/v1/speech:recognize";

/// Google Cloud STT provider.
#[derive(Clone)]
pub struct GoogleStt {
    client: Client,
    api_key: Option<Secret<String>>,
    language: Option<String>,
    model: Option<String>,
}

impl std::fmt::Debug for GoogleStt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleStt")
            .field("api_key", &"[REDACTED]")
            .field("language", &self.language)
            .field("model", &self.model)
            .finish()
    }
}

impl Default for GoogleStt {
    fn default() -> Self {
        Self::new(None)
    }
}

impl GoogleStt {
    /// Create a new Google STT provider.
    #[must_use]
    pub fn new(api_key: Option<Secret<String>>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            language: None,
            model: None,
        }
    }

    /// Create with custom options.
    #[must_use]
    pub fn with_options(
        api_key: Option<Secret<String>>,
        language: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key,
            language,
            model,
        }
    }

    /// Get the API key, returning an error if not configured.
    fn get_api_key(&self) -> Result<&Secret<String>> {
        self.api_key
            .as_ref()
            .ok_or_else(|| anyhow!("Google Cloud API key not configured"))
    }

    /// Map our AudioFormat to Google's encoding enum.
    fn encoding(format: crate::tts::AudioFormat) -> &'static str {
        match format {
            crate::tts::AudioFormat::Mp3 => "MP3",
            crate::tts::AudioFormat::Opus | crate::tts::AudioFormat::Webm => "OGG_OPUS",
            crate::tts::AudioFormat::Aac => "MP3", // Fallback, not directly supported
            crate::tts::AudioFormat::Pcm => "LINEAR16",
        }
    }
}

#[async_trait]
impl SttProvider for GoogleStt {
    fn id(&self) -> &'static str {
        "google"
    }

    fn name(&self) -> &'static str {
        "Google Cloud"
    }

    fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
        let api_key = self.get_api_key()?;

        // Build the URL with API key
        let url = format!("{}?key={}", API_BASE, api_key.expose_secret());

        // Encode audio as base64
        let audio_content = base64::engine::general_purpose::STANDARD.encode(&request.audio);

        // Determine language code
        let language_code = request
            .language
            .clone()
            .or_else(|| self.language.clone())
            .unwrap_or_else(|| "en-US".to_string());

        // Build the request body
        let body = GoogleRequest {
            config: GoogleRecognitionConfig {
                encoding: Self::encoding(request.format).to_string(),
                language_code,
                enable_automatic_punctuation: true,
                enable_word_time_offsets: true,
                model: self.model.clone(),
            },
            audio: GoogleAudio {
                content: audio_content,
            },
        };

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("failed to send Google transcription request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Google transcription request failed: {} - {}",
                status,
                body
            ));
        }

        let google_response: GoogleResponse = response
            .json()
            .await
            .context("failed to parse Google response")?;

        // Extract the language from first result before consuming
        let language = google_response
            .results
            .first()
            .and_then(|r| r.language_code.clone());

        // Extract the first result
        let result = google_response
            .results
            .into_iter()
            .next()
            .and_then(|r| r.alternatives.into_iter().next());

        let Some(alt) = result else {
            return Ok(Transcript {
                text: String::new(),
                language: None,
                confidence: None,
                duration_seconds: None,
                words: None,
            });
        };

        Ok(Transcript {
            text: alt.transcript,
            language,
            confidence: alt.confidence,
            duration_seconds: None, // Google doesn't return total duration
            words: alt.words.map(|words| {
                words
                    .into_iter()
                    .map(|w| {
                        // Google returns times as strings like "1.500s"
                        let start = parse_google_time(&w.start_time);
                        let end = parse_google_time(&w.end_time);
                        Word {
                            word: w.word,
                            start,
                            end,
                        }
                    })
                    .collect()
            }),
        })
    }
}

/// Parse Google's time format (e.g., "1.500s") to seconds as f32.
fn parse_google_time(time_str: &str) -> f32 {
    time_str
        .strip_suffix('s')
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0)
}

// ── API Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct GoogleRequest {
    config: GoogleRecognitionConfig,
    audio: GoogleAudio,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GoogleRecognitionConfig {
    encoding: String,
    language_code: String,
    enable_automatic_punctuation: bool,
    enable_word_time_offsets: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Debug, Serialize)]
struct GoogleAudio {
    content: String,
}

#[derive(Debug, Deserialize, Default)]
struct GoogleResponse {
    #[serde(default)]
    results: Vec<GoogleResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleResult {
    alternatives: Vec<GoogleAlternative>,
    #[serde(default)]
    language_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleAlternative {
    transcript: String,
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    words: Option<Vec<GoogleWord>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleWord {
    word: String,
    #[serde(default)]
    start_time: String,
    #[serde(default)]
    end_time: String,
}

#[cfg(test)]
mod tests {
    use {super::*, crate::tts::AudioFormat, bytes::Bytes};

    #[test]
    fn test_provider_metadata() {
        let provider = GoogleStt::new(None);
        assert_eq!(provider.id(), "google");
        assert_eq!(provider.name(), "Google Cloud");
        assert!(!provider.is_configured());

        let configured = GoogleStt::new(Some(Secret::new("test-key".into())));
        assert!(configured.is_configured());
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let provider = GoogleStt::new(Some(Secret::new("super-secret-key".into())));
        let debug_output = format!("{:?}", provider);
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[test]
    fn test_with_options() {
        let provider = GoogleStt::with_options(
            Some(Secret::new("key".into())),
            Some("fr-FR".into()),
            Some("latest_long".into()),
        );
        assert_eq!(provider.language, Some("fr-FR".into()));
        assert_eq!(provider.model, Some("latest_long".into()));
    }

    #[tokio::test]
    async fn test_transcribe_without_api_key() {
        let provider = GoogleStt::new(None);
        let request = TranscribeRequest {
            audio: Bytes::from_static(b"fake audio"),
            format: AudioFormat::Mp3,
            language: None,
            prompt: None,
        };

        let result = provider.transcribe(request).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not configured"));
    }

    #[test]
    fn test_parse_google_time() {
        assert!((parse_google_time("1.500s") - 1.5).abs() < 0.001);
        assert!((parse_google_time("0s") - 0.0).abs() < 0.001);
        assert!((parse_google_time("10.123s") - 10.123).abs() < 0.001);
        assert!((parse_google_time("invalid") - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_google_response_parsing() {
        let json = r#"{
            "results": [{
                "alternatives": [{
                    "transcript": "Hello, how are you?",
                    "confidence": 0.95,
                    "words": [
                        {"word": "Hello", "startTime": "0s", "endTime": "0.5s"},
                        {"word": "how", "startTime": "0.6s", "endTime": "0.8s"}
                    ]
                }],
                "languageCode": "en-us"
            }]
        }"#;

        let response: GoogleResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.results.len(), 1);
        let alt = &response.results[0].alternatives[0];
        assert_eq!(alt.transcript, "Hello, how are you?");
        assert_eq!(alt.confidence, Some(0.95));
        assert_eq!(alt.words.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_google_response_empty() {
        let json = r#"{"results": []}"#;
        let response: GoogleResponse = serde_json::from_str(json).unwrap();
        assert!(response.results.is_empty());
    }

    #[test]
    fn test_encoding_mapping() {
        assert_eq!(GoogleStt::encoding(AudioFormat::Mp3), "MP3");
        assert_eq!(GoogleStt::encoding(AudioFormat::Opus), "OGG_OPUS");
        assert_eq!(GoogleStt::encoding(AudioFormat::Pcm), "LINEAR16");
    }
}
