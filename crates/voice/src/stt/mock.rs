//! Mock STT provider for testing.
//!
//! Returns a configurable canned transcript without network calls.

use async_trait::async_trait;

use super::{SttProvider, TranscribeRequest, Transcript};

/// Mock STT provider that returns a canned transcript.
pub struct MockStt {
    /// Text to return for any transcription request.
    canned_text: String,
}

impl MockStt {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            canned_text: text.into(),
        }
    }
}

#[async_trait]
impl SttProvider for MockStt {
    fn id(&self) -> &'static str {
        "mock"
    }

    fn name(&self) -> &'static str {
        "Mock"
    }

    fn is_configured(&self) -> bool {
        true
    }

    async fn transcribe(&self, _request: TranscribeRequest) -> anyhow::Result<Transcript> {
        Ok(Transcript {
            text: self.canned_text.clone(),
            language: Some("en".into()),
            confidence: Some(1.0),
            duration_seconds: Some(1.0),
            words: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::tts::AudioFormat, bytes::Bytes};

    #[tokio::test]
    async fn mock_stt_returns_canned_text() {
        let stt = MockStt::new("hello world");
        let result = stt
            .transcribe(TranscribeRequest {
                audio: Bytes::from_static(b"fake-audio"),
                format: AudioFormat::Pcm,
                language: None,
                prompt: None,
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(result.text, "hello world");
        assert_eq!(result.confidence, Some(1.0));
    }

    #[tokio::test]
    async fn mock_stt_is_configured() {
        assert!(MockStt::new("test").is_configured());
    }
}
