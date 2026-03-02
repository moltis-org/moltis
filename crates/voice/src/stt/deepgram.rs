//! Deepgram STT provider implementation.
//!
//! Deepgram provides fast and accurate speech-to-text with various
//! models optimized for different use cases (Nova-3 being the latest).

use {
    anyhow::{Context, Result, anyhow},
    async_trait::async_trait,
    reqwest::{Client, Url},
    secrecy::{ExposeSecret, Secret},
    serde::Deserialize,
};

use {
    super::{SttProvider, TranscribeRequest, Transcript, Word},
    crate::tts::AudioFormat,
};

/// Deepgram API base URL.
const API_BASE: &str = "https://api.deepgram.com/v1/listen";

/// Default Deepgram model.
const DEFAULT_MODEL: &str = "nova-3";

/// Deepgram STT provider.
#[derive(Clone)]
pub struct DeepgramStt {
    client: Client,
    api_key: Option<Secret<String>>,
    model: String,
    language: Option<String>,
    smart_format: bool,
}

impl std::fmt::Debug for DeepgramStt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeepgramStt")
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("language", &self.language)
            .field("smart_format", &self.smart_format)
            .finish()
    }
}

impl Default for DeepgramStt {
    fn default() -> Self {
        Self::new(None)
    }
}

impl DeepgramStt {
    /// Create a new Deepgram STT provider.
    #[must_use]
    pub fn new(api_key: Option<Secret<String>>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: DEFAULT_MODEL.into(),
            language: None,
            smart_format: true,
        }
    }

    /// Create with custom options.
    #[must_use]
    pub fn with_options(
        api_key: Option<Secret<String>>,
        model: Option<String>,
        language: Option<String>,
        smart_format: bool,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            language,
            smart_format,
        }
    }

    /// Get the API key, returning an error if not configured.
    fn get_api_key(&self) -> Result<&Secret<String>> {
        self.api_key
            .as_ref()
            .ok_or_else(|| anyhow!("Deepgram API key not configured"))
    }

    /// Get MIME type for audio format.
    fn content_type(format: AudioFormat) -> &'static str {
        format.mime_type()
    }
}

#[async_trait]
impl SttProvider for DeepgramStt {
    fn id(&self) -> &'static str {
        "deepgram"
    }

    fn name(&self) -> &'static str {
        "Deepgram"
    }

    fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
        let api_key = self.get_api_key()?;

        // Build query parameters
        let mut url = Url::parse(API_BASE).context("invalid API base URL")?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("model", &self.model);
            query.append_pair("punctuate", "true");

            if self.smart_format {
                query.append_pair("smart_format", "true");
            }

            // Use request language if provided, otherwise fall back to configured language
            if let Some(language) = request.language.as_ref().or(self.language.as_ref()) {
                query.append_pair("language", language);
            }
        }

        let response = self
            .client
            .post(url)
            .header(
                "Authorization",
                format!("Token {}", api_key.expose_secret()),
            )
            .header("Content-Type", Self::content_type(request.format))
            .body(request.audio.to_vec())
            .send()
            .await
            .context("failed to send Deepgram transcription request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Deepgram transcription request failed: {} - {}",
                status,
                body
            ));
        }

        let dg_response: DeepgramResponse = response
            .json()
            .await
            .context("failed to parse Deepgram response")?;

        // Extract language and duration before consuming channels
        let language = dg_response
            .results
            .channels
            .first()
            .and_then(|ch| ch.detected_language.clone());
        let duration_seconds = dg_response.metadata.as_ref().and_then(|m| m.duration);

        // Extract the first result from the first channel
        let result = dg_response
            .results
            .channels
            .into_iter()
            .next()
            .and_then(|ch| ch.alternatives.into_iter().next());

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
            duration_seconds,
            words: alt.words.map(|words| {
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
}

// ── API Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DeepgramResponse {
    #[serde(default)]
    metadata: Option<DeepgramMetadata>,
    results: DeepgramResults,
}

#[derive(Debug, Deserialize)]
struct DeepgramMetadata {
    #[serde(default)]
    duration: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct DeepgramResults {
    channels: Vec<DeepgramChannel>,
}

#[derive(Debug, Deserialize)]
struct DeepgramChannel {
    alternatives: Vec<DeepgramAlternative>,
    #[serde(default)]
    detected_language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeepgramAlternative {
    transcript: String,
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    words: Option<Vec<DeepgramWord>>,
}

#[derive(Debug, Deserialize)]
struct DeepgramWord {
    word: String,
    start: f32,
    end: f32,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, bytes::Bytes};

    #[test]
    fn test_provider_metadata() {
        let provider = DeepgramStt::new(None);
        assert_eq!(provider.id(), "deepgram");
        assert_eq!(provider.name(), "Deepgram");
        assert!(!provider.is_configured());

        let configured = DeepgramStt::new(Some(Secret::new("test-key".into())));
        assert!(configured.is_configured());
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let provider = DeepgramStt::new(Some(Secret::new("super-secret-key".into())));
        let debug_output = format!("{:?}", provider);
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[test]
    fn test_with_options() {
        let provider = DeepgramStt::with_options(
            Some(Secret::new("key".into())),
            Some("nova-2".into()),
            Some("en-US".into()),
            false,
        );
        assert_eq!(provider.model, "nova-2");
        assert_eq!(provider.language, Some("en-US".into()));
        assert!(!provider.smart_format);
    }

    #[tokio::test]
    async fn test_transcribe_without_api_key() {
        let provider = DeepgramStt::new(None);
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
    fn test_deepgram_response_parsing() {
        let json = r#"{
            "metadata": {
                "duration": 2.5
            },
            "results": {
                "channels": [{
                    "detected_language": "en",
                    "alternatives": [{
                        "transcript": "Hello, how are you?",
                        "confidence": 0.95,
                        "words": [
                            {"word": "Hello", "start": 0.0, "end": 0.5},
                            {"word": "how", "start": 0.6, "end": 0.8}
                        ]
                    }]
                }]
            }
        }"#;

        let response: DeepgramResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.metadata.unwrap().duration, Some(2.5));
        assert_eq!(response.results.channels.len(), 1);
        let alt = &response.results.channels[0].alternatives[0];
        assert_eq!(alt.transcript, "Hello, how are you?");
        assert_eq!(alt.confidence, Some(0.95));
        assert_eq!(alt.words.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_deepgram_response_minimal() {
        let json = r#"{
            "results": {
                "channels": [{
                    "alternatives": [{
                        "transcript": "Hello"
                    }]
                }]
            }
        }"#;
        let response: DeepgramResponse = serde_json::from_str(json).unwrap();
        let alt = &response.results.channels[0].alternatives[0];
        assert_eq!(alt.transcript, "Hello");
        assert!(alt.confidence.is_none());
        assert!(alt.words.is_none());
    }
}
