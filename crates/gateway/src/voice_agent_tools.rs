use {
    anyhow::Result,
    async_trait::async_trait,
    base64::Engine,
    moltis_agents::tool_registry::AgentTool,
    serde::{Deserialize, Serialize},
    serde_json::{Value, json},
    std::{path::PathBuf, sync::Arc},
    uuid::Uuid,
};

use crate::{services::TtsService, voice::SttService};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeakToolParams {
    text: String,
    provider: Option<String>,
    format: Option<String>,
    voice_id: Option<String>,
    model: Option<String>,
    speed: Option<f64>,
    stability: Option<f64>,
    similarity_boost: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TtsConvertParams {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stability: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    similarity_boost: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TtsConvertResult {
    audio: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    duration_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpeakToolResult {
    ok: bool,
    media_path: String,
    mime_type: String,
    voice_compatible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    size: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscribeToolParams {
    audio: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscribeToolResult {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    words: Option<Value>,
}

pub struct SpeakTool {
    tts: Arc<dyn TtsService>,
}

impl SpeakTool {
    pub fn new(tts: Arc<dyn TtsService>) -> Self {
        Self { tts }
    }
}

#[async_trait]
impl AgentTool for SpeakTool {
    fn name(&self) -> &str {
        "speak"
    }

    fn description(&self) -> &str {
        "Convert text to speech. Use when the user asks for audio/voice output. Returns an audio file path and metadata."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string", "description": "Text to synthesize." },
                "provider": { "type": "string", "description": "Optional TTS provider override." },
                "format": { "type": "string", "enum": ["ogg", "opus", "mp3", "aac", "pcm"], "description": "Output format. Use ogg/opus for voice notes." },
                "voiceId": { "type": "string", "description": "Optional voice ID." },
                "model": { "type": "string", "description": "Optional provider model override." },
                "speed": { "type": "number", "description": "Optional speaking speed." },
                "stability": { "type": "number", "description": "Optional stability (provider-specific)." },
                "similarityBoost": { "type": "number", "description": "Optional similarity boost (provider-specific)." }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let input: SpeakToolParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid speak parameters: {e}"))?;

        let request = TtsConvertParams {
            text: input.text,
            provider: input.provider,
            format: input.format,
            voice_id: input.voice_id,
            model: input.model,
            speed: input.speed,
            stability: input.stability,
            similarity_boost: input.similarity_boost,
        };

        let result = self
            .tts
            .convert(serde_json::to_value(request)?)
            .await
            .map_err(anyhow::Error::msg)?;

        let tts_result: TtsConvertResult = serde_json::from_value(result)
            .map_err(|e| anyhow::anyhow!("invalid tts.convert response: {e}"))?;

        let mime_type = tts_result
            .mime_type
            .unwrap_or_else(|| "audio/ogg".to_string());
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(tts_result.audio)
            .map_err(|e| anyhow::anyhow!("invalid base64 audio from tts.convert: {e}"))?;

        let ext = match mime_type.as_str() {
            "audio/ogg" => "ogg",
            "audio/mpeg" => "mp3",
            "audio/aac" => "aac",
            "audio/pcm" => "pcm",
            _ => "bin",
        };
        let file_name = format!("speak-{}.{}", Uuid::new_v4(), ext);
        let file_path = write_tool_audio_file(&file_name, &bytes)?;

        let output = SpeakToolResult {
            ok: true,
            media_path: file_path,
            mime_type: mime_type.clone(),
            voice_compatible: mime_type == "audio/ogg",
            duration_ms: tts_result.duration_ms,
            size: bytes.len(),
        };

        Ok(serde_json::to_value(output)?)
    }
}

pub struct TranscribeTool {
    stt: Arc<dyn SttService>,
}

impl TranscribeTool {
    pub fn new(stt: Arc<dyn SttService>) -> Self {
        Self { stt }
    }
}

#[async_trait]
impl AgentTool for TranscribeTool {
    fn name(&self) -> &str {
        "transcribe"
    }

    fn description(&self) -> &str {
        "Transcribe audio to text. Use when the user provides audio content that must be turned into text."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["audio"],
            "properties": {
                "audio": { "type": "string", "description": "Base64-encoded audio bytes." },
                "format": { "type": "string", "enum": ["ogg", "opus", "mp3", "aac", "pcm", "wav", "webm"], "description": "Input audio format." },
                "provider": { "type": "string", "description": "Optional STT provider override." },
                "language": { "type": "string", "description": "Optional language hint." },
                "prompt": { "type": "string", "description": "Optional transcription prompt." }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let input: TranscribeToolParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid transcribe parameters: {e}"))?;

        let result = self
            .stt
            .transcribe(serde_json::to_value(input)?)
            .await
            .map_err(anyhow::Error::msg)?;

        let parsed: TranscribeToolResult = serde_json::from_value(result)
            .map_err(|e| anyhow::anyhow!("invalid stt.transcribe response: {e}"))?;
        Ok(serde_json::to_value(parsed)?)
    }
}

fn write_tool_audio_file(file_name: &str, bytes: &[u8]) -> Result<String> {
    let dir = moltis_config::data_dir().join("tool-audio");
    std::fs::create_dir_all(&dir)?;
    let path: PathBuf = dir.join(file_name);
    std::fs::write(&path, bytes)?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use {super::*, crate::services::ServiceResult};

    struct MockTts;

    #[async_trait]
    impl TtsService for MockTts {
        async fn status(&self) -> ServiceResult {
            Ok(json!({ "enabled": true }))
        }

        async fn providers(&self) -> ServiceResult {
            Ok(json!([]))
        }

        async fn enable(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn disable(&self) -> ServiceResult {
            Ok(json!({}))
        }

        async fn convert(&self, _params: Value) -> ServiceResult {
            Ok(json!({
                "audio": base64::engine::general_purpose::STANDARD.encode(b"fake-audio"),
                "mimeType": "audio/ogg",
                "durationMs": 123,
            }))
        }

        async fn set_provider(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }
    }

    struct MockStt;

    #[async_trait]
    impl SttService for MockStt {
        async fn status(&self) -> ServiceResult {
            Ok(json!({ "enabled": true }))
        }

        async fn providers(&self) -> ServiceResult {
            Ok(json!([]))
        }

        async fn transcribe(&self, _params: Value) -> ServiceResult {
            Ok(json!({ "text": "hello" }))
        }

        async fn transcribe_bytes(
            &self,
            _audio: bytes::Bytes,
            _format: &str,
            _provider: Option<&str>,
            _language: Option<&str>,
            _prompt: Option<&str>,
        ) -> ServiceResult {
            Err("STT not available in agent mode".to_string())
        }

        async fn set_provider(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }
    }

    #[tokio::test]
    async fn speak_tool_returns_media_path() {
        let tool = SpeakTool::new(Arc::new(MockTts));
        let out = tool
            .execute(json!({ "text": "hello world", "format": "ogg" }))
            .await
            .expect("speak execute");

        let media_path = out
            .get("mediaPath")
            .and_then(Value::as_str)
            .expect("mediaPath in output");
        assert!(std::path::Path::new(media_path).exists());
        let _ = std::fs::remove_file(media_path);
        assert_eq!(
            out.get("voiceCompatible").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn transcribe_tool_returns_text() {
        let tool = TranscribeTool::new(Arc::new(MockStt));
        let out = tool
            .execute(json!({ "audio": base64::engine::general_purpose::STANDARD.encode(b"abc") }))
            .await
            .expect("transcribe execute");
        assert_eq!(out.get("text").and_then(Value::as_str), Some("hello"));
    }
}
