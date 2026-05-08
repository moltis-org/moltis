use super::*;

impl LiveSessionService {
    pub(super) async fn voice_generate_impl(&self, params: Value) -> ServiceResult {
        let p: VoiceGenerateParams = parse_params(params)?;
        let key = &p.key;
        let target = p.target().map_err(ServiceError::message)?;

        let tts = self
            .tts_service
            .as_ref()
            .ok_or_else(|| "session voice generation is not configured".to_string())?;

        let mut history = self.store.read(key).await.map_err(ServiceError::message)?;
        if history.is_empty() {
            return Err(format!("session '{key}' has no messages").into());
        }

        let target_index = match &target {
            VoiceTarget::ByRunId(id) => history
                .iter()
                .rposition(|msg| {
                    msg.get("role").and_then(|v| v.as_str()) == Some("assistant")
                        && msg.get("run_id").and_then(|v| v.as_str()) == Some(id)
                })
                .ok_or_else(|| "target assistant message not found".to_string())?,
            VoiceTarget::ByMessageIndex(idx) => *idx,
        };
        let target_msg = history
            .get(target_index)
            .ok_or_else(|| format!("message index {target_index} is out of range"))?;
        if target_msg.get("role").and_then(|v| v.as_str()) != Some("assistant") {
            return Err("target message is not an assistant response".into());
        }

        if let Some(existing_audio) = target_msg.get("audio").and_then(|v| v.as_str())
            && !existing_audio.trim().is_empty()
            && let Some(filename) = media_filename(existing_audio)
            && self.store.read_media(key, filename).await.is_ok()
        {
            let tts_provider = target_msg.get("tts_provider").and_then(|v| v.as_str());
            return Ok(serde_json::json!({
                "sessionKey": key,
                "messageIndex": target_index,
                "audio": existing_audio,
                "ttsProvider": tts_provider,
                "reused": true,
            }));
        }

        let text = message_text(target_msg)
            .ok_or_else(|| "assistant message has no text content to synthesize".to_string())?;
        let sanitized = sanitize_tts_text(&text).trim().to_string();
        if sanitized.is_empty() {
            return Err("assistant message has no speakable text for TTS".into());
        }

        let status_value = tts
            .status()
            .await
            .map_err(|e| format!("failed to check TTS status: {e}"))?;
        let status: TtsStatusPayload = serde_json::from_value(status_value)
            .map_err(|_| ServiceError::message("invalid TTS status payload"))?;
        if !status.enabled {
            return Err("TTS is disabled or provider is not configured".into());
        }
        if let Some(max_text_length) = status.max_text_length
            && sanitized.len() > max_text_length
        {
            return Err(format!(
                "text exceeds max length ({} > {})",
                sanitized.len(),
                max_text_length
            )
            .into());
        }

        // Resolve voice persona through the full chain:
        // session agent's voice_persona_id → global active persona.
        let mut convert_params = serde_json::json!({
            "text": sanitized,
            "format": "ogg",
        });
        if let Some(ref vp_store) = self.voice_persona_store {
            let persona = crate::voice_persona::resolve_persona(
                vp_store,
                self.agent_persona_store.as_deref(),
                None,
                Some(key),
                Some(&*self.metadata),
            )
            .await;
            if let Some(persona) = persona
                && let Ok(v) = serde_json::to_value(&persona)
            {
                convert_params["persona"] = v;
            }
        }

        let convert_value = tts
            .convert(convert_params)
            .await
            .map_err(|e| format!("TTS convert failed: {e}"))?;
        let convert: TtsConvertPayload = serde_json::from_value(convert_value)
            .map_err(|_| ServiceError::message("invalid TTS convert payload"))?;
        let audio_bytes = general_purpose::STANDARD
            .decode(convert.audio.trim())
            .map_err(|_| {
                ServiceError::message("invalid base64 audio payload returned by TTS provider")
            })?;

        let filename = format!("voice-msg-{target_index}.ogg");
        let audio_path = self
            .store
            .save_media(key, &filename, &audio_bytes)
            .await
            .map_err(ServiceError::message)?;

        let target_mut = history
            .get_mut(target_index)
            .ok_or_else(|| format!("message index {target_index} is out of range"))?;
        let target_obj = target_mut
            .as_object_mut()
            .ok_or_else(|| "target message is not an object".to_string())?;
        target_obj.insert("audio".to_string(), Value::String(audio_path.clone()));
        if let Some(provider) = convert
            .provider
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            target_obj.insert(
                "tts_provider".to_string(),
                Value::String(provider.to_string()),
            );
        }

        let message_count = history.len() as u32;
        self.store
            .replace_history(key, history)
            .await
            .map_err(ServiceError::message)?;
        self.metadata.touch(key, message_count).await;

        Ok(serde_json::json!({
            "sessionKey": key,
            "messageIndex": target_index,
            "audio": audio_path,
            "ttsProvider": convert.provider,
            "reused": false,
        }))
    }
}
