#![allow(clippy::module_inception)]

use super::*;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::*,
        async_trait::async_trait,
        moltis_common::hooks::{HookAction, HookEvent, HookHandler, HookPayload},
    };

    struct RecordingHook {
        payloads: Arc<std::sync::Mutex<Vec<HookPayload>>>,
    }

    #[async_trait]
    impl HookHandler for RecordingHook {
        fn name(&self) -> &str {
            "recording-hook"
        }

        fn events(&self) -> &[HookEvent] {
            static EVENTS: [HookEvent; 1] = [HookEvent::SessionStart];
            &EVENTS
        }

        async fn handle(
            &self,
            _event: HookEvent,
            payload: &HookPayload,
        ) -> moltis_common::error::Result<HookAction> {
            self.payloads.lock().unwrap().push(payload.clone());
            Ok(HookAction::Continue)
        }
    }

    #[test]
    fn filter_ui_history_removes_empty_assistant_messages() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "hi there"}),
            serde_json::json!({"role": "user", "content": "run ls"}),
            // Empty assistant after tool use — should be filtered
            serde_json::json!({"role": "assistant", "content": ""}),
            serde_json::json!({"role": "user", "content": "run pwd"}),
            serde_json::json!({"role": "assistant", "content": "here is the output"}),
        ];
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 5);
        // The empty assistant message at index 3 should be gone.
        assert_eq!(filtered[2]["role"], "user");
        assert_eq!(filtered[2]["content"], "run ls");
        assert_eq!(filtered[3]["role"], "user");
        assert_eq!(filtered[3]["content"], "run pwd");
    }

    #[test]
    fn filter_ui_history_removes_whitespace_only_assistant() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "   \n  "}),
        ];
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0]["role"], "user");
    }

    #[test]
    fn filter_ui_history_keeps_non_empty_assistant() {
        let messages = vec![
            serde_json::json!({"role": "assistant", "content": "real response"}),
            serde_json::json!({"role": "assistant", "content": ".", "model": "gpt-4o"}),
        ];
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_ui_history_keeps_non_assistant_roles() {
        let messages = vec![
            serde_json::json!({"role": "system", "content": ""}),
            serde_json::json!({"role": "tool", "tool_call_id": "x", "content": ""}),
            serde_json::json!({"role": "user", "content": ""}),
        ];
        // Non-assistant roles pass through even if content is empty.
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn filter_ui_history_keeps_reasoning_only_assistant() {
        let messages = vec![
            serde_json::json!({"role": "assistant", "content": "", "reasoning": "internal plan"}),
        ];
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0]["role"], "assistant");
        assert_eq!(filtered[0]["reasoning"], "internal plan");
    }

    #[test]
    fn trim_ui_history_drops_oldest_messages_when_payload_is_too_large() {
        let payload = "x".repeat(30_000);
        let history: Vec<Value> = (0..150)
            .map(|idx| serde_json::json!({ "id": idx, "role": "assistant", "content": payload }))
            .collect();

        let (trimmed, dropped) = trim_ui_history(history);
        assert!(dropped > 0, "expected some messages to be dropped");
        assert_eq!(trimmed.len() + dropped, 150);
        assert_eq!(trimmed[0]["id"], serde_json::json!(dropped));
        assert!(
            trimmed.len() >= UI_HISTORY_MIN_MESSAGES,
            "must keep at least the configured recent tail",
        );

        let trimmed_bytes = serde_json::to_vec(&trimmed).expect("serialize trimmed history");
        assert!(
            trimmed_bytes.len() <= UI_HISTORY_MAX_BYTES || trimmed.len() == UI_HISTORY_MIN_MESSAGES,
            "trimmed payload should stay under budget unless minimum tail is reached",
        );
    }

    // --- Preview extraction tests ---

    #[test]
    fn message_text_from_string_content() {
        let msg = serde_json::json!({"role": "user", "content": "hello world"});
        assert_eq!(message_text(&msg), Some("hello world".to_string()));
    }

    #[test]
    fn message_text_from_content_blocks() {
        let msg = serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "image_url", "url": "http://example.com/img.png"},
                {"type": "text", "text": "world"}
            ]
        });
        assert_eq!(message_text(&msg), Some("hello world".to_string()));
    }

    #[test]
    fn message_text_empty_content() {
        let msg = serde_json::json!({"role": "user", "content": "  "});
        assert_eq!(message_text(&msg), None);
    }

    #[test]
    fn message_text_no_content_field() {
        let msg = serde_json::json!({"role": "user"});
        assert_eq!(message_text(&msg), None);
    }

    #[test]
    fn truncate_preview_short_string() {
        assert_eq!(truncate_preview("short", 200), "short");
    }

    #[test]
    fn truncate_preview_long_string() {
        let long = "a".repeat(250);
        let result = truncate_preview(&long, 200);
        assert!(result.ends_with('…'));
        // 200 'a' chars + the '…' char
        assert!(result.len() <= 204); // 200 bytes + up to 3 for '…'
    }

    #[test]
    fn extract_preview_single_short_message() {
        let history = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let result = extract_preview(&history);
        // Short message is still returned, just won't reach the 80-char target
        assert_eq!(result, Some("hi".to_string()));
    }

    #[test]
    fn extract_preview_combines_messages_until_target() {
        let history = vec![
            serde_json::json!({"role": "user", "content": "hi"}),
            serde_json::json!({"role": "assistant", "content": "Hello! How can I help you today?"}),
            serde_json::json!({"role": "user", "content": "Tell me about Rust programming language"}),
        ];
        let result = extract_preview(&history).expect("should produce preview");
        assert!(result.contains("hi"));
        assert!(result.contains(" — "));
        assert!(result.contains("Hello!"));
        // Should stop once target (80) is reached
        assert!(result.len() >= 30);
    }

    #[test]
    fn extract_preview_skips_system_and_tool_messages() {
        let history = vec![
            serde_json::json!({"role": "system", "content": "You are a helpful assistant."}),
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "tool", "content": "tool output"}),
            serde_json::json!({"role": "assistant", "content": "Hi there!"}),
        ];
        let result = extract_preview(&history).expect("should produce preview");
        // Should not contain system or tool content
        assert!(!result.contains("helpful assistant"));
        assert!(!result.contains("tool output"));
        assert!(result.contains("hello"));
        assert!(result.contains("Hi there!"));
    }

    #[test]
    fn extract_preview_empty_history() {
        let result = extract_preview(&[]);
        assert_eq!(result, None);
    }

    #[test]
    fn extract_preview_only_system_messages() {
        let history =
            vec![serde_json::json!({"role": "system", "content": "You are a helpful assistant."})];
        let result = extract_preview(&history);
        assert_eq!(result, None);
    }

    #[test]
    fn extract_preview_truncates_at_max() {
        // Build a very long message that exceeds MAX (200)
        let long_text = "a".repeat(300);
        let history = vec![serde_json::json!({"role": "user", "content": long_text})];
        let result = extract_preview(&history).expect("should produce preview");
        assert!(result.ends_with('…'));
        assert!(result.len() <= 204);
    }

    #[test]
    fn media_filename_extracts_last_segment() {
        assert_eq!(media_filename("media/main/voice.ogg"), Some("voice.ogg"));
        assert_eq!(media_filename("voice.ogg"), Some("voice.ogg"));
        assert_eq!(media_filename(""), None);
    }

    #[test]
    fn audio_mime_type_maps_known_extensions() {
        assert_eq!(audio_mime_type("voice.ogg"), "audio/ogg");
        assert_eq!(audio_mime_type("voice.webm"), "audio/webm");
        assert_eq!(audio_mime_type("voice.mp3"), "audio/mpeg");
        assert_eq!(audio_mime_type("voice.unknown"), "application/octet-stream");
    }

    #[test]
    fn image_mime_type_maps_known_extensions() {
        assert_eq!(image_mime_type("map.png"), "image/png");
        assert_eq!(image_mime_type("map.jpeg"), "image/jpeg");
        assert_eq!(image_mime_type("map.webp"), "image/webp");
        assert_eq!(image_mime_type("map.unknown"), "application/octet-stream");
    }

    #[test]
    fn sanitize_share_url_rejects_unsafe_schemes() {
        assert_eq!(
            sanitize_share_url("https://maps.apple.com/?q=test"),
            Some("https://maps.apple.com/?q=test".to_string())
        );
        assert_eq!(sanitize_share_url("javascript:alert(1)"), None);
        assert_eq!(sanitize_share_url("data:text/html,test"), None);
    }

    #[tokio::test]
    async fn message_audio_data_url_for_share_reads_media_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        let bytes = b"OggSfake".to_vec();
        store
            .save_media("main", "voice.ogg", &bytes)
            .await
            .expect("save media");

        let msg = serde_json::json!({
            "role": "assistant",
            "audio": "media/main/voice.ogg",
        });

        let data_url = message_audio_data_url_for_share(&msg, "main", &store).await;
        assert!(data_url.is_some());
        assert!(
            data_url
                .as_deref()
                .unwrap_or_default()
                .starts_with("data:audio/ogg;base64,")
        );
    }

    #[tokio::test]
    async fn to_shared_message_skips_system_and_notice_roles() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());

        let system_msg = serde_json::json!({
            "role": "system",
            "content": "system info",
        });
        let notice_msg = serde_json::json!({
            "role": "notice",
            "content": "share boundary",
        });
        let assistant_msg = serde_json::json!({
            "role": "assistant",
            "content": "hello",
        });

        assert!(
            to_shared_message(&system_msg, "main", &store)
                .await
                .is_none()
        );
        assert!(
            to_shared_message(&notice_msg, "main", &store)
                .await
                .is_none()
        );
        assert!(
            to_shared_message(&assistant_msg, "main", &store)
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn to_shared_message_includes_user_audio_without_text() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        store
            .save_media("main", "voice-input.webm", b"RIFFfake")
            .await
            .expect("save media");

        let user_audio_msg = serde_json::json!({
            "role": "user",
            "content": "",
            "audio": "media/main/voice-input.webm",
        });

        let shared = to_shared_message(&user_audio_msg, "main", &store)
            .await
            .expect("shared message");

        assert!(matches!(shared.role, SharedMessageRole::User));
        assert!(shared.content.is_empty());
        assert!(
            shared
                .audio_data_url
                .as_deref()
                .unwrap_or_default()
                .starts_with("data:audio/webm;base64,")
        );
    }

    #[tokio::test]
    async fn to_shared_message_includes_assistant_audio() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        store
            .save_media("main", "voice-output.ogg", b"OggSfake")
            .await
            .expect("save media");

        let assistant_audio_msg = serde_json::json!({
            "role": "assistant",
            "content": "Here you go",
            "audio": "media/main/voice-output.ogg",
        });

        let shared = to_shared_message(&assistant_audio_msg, "main", &store)
            .await
            .expect("shared message");

        assert!(matches!(shared.role, SharedMessageRole::Assistant));
        assert_eq!(shared.content, "Here you go");
        assert!(
            shared
                .audio_data_url
                .as_deref()
                .unwrap_or_default()
                .starts_with("data:audio/ogg;base64,")
        );
    }

    #[tokio::test]
    async fn to_shared_message_includes_assistant_reasoning_without_text() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        let assistant_msg = serde_json::json!({
            "role": "assistant",
            "content": "",
            "reasoning": "step one\nstep two",
        });

        let shared = to_shared_message(&assistant_msg, "main", &store)
            .await
            .expect("shared message");

        assert!(matches!(shared.role, SharedMessageRole::Assistant));
        assert!(shared.content.is_empty());
        assert_eq!(shared.reasoning.as_deref(), Some("step one\nstep two"));
        assert!(shared.audio_data_url.is_none());
    }

    #[tokio::test]
    async fn to_shared_message_includes_tool_result_screenshot_and_map_links() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        let tiny_png = general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+tmXcAAAAASUVORK5CYII=")
            .unwrap();
        store
            .save_media("main", "call-map.png", &tiny_png)
            .await
            .expect("save media");

        let tool_msg = serde_json::json!({
            "role": "tool_result",
            "tool_name": "show_map",
            "success": true,
            "created_at": 1_770_966_725_000_u64,
            "result": {
                "label": "Tartine Bakery",
                "screenshot": "media/main/call-map.png",
                "map_links": {
                    "google_maps": "https://www.google.com/maps/search/?api=1&query=Tartine+Bakery",
                    "apple_maps": "javascript:alert(1)",
                    "openstreetmap": "https://www.openstreetmap.org/search?query=Tartine+Bakery",
                },
            },
        });

        let shared = to_shared_message(&tool_msg, "main", &store)
            .await
            .expect("shared tool_result message");

        assert!(matches!(shared.role, SharedMessageRole::ToolResult));
        assert_eq!(shared.tool_success, Some(true));
        assert_eq!(shared.tool_name.as_deref(), Some("show_map"));
        assert!(shared.tool_command.is_none());
        assert!(shared.audio_data_url.is_none());
        assert!(shared.image_data_url.is_none());
        let image = shared.image.expect("shared image variants");
        assert!(image.preview.data_url.starts_with("data:image/png;base64,"));
        assert_eq!(image.preview.width, 1);
        assert_eq!(image.preview.height, 1);
        assert!(image.full.is_none());
        let map_links = shared.map_links.expect("map links");
        assert!(map_links.google_maps.is_some());
        assert!(map_links.openstreetmap.is_some());
        assert!(map_links.apple_maps.is_none());
        assert!(shared.content.contains("Tartine Bakery"));
    }

    #[test]
    fn tool_result_text_for_share_preserves_full_stdout() {
        let large_stdout = format!("{{\"items\":[\"{}\"]}}", "x".repeat(2_000));
        let msg = serde_json::json!({
            "role": "tool_result",
            "result": {
                "stdout": large_stdout
            }
        });

        let text = tool_result_text_for_share(&msg).expect("tool text should exist");
        assert!(text.contains("\"items\""));
        assert!(!text.contains("(truncated)"));
        assert!(!text.ends_with('…'));
        assert!(text.len() > 1_800);
    }

    #[test]
    fn redact_share_secret_values_masks_env_vars_and_api_tokens() {
        let input = "OPENAI_API_KEY=sk-openai BRAVE_API_KEY=brave-secret Authorization: Bearer bearer-secret https://api.example.com/search?q=test&api_key=url-secret";
        let redacted = redact_share_secret_values(input);

        assert!(!redacted.contains("sk-openai"));
        assert!(!redacted.contains("brave-secret"));
        assert!(!redacted.contains("bearer-secret"));
        assert!(!redacted.contains("url-secret"));
        assert!(redacted.contains("OPENAI_API_KEY=[REDACTED]"));
        assert!(redacted.contains("BRAVE_API_KEY=[REDACTED]"));
        assert!(redacted.contains("Bearer [REDACTED]"));
    }

    #[test]
    fn tool_result_text_for_share_redacts_sensitive_values() {
        let msg = serde_json::json!({
            "role": "tool_result",
            "result": {
                "stdout": "{\"apiKey\":\"llm-secret\",\"voice_api_key\":\"voice-secret\"}\nOPENAI_API_KEY=env-secret",
                "stderr": "Authorization: Bearer bearer-secret\nx-api-key: header-secret",
            }
        });

        let text = tool_result_text_for_share(&msg).unwrap_or_default();
        assert!(!text.contains("llm-secret"));
        assert!(!text.contains("voice-secret"));
        assert!(!text.contains("env-secret"));
        assert!(!text.contains("bearer-secret"));
        assert!(!text.contains("header-secret"));
        assert!(text.contains(SHARE_REDACTED_VALUE));
    }

    #[tokio::test]
    async fn to_shared_message_includes_exec_command_for_tool_result() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        let tool_msg = serde_json::json!({
            "role": "tool_result",
            "tool_name": "exec",
            "arguments": {
                "command": "curl -s https://example.com"
            },
            "success": true,
            "result": {
                "stdout": "{\"ok\":true}",
                "stderr": "",
                "exit_code": 0,
            },
        });

        let shared = to_shared_message(&tool_msg, "main", &store)
            .await
            .expect("shared exec tool result");
        assert_eq!(shared.tool_name.as_deref(), Some("exec"));
        assert_eq!(
            shared.tool_command.as_deref(),
            Some("curl -s https://example.com")
        );
        assert!(shared.content.contains("{\"ok\":true}"));
    }

    #[tokio::test]
    async fn to_shared_message_redacts_exec_command_and_output_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        let tool_msg = serde_json::json!({
            "role": "tool_result",
            "tool_name": "exec",
            "arguments": {
                "command": "OPENAI_API_KEY=sk-openai curl -s -H 'Authorization: Bearer bearer-secret' 'https://api.example.com?q=test&api_key=url-secret'"
            },
            "success": true,
            "result": {
                "stdout": "{\"api_key\":\"stdout-secret\"}",
                "stderr": "ELEVENLABS_API_KEY=voice-secret",
                "exit_code": 0,
            },
        });

        let shared = to_shared_message(&tool_msg, "main", &store)
            .await
            .expect("shared exec tool result");

        assert_eq!(shared.tool_name.as_deref(), Some("exec"));
        let command = shared.tool_command.unwrap_or_default();
        assert!(!command.contains("sk-openai"));
        assert!(!command.contains("bearer-secret"));
        assert!(!command.contains("url-secret"));
        assert!(command.contains(SHARE_REDACTED_VALUE));

        assert!(!shared.content.contains("stdout-secret"));
        assert!(!shared.content.contains("voice-secret"));
        assert!(shared.content.contains(SHARE_REDACTED_VALUE));
    }

    struct MockTtsService {
        status_payload: Value,
        convert_payload: Option<Value>,
        convert_error: Option<String>,
        convert_calls: AtomicU32,
    }

    impl MockTtsService {
        fn new(status_payload: Value, convert_payload: Option<Value>) -> Self {
            Self {
                status_payload,
                convert_payload,
                convert_error: None,
                convert_calls: AtomicU32::new(0),
            }
        }

        fn with_convert_error(status_payload: Value, error: &str) -> Self {
            Self {
                status_payload,
                convert_payload: None,
                convert_error: Some(error.to_string()),
                convert_calls: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl TtsService for MockTtsService {
        async fn status(&self) -> ServiceResult {
            Ok(self.status_payload.clone())
        }

        async fn providers(&self) -> ServiceResult {
            Ok(serde_json::json!([]))
        }

        async fn enable(&self, _params: Value) -> ServiceResult {
            Err("mock".into())
        }

        async fn disable(&self) -> ServiceResult {
            Ok(serde_json::json!({}))
        }

        async fn convert(&self, _params: Value) -> ServiceResult {
            self.convert_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(ref error) = self.convert_error {
                return Err(error.clone().into());
            }
            self.convert_payload
                .clone()
                .ok_or_else(|| ServiceError::message("mock missing convert payload"))
        }

        async fn set_provider(&self, _params: Value) -> ServiceResult {
            Err("mock".into())
        }
    }

    #[tokio::test]
    async fn voice_generate_reuses_existing_audio_without_tts_convert() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        let existing_path = store
            .save_media("main", "voice-msg-1.ogg", b"OggSreuse")
            .await
            .expect("save media");

        store
            .append(
                "main",
                &serde_json::json!({ "role": "user", "content": "hello" }),
            )
            .await
            .expect("append user");
        store
            .append(
                "main",
                &serde_json::json!({
                    "role": "assistant",
                    "content": "hi there",
                    "audio": existing_path,
                    "run_id": "run-abc",
                }),
            )
            .await
            .expect("append assistant");

        let mock_tts = Arc::new(MockTtsService::with_convert_error(
            serde_json::json!({ "enabled": true, "maxTextLength": 8000 }),
            "convert should not be called",
        ));
        let service = LiveSessionService::new(Arc::clone(&store), metadata)
            .with_tts_service(Arc::clone(&mock_tts) as Arc<dyn TtsService>);

        let result = service
            .voice_generate(serde_json::json!({ "key": "main", "messageIndex": 1 }))
            .await
            .expect("voice generate");

        assert_eq!(result["reused"], true);
        assert_eq!(result["audio"].as_str(), Some("media/main/voice-msg-1.ogg"));
        assert_eq!(mock_tts.convert_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn voice_generate_creates_and_persists_audio() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        store
            .append(
                "main",
                &serde_json::json!({ "role": "user", "content": "hello" }),
            )
            .await
            .expect("append user");
        store
            .append(
                "main",
                &serde_json::json!({
                    "role": "assistant",
                    "content": "here is the reply",
                    "run_id": "run-generate",
                }),
            )
            .await
            .expect("append assistant");

        let audio_bytes = b"OggSnew".to_vec();
        let mock_tts = Arc::new(MockTtsService::new(
            serde_json::json!({ "enabled": true, "maxTextLength": 8000 }),
            Some(serde_json::json!({
                "audio": general_purpose::STANDARD.encode(&audio_bytes),
            })),
        ));
        let service = LiveSessionService::new(Arc::clone(&store), metadata)
            .with_tts_service(Arc::clone(&mock_tts) as Arc<dyn TtsService>);

        let result = service
            .voice_generate(serde_json::json!({ "key": "main", "runId": "run-generate" }))
            .await
            .expect("voice generate");

        assert_eq!(result["reused"], false);
        let audio_path = result["audio"].as_str().unwrap_or_default().to_string();
        assert_eq!(audio_path, "media/main/voice-msg-1.ogg");
        assert_eq!(mock_tts.convert_calls.load(Ordering::SeqCst), 1);

        let history = store.read("main").await.expect("read history");
        assert_eq!(history[1]["audio"].as_str(), Some(audio_path.as_str()));

        let filename = media_filename(&audio_path).expect("filename");
        let saved = store
            .read_media("main", filename)
            .await
            .expect("read media");
        assert_eq!(saved, audio_bytes);
    }

    #[tokio::test]
    async fn voice_generate_rejects_non_assistant_target() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        store
            .append(
                "main",
                &serde_json::json!({ "role": "user", "content": "hello" }),
            )
            .await
            .expect("append user");

        let mock_tts = Arc::new(MockTtsService::new(
            serde_json::json!({ "enabled": true, "maxTextLength": 8000 }),
            None,
        ));
        let service = LiveSessionService::new(Arc::clone(&store), metadata)
            .with_tts_service(Arc::clone(&mock_tts) as Arc<dyn TtsService>);

        let error = service
            .voice_generate(serde_json::json!({ "key": "main", "messageIndex": 0 }))
            .await
            .expect_err("should reject non-assistant target");
        assert!(error.to_string().contains("not an assistant"));
    }

    #[tokio::test]
    async fn voice_generate_prefers_run_id_over_non_assistant_message_index() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        let existing_path = store
            .save_media("main", "voice-msg-2.ogg", b"OggSreuse")
            .await
            .expect("save media");

        store
            .append(
                "main",
                &serde_json::json!({ "role": "user", "content": "hello" }),
            )
            .await
            .expect("append user");
        store
            .append(
                "main",
                &serde_json::json!({ "role": "tool_result", "content": "tool output" }),
            )
            .await
            .expect("append tool_result");
        store
            .append(
                "main",
                &serde_json::json!({
                    "role": "assistant",
                    "content": "assistant answer",
                    "audio": existing_path,
                    "run_id": "run-target",
                }),
            )
            .await
            .expect("append assistant");

        let mock_tts = Arc::new(MockTtsService::with_convert_error(
            serde_json::json!({ "enabled": true, "maxTextLength": 8000 }),
            "convert should not be called",
        ));
        let service = LiveSessionService::new(Arc::clone(&store), metadata)
            .with_tts_service(Arc::clone(&mock_tts) as Arc<dyn TtsService>);

        let result = service
            .voice_generate(
                serde_json::json!({ "key": "main", "runId": "run-target", "messageIndex": 1 }),
            )
            .await
            .expect("voice generate");

        assert_eq!(result["reused"], true);
        assert_eq!(result["messageIndex"], 2);
        assert_eq!(result["audio"].as_str(), Some("media/main/voice-msg-2.ogg"));
        assert_eq!(mock_tts.convert_calls.load(Ordering::SeqCst), 0);
    }

    // --- Browser service integration tests ---

    use std::sync::atomic::{AtomicU32, Ordering};

    /// Mock browser service that tracks lifecycle method calls.
    struct MockBrowserService {
        close_all_calls: AtomicU32,
    }

    impl MockBrowserService {
        fn new() -> Self {
            Self {
                close_all_calls: AtomicU32::new(0),
            }
        }

        fn close_all_count(&self) -> u32 {
            self.close_all_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl crate::services::BrowserService for MockBrowserService {
        async fn request(&self, _p: Value) -> ServiceResult {
            Err("mock".into())
        }

        async fn close_all(&self) {
            self.close_all_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn sqlite_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        // Projects table must exist before sessions (FK constraint).
        moltis_projects::run_migrations(&pool).await.unwrap();
        SqliteSessionMetadata::init(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn resolve_dispatches_session_start_with_channel_binding() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        let key = "telegram:bot-main:-100123";
        metadata.upsert(key, None).await.unwrap();
        let binding_json = serde_json::to_string(&moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "bot-main".to_string(),
            chat_id: "-100123".to_string(),
            message_id: Some("9".to_string()),
            thread_id: None,
        })
        .unwrap();
        metadata.set_channel_binding(key, Some(binding_json)).await;

        let payloads = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut hook_registry = HookRegistry::new();
        hook_registry.register(Arc::new(RecordingHook {
            payloads: Arc::clone(&payloads),
        }));

        let service = LiveSessionService::new(store, metadata).with_hooks(Arc::new(hook_registry));
        service
            .resolve(serde_json::json!({ "key": key, "include_history": false }))
            .await
            .unwrap();

        let payloads = payloads.lock().unwrap();
        let payload = payloads
            .first()
            .unwrap_or_else(|| panic!("missing SessionStart payload"));
        match payload {
            HookPayload::SessionStart { channel, .. } => {
                let channel = channel.clone().unwrap_or_else(|| panic!("missing channel"));
                assert_eq!(channel.surface.as_deref(), Some("telegram"));
                assert_eq!(channel.session_kind.as_deref(), Some("channel"));
                assert_eq!(channel.channel_type.as_deref(), Some("telegram"));
                assert_eq!(channel.account_id.as_deref(), Some("bot-main"));
                assert_eq!(channel.chat_id.as_deref(), Some("-100123"));
                assert_eq!(channel.chat_type.as_deref(), Some("channel_or_supergroup"));
            },
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_dispatches_session_start_with_web_binding_for_unbound_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        metadata.upsert("main", None).await.unwrap();

        let payloads = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut hook_registry = HookRegistry::new();
        hook_registry.register(Arc::new(RecordingHook {
            payloads: Arc::clone(&payloads),
        }));

        let service = LiveSessionService::new(store, metadata).with_hooks(Arc::new(hook_registry));
        service
            .resolve(serde_json::json!({ "key": "main", "include_history": false }))
            .await
            .unwrap();

        let payloads = payloads.lock().unwrap();
        let payload = payloads
            .first()
            .unwrap_or_else(|| panic!("missing SessionStart payload"));
        match payload {
            HookPayload::SessionStart { channel, .. } => {
                let channel = channel.clone().unwrap_or_else(|| panic!("missing channel"));
                assert_eq!(channel.surface.as_deref(), Some("web"));
                assert_eq!(channel.session_kind.as_deref(), Some("web"));
                assert!(channel.channel_type.is_none());
                assert!(channel.account_id.is_none());
                assert!(channel.chat_id.is_none());
            },
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_dispatches_session_start_with_web_binding_for_invalid_channel_binding() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        let key = "telegram:bot-main:-100123";
        metadata.upsert(key, None).await.unwrap();
        metadata
            .set_channel_binding(key, Some("{not-json".to_string()))
            .await;

        let payloads = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut hook_registry = HookRegistry::new();
        hook_registry.register(Arc::new(RecordingHook {
            payloads: Arc::clone(&payloads),
        }));

        let service = LiveSessionService::new(store, metadata).with_hooks(Arc::new(hook_registry));
        service
            .resolve(serde_json::json!({ "key": key, "include_history": false }))
            .await
            .unwrap();

        let payloads = payloads.lock().unwrap();
        let payload = payloads
            .first()
            .unwrap_or_else(|| panic!("missing SessionStart payload"));
        match payload {
            HookPayload::SessionStart { channel, .. } => {
                let channel = channel.clone().unwrap_or_else(|| panic!("missing channel"));
                assert_eq!(channel.surface.as_deref(), Some("web"));
                assert_eq!(channel.session_kind.as_deref(), Some("web"));
                assert!(channel.channel_type.is_none());
                assert!(channel.account_id.is_none());
                assert!(channel.chat_id.is_none());
            },
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn with_browser_service_builder() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        let mock = Arc::new(MockBrowserService::new());
        let svc = LiveSessionService::new(store, metadata)
            .with_browser_service(Arc::clone(&mock) as Arc<dyn crate::services::BrowserService>);

        assert!(svc.browser_service.is_some());
    }

    #[tokio::test]
    async fn clear_all_calls_browser_close_all() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        let mock = Arc::new(MockBrowserService::new());
        let svc = LiveSessionService::new(store, metadata)
            .with_browser_service(Arc::clone(&mock) as Arc<dyn crate::services::BrowserService>);

        let result = svc.clear_all().await;
        assert!(result.is_ok());
        assert_eq!(mock.close_all_count(), 1, "close_all should be called once");
    }

    #[tokio::test]
    async fn clear_all_without_browser_service() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        // No browser_service wired.
        let svc = LiveSessionService::new(store, metadata);

        let result = svc.clear_all().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn patch_sandbox_toggle_appends_system_notification() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        metadata
            .upsert("main", Some("Test".to_string()))
            .await
            .unwrap();

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        // Enable sandbox — should append a system notification.
        let result = svc
            .patch(serde_json::json!({ "key": "main", "sandboxEnabled": true }))
            .await;
        assert!(result.is_ok());
        let msgs = store.read("main").await.unwrap();
        assert_eq!(msgs.len(), 1, "should have one system notification");
        assert_eq!(msgs[0]["role"], "system");
        let content = msgs[0]["content"].as_str().unwrap();
        assert!(
            content.contains("enabled"),
            "notification should mention enabled"
        );

        // Disable sandbox — should append another notification.
        let result = svc
            .patch(serde_json::json!({ "key": "main", "sandboxEnabled": false }))
            .await;
        assert!(result.is_ok());
        let msgs = store.read("main").await.unwrap();
        assert_eq!(msgs.len(), 2, "should have two system notifications");
        assert_eq!(msgs[1]["role"], "system");
        let content = msgs[1]["content"].as_str().unwrap();
        assert!(
            content.contains("disabled"),
            "notification should mention disabled"
        );
    }

    #[tokio::test]
    async fn patch_sandbox_no_change_skips_notification() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        metadata
            .upsert("main", Some("Test".to_string()))
            .await
            .unwrap();

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        // Enable sandbox first.
        svc.patch(serde_json::json!({ "key": "main", "sandboxEnabled": true }))
            .await
            .unwrap();

        // Patch again with the same value — no new notification.
        svc.patch(serde_json::json!({ "key": "main", "sandboxEnabled": true }))
            .await
            .unwrap();
        let msgs = store.read("main").await.unwrap();
        assert_eq!(msgs.len(), 1, "no duplicate notification for same value");
    }

    #[tokio::test]
    async fn patch_sandbox_null_clears_override_with_notification() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        metadata
            .upsert("main", Some("Test".to_string()))
            .await
            .unwrap();

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        // Enable sandbox first.
        svc.patch(serde_json::json!({ "key": "main", "sandboxEnabled": true }))
            .await
            .unwrap();

        // Clear override with null.
        svc.patch(serde_json::json!({ "key": "main", "sandboxEnabled": null }))
            .await
            .unwrap();
        let msgs = store.read("main").await.unwrap();
        assert_eq!(msgs.len(), 2, "clearing override should add notification");
        let content = msgs[1]["content"].as_str().unwrap();
        assert!(
            content.contains("cleared"),
            "notification should mention cleared"
        );
    }

    #[tokio::test]
    async fn patch_archived_updates_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        metadata
            .upsert("session:archive-me", Some("Test".to_string()))
            .await
            .unwrap();

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        let result = svc
            .patch(serde_json::json!({ "key": "session:archive-me", "archived": true }))
            .await
            .unwrap();
        assert_eq!(result.get("archived").and_then(|v| v.as_bool()), Some(true));
        assert!(metadata.get("session:archive-me").await.unwrap().archived);
    }

    #[tokio::test]
    async fn patch_archived_rejects_main_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        metadata
            .upsert("main", Some("Main".to_string()))
            .await
            .unwrap();

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        let error = svc
            .patch(serde_json::json!({ "key": "main", "archived": true }))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("cannot be archived"));
        assert!(!metadata.get("main").await.unwrap().archived);
    }

    #[tokio::test]
    async fn patch_archived_rejects_current_default_channel_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        let binding =
            r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#.to_string();
        metadata
            .upsert("telegram:bot1:123", Some("Telegram current".to_string()))
            .await
            .unwrap();
        metadata
            .set_channel_binding("telegram:bot1:123", Some(binding))
            .await;

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        let error = svc
            .patch(serde_json::json!({ "key": "telegram:bot1:123", "archived": true }))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("cannot be archived"));
        assert!(!metadata.get("telegram:bot1:123").await.unwrap().archived);
    }

    #[tokio::test]
    async fn patch_archived_allows_noncurrent_channel_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        let binding =
            r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#.to_string();
        metadata
            .upsert(
                "session:telegram-archive",
                Some("Telegram archive".to_string()),
            )
            .await
            .unwrap();
        metadata
            .set_channel_binding("session:telegram-archive", Some(binding.clone()))
            .await;
        metadata
            .set_active_session("telegram", "bot1", "123", None, "telegram:bot1:123")
            .await;

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        let result = svc
            .patch(serde_json::json!({ "key": "session:telegram-archive", "archived": true }))
            .await
            .unwrap();
        assert_eq!(result.get("archived").and_then(|v| v.as_bool()), Some(true));
        assert!(
            metadata
                .get("session:telegram-archive")
                .await
                .unwrap()
                .archived
        );
    }

    #[tokio::test]
    async fn patch_archived_allows_cron_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        metadata
            .upsert("cron:archive-me", Some("Cron archive".to_string()))
            .await
            .unwrap();

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        let result = svc
            .patch(serde_json::json!({ "key": "cron:archive-me", "archived": true }))
            .await
            .unwrap();
        assert_eq!(result.get("archived").and_then(|v| v.as_bool()), Some(true));
        assert!(metadata.get("cron:archive-me").await.unwrap().archived);
    }

    #[tokio::test]
    async fn patch_archived_allows_unarchive_for_current_channel_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        let binding =
            r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#.to_string();
        metadata
            .upsert("telegram:bot1:123", Some("Telegram current".to_string()))
            .await
            .unwrap();
        metadata
            .set_channel_binding("telegram:bot1:123", Some(binding))
            .await;
        metadata.set_archived("telegram:bot1:123", true).await;

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        let result = svc
            .patch(serde_json::json!({ "key": "telegram:bot1:123", "archived": false }))
            .await
            .unwrap();
        assert_eq!(
            result.get("archived").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert!(!metadata.get("telegram:bot1:123").await.unwrap().archived);
    }

    #[tokio::test]
    async fn patch_archived_rejection_does_not_partially_mutate_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        metadata
            .upsert("main", Some("Main".to_string()))
            .await
            .unwrap();
        metadata
            .set_model("main", Some("claude-sonnet".to_string()))
            .await;

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        let error = svc
            .patch(serde_json::json!({
                "key": "main",
                "label": "Mutated?",
                "model": "gpt-5",
                "archived": true
            }))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("cannot be archived"));

        let entry = metadata.get("main").await.unwrap();
        assert_eq!(entry.label.as_deref(), Some("Main"));
        assert_eq!(entry.model.as_deref(), Some("claude-sonnet"));
        assert!(!entry.archived);
    }

    #[tokio::test]
    async fn search_excludes_archived_sessions_unless_requested() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        metadata
            .upsert("session:visible", Some("Visible".to_string()))
            .await
            .unwrap();
        metadata
            .upsert("session:hidden", Some("Hidden".to_string()))
            .await
            .unwrap();
        metadata.set_archived("session:hidden", true).await;
        store
            .append(
                "session:visible",
                &serde_json::json!({"role": "user", "content": "archive needle visible"}),
            )
            .await
            .unwrap();
        store
            .append(
                "session:hidden",
                &serde_json::json!({"role": "user", "content": "archive needle hidden"}),
            )
            .await
            .unwrap();

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        let default_results = svc
            .search(serde_json::json!({ "query": "needle", "limit": 10 }))
            .await
            .unwrap()
            .as_array()
            .cloned()
            .unwrap();
        assert_eq!(default_results.len(), 1);
        assert_eq!(default_results[0]["sessionKey"], "session:visible");
        assert_eq!(default_results[0]["archived"], false);

        let include_archived_results = svc
            .search(serde_json::json!({
                "query": "needle",
                "limit": 10,
                "includeArchived": true
            }))
            .await
            .unwrap()
            .as_array()
            .cloned()
            .unwrap();
        assert_eq!(include_archived_results.len(), 2);
        assert!(
            include_archived_results
                .iter()
                .any(|entry| entry["sessionKey"] == "session:hidden" && entry["archived"] == true)
        );
    }

    #[tokio::test]
    async fn search_includes_results_without_metadata_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        store
            .append(
                "session:orphaned",
                &serde_json::json!({"role": "user", "content": "needle without metadata"}),
            )
            .await
            .unwrap();

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

        let results = svc
            .search(serde_json::json!({ "query": "needle", "limit": 10 }))
            .await
            .unwrap()
            .as_array()
            .cloned()
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["sessionKey"], "session:orphaned");
        assert!(results[0]["label"].is_null());
        assert_eq!(results[0]["archived"], false);
    }

    #[cfg(feature = "fs-tools")]
    #[tokio::test]
    async fn delete_clears_fs_state_for_session() {
        use std::path::{Path, PathBuf};

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        metadata
            .upsert("side", Some("Test".to_string()))
            .await
            .unwrap();

        let fs_state = moltis_tools::fs::new_fs_state(false);
        {
            let mut guard = fs_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = guard.record_read("side", PathBuf::from("/tmp/demo.txt"), 0, 25, None);
            assert!(guard.has_been_read("side", Path::new("/tmp/demo.txt")));
        }

        let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata))
            .with_fs_state(Arc::clone(&fs_state));

        let result = svc.delete(serde_json::json!({ "key": "side" })).await;
        assert!(result.is_ok());

        let guard = fs_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(!guard.has_been_read("side", Path::new("/tmp/demo.txt")));
    }
}
