//! Mock TTS provider for testing.
//!
//! Returns silence (zero-filled PCM) of configurable duration.
//! No network calls, no API keys needed.

use {async_trait::async_trait, bytes::Bytes};

use super::{AudioOutput, SynthesizeRequest, TtsProvider, Voice};

/// Mock TTS provider that returns silence.
pub struct MockTts {
    /// Duration of generated silence in milliseconds.
    duration_ms: u64,
}

impl MockTts {
    pub fn new() -> Self {
        Self { duration_ms: 500 }
    }

    pub fn with_duration_ms(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }
}

impl Default for MockTts {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TtsProvider for MockTts {
    fn id(&self) -> &'static str {
        "mock"
    }

    fn name(&self) -> &'static str {
        "Mock"
    }

    fn is_configured(&self) -> bool {
        true
    }

    async fn voices(&self) -> anyhow::Result<Vec<Voice>> {
        Ok(vec![Voice {
            id: "mock-voice".into(),
            name: "Mock Voice".into(),
            description: Some("Silent test voice".into()),
            preview_url: None,
        }])
    }

    async fn synthesize(&self, request: SynthesizeRequest) -> anyhow::Result<AudioOutput> {
        // Generate silence: 8kHz mu-law = 8 bytes per ms.
        let num_bytes = (self.duration_ms * 8) as usize;
        // mu-law silence byte is 0xFF (positive zero).
        let data = Bytes::from(vec![0xFF_u8; num_bytes]);

        Ok(AudioOutput {
            data,
            format: request.output_format,
            duration_ms: Some(self.duration_ms),
        })
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::AudioFormat};

    #[tokio::test]
    async fn mock_tts_returns_silence() {
        let tts = MockTts::new().with_duration_ms(100);
        let output = tts
            .synthesize(SynthesizeRequest {
                text: "hello".into(),
                voice_id: None,
                model: None,
                output_format: AudioFormat::Pcm,
                speed: None,
                stability: None,
                similarity_boost: None,
                instructions: None,
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        // 100ms at 8kHz = 800 bytes
        assert_eq!(output.data.len(), 800);
        assert_eq!(output.duration_ms, Some(100));
    }

    #[tokio::test]
    async fn mock_tts_is_configured() {
        assert!(MockTts::new().is_configured());
    }
}
