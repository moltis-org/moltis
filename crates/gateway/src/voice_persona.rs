//! Voice persona store — SQLite-backed CRUD for named voice identities.
//!
//! Each persona defines a stable spoken identity (voice, model, style
//! instructions) that is injected deterministically into TTS synthesis calls.

use {
    moltis_voice::{
        FallbackPolicy, SynthesizeRequest, TtsProviderId, VoicePersona, VoicePersonaPrompt,
        VoicePersonaProviderBinding,
    },
    serde::{Deserialize, Serialize},
    std::time::{SystemTime, UNIX_EPOCH},
};

/// Errors from voice persona operations.
#[derive(Debug, thiserror::Error)]
pub enum VoicePersonaError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    InvalidRequest(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl From<VoicePersonaError> for moltis_protocol::ErrorShape {
    fn from(err: VoicePersonaError) -> Self {
        use moltis_protocol::error_codes;
        match &err {
            VoicePersonaError::NotFound(_) | VoicePersonaError::InvalidRequest(_) => {
                Self::new(error_codes::INVALID_REQUEST, err.to_string())
            },
            VoicePersonaError::Db(_) | VoicePersonaError::Json(_) => {
                Self::new(error_codes::UNAVAILABLE, err.to_string())
            },
        }
    }
}

type Result<T> = std::result::Result<T, VoicePersonaError>;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Validate a persona ID: lowercase alphanumeric + hyphens, 1-50 chars.
pub fn validate_persona_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 50 {
        return Err(VoicePersonaError::InvalidRequest(
            "id must be 1-50 characters".into(),
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(VoicePersonaError::InvalidRequest(
            "id must contain only lowercase letters, digits, and hyphens".into(),
        ));
    }
    if id.starts_with('-') || id.ends_with('-') {
        return Err(VoicePersonaError::InvalidRequest(
            "id must not start or end with a hyphen".into(),
        ));
    }
    Ok(())
}

/// Parameters for creating a new voice persona.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVoicePersonaParams {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub fallback_policy: Option<String>,
    #[serde(default)]
    pub prompt: Option<VoicePersonaPrompt>,
    #[serde(default)]
    pub provider_bindings: Option<Vec<VoicePersonaProviderBinding>>,
}

/// Parameters for updating a voice persona.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVoicePersonaParams {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub fallback_policy: Option<String>,
    #[serde(default)]
    pub prompt: Option<VoicePersonaPrompt>,
    #[serde(default)]
    pub provider_bindings: Option<Vec<VoicePersonaProviderBinding>>,
}

/// Wire format returned by list/get endpoints.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoicePersonaResponse {
    pub persona: VoicePersona,
    pub is_active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct PersonaRow {
    id: String,
    label: String,
    description: Option<String>,
    provider: Option<String>,
    fallback_policy: String,
    prompt_json: String,
    bindings_json: String,
    is_active: i64,
    created_at: i64,
    updated_at: i64,
}

impl PersonaRow {
    fn into_response(self) -> std::result::Result<VoicePersonaResponse, serde_json::Error> {
        let prompt: VoicePersonaPrompt = serde_json::from_str(&self.prompt_json)?;
        let provider_bindings: Vec<VoicePersonaProviderBinding> =
            serde_json::from_str(&self.bindings_json)?;
        let provider = self.provider.and_then(|s| TtsProviderId::parse(&s));
        let fallback_policy: FallbackPolicy =
            serde_json::from_value(serde_json::Value::String(self.fallback_policy.clone()))
                .unwrap_or_default();

        Ok(VoicePersonaResponse {
            persona: VoicePersona {
                id: self.id,
                label: self.label,
                description: self.description,
                provider,
                fallback_policy,
                prompt,
                provider_bindings,
            },
            is_active: self.is_active != 0,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// SQLite-backed voice persona store.
pub struct VoicePersonaStore {
    pool: sqlx::SqlitePool,
}

impl VoicePersonaStore {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    /// List all voice personas.
    pub async fn list(&self) -> Result<Vec<VoicePersonaResponse>> {
        let rows: Vec<PersonaRow> =
            sqlx::query_as("SELECT * FROM voice_personas ORDER BY created_at ASC")
                .fetch_all(&self.pool)
                .await?;

        rows.into_iter()
            .map(|r| r.into_response().map_err(VoicePersonaError::Json))
            .collect()
    }

    /// Get a single voice persona by ID.
    pub async fn get(&self, id: &str) -> Result<Option<VoicePersonaResponse>> {
        let row: Option<PersonaRow> = sqlx::query_as("SELECT * FROM voice_personas WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        row.map(|r| r.into_response().map_err(VoicePersonaError::Json))
            .transpose()
    }

    /// Create a new voice persona.
    pub async fn create(&self, params: CreateVoicePersonaParams) -> Result<VoicePersonaResponse> {
        validate_persona_id(&params.id)?;

        let prompt_json = serde_json::to_string(&params.prompt.unwrap_or_default())?;
        let bindings_json = serde_json::to_string(&params.provider_bindings.unwrap_or_default())?;
        let fallback = params.fallback_policy.unwrap_or_default();
        let now = now_ms();

        sqlx::query(
            r"INSERT INTO voice_personas (id, label, description, provider, fallback_policy, prompt_json, bindings_json, is_active, created_at, updated_at)
              VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&params.id)
        .bind(&params.label)
        .bind(&params.description)
        .bind(&params.provider)
        .bind(if fallback.is_empty() {
            "preserve-persona"
        } else {
            &fallback
        })
        .bind(&prompt_json)
        .bind(&bindings_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        self.get(&params.id)
            .await?
            .ok_or_else(|| VoicePersonaError::NotFound("just-created persona missing".into()))
    }

    /// Update an existing voice persona.
    pub async fn update(
        &self,
        id: &str,
        params: UpdateVoicePersonaParams,
    ) -> Result<VoicePersonaResponse> {
        let existing = self
            .get(id)
            .await?
            .ok_or_else(|| VoicePersonaError::NotFound(format!("persona '{id}' not found")))?;

        let label = params.label.unwrap_or(existing.persona.label);
        let description = params.description.or(existing.persona.description);
        let provider = params
            .provider
            .or_else(|| existing.persona.provider.map(|p| p.to_string()));
        let fallback_policy = params.fallback_policy.unwrap_or_else(|| {
            serde_json::to_value(existing.persona.fallback_policy)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "preserve-persona".into())
        });
        let prompt_json = if let Some(ref prompt) = params.prompt {
            serde_json::to_string(prompt)?
        } else {
            serde_json::to_string(&existing.persona.prompt)?
        };
        let bindings_json = if let Some(ref bindings) = params.provider_bindings {
            serde_json::to_string(bindings)?
        } else {
            serde_json::to_string(&existing.persona.provider_bindings)?
        };
        let now = now_ms();

        sqlx::query(
            "UPDATE voice_personas SET label = ?, description = ?, provider = ?, fallback_policy = ?, prompt_json = ?, bindings_json = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&label)
        .bind(&description)
        .bind(&provider)
        .bind(&fallback_policy)
        .bind(&prompt_json)
        .bind(&bindings_json)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        self.get(id)
            .await?
            .ok_or_else(|| VoicePersonaError::NotFound(format!("persona '{id}' not found")))
    }

    /// Delete a voice persona.
    pub async fn delete(&self, id: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM voice_personas WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(VoicePersonaError::NotFound(format!(
                "persona '{id}' not found"
            )));
        }
        Ok(())
    }

    /// Get the currently active voice persona, if any.
    pub async fn get_active(&self) -> Result<Option<VoicePersonaResponse>> {
        let row: Option<PersonaRow> =
            sqlx::query_as("SELECT * FROM voice_personas WHERE is_active = 1 LIMIT 1")
                .fetch_optional(&self.pool)
                .await?;

        row.map(|r| r.into_response().map_err(VoicePersonaError::Json))
            .transpose()
    }

    /// Set the active voice persona. Pass `None` to deactivate all.
    pub async fn set_active(&self, id: Option<&str>) -> Result<Option<VoicePersonaResponse>> {
        let mut tx = self.pool.begin().await?;

        // Clear all active flags.
        sqlx::query("UPDATE voice_personas SET is_active = 0")
            .execute(&mut *tx)
            .await?;

        if let Some(id) = id {
            let updated =
                sqlx::query("UPDATE voice_personas SET is_active = 1, updated_at = ? WHERE id = ?")
                    .bind(now_ms())
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;

            if updated.rows_affected() == 0 {
                return Err(VoicePersonaError::NotFound(format!(
                    "persona '{id}' not found"
                )));
            }
        }

        tx.commit().await?;

        match id {
            Some(id) => self.get(id).await,
            None => Ok(None),
        }
    }

    /// Seed built-in default personas on first run.
    ///
    /// Only inserts personas whose IDs don't already exist, so users who
    /// delete or rename a default won't get it re-created on restart.
    pub async fn seed_defaults(&self) -> Result<usize> {
        let mut seeded = 0usize;
        for def in DEFAULT_PERSONAS {
            let exists =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM voice_personas WHERE id = ?")
                    .bind(def.id)
                    .fetch_one(&self.pool)
                    .await?;

            if exists > 0 {
                continue;
            }

            let prompt = VoicePersonaPrompt {
                profile: Some(def.profile.to_string()),
                style: Some(def.style.to_string()),
                accent: def.accent.map(str::to_string),
                pacing: def.pacing.map(str::to_string),
                scene: None,
                constraints: Vec::new(),
            };
            let prompt_json = serde_json::to_string(&prompt)?;
            let bindings_json = serde_json::to_string(&def.bindings())?;
            let now = now_ms();

            sqlx::query(
                r"INSERT OR IGNORE INTO voice_personas (id, label, description, provider, fallback_policy, prompt_json, bindings_json, is_active, created_at, updated_at)
                  VALUES (?, ?, ?, ?, 'preserve-persona', ?, ?, 0, ?, ?)",
            )
            .bind(def.id)
            .bind(def.label)
            .bind(def.description)
            .bind(def.provider)
            .bind(&prompt_json)
            .bind(&bindings_json)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await?;

            seeded += 1;
        }
        Ok(seeded)
    }
}

// ── Built-in default personas ──────────────────────────────────────────────

struct DefaultPersona {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    provider: &'static str,
    profile: &'static str,
    style: &'static str,
    accent: Option<&'static str>,
    pacing: Option<&'static str>,
    openai_voice: &'static str,
    elevenlabs_voice: &'static str,
}

impl DefaultPersona {
    fn bindings(&self) -> Vec<VoicePersonaProviderBinding> {
        vec![
            VoicePersonaProviderBinding {
                provider: TtsProviderId::OpenAi,
                voice_id: Some(self.openai_voice.to_string()),
                model: Some("gpt-4o-mini-tts".to_string()),
                speed: None,
                stability: None,
                similarity_boost: None,
                speaking_rate: None,
                pitch: None,
            },
            VoicePersonaProviderBinding {
                provider: TtsProviderId::ElevenLabs,
                voice_id: Some(self.elevenlabs_voice.to_string()),
                model: None,
                speed: None,
                stability: Some(0.5),
                similarity_boost: Some(0.75),
                speaking_rate: None,
                pitch: None,
            },
        ]
    }
}

const DEFAULT_PERSONAS: &[DefaultPersona] = &[
    DefaultPersona {
        id: "assistant",
        label: "Assistant",
        description: "Friendly, clear, and helpful — the default voice",
        provider: "openai",
        profile: "A warm, knowledgeable assistant. Clear and approachable.",
        style: "Conversational, friendly, and concise",
        accent: None,
        pacing: Some("Natural, moderate pace"),
        openai_voice: "alloy",
        elevenlabs_voice: "21m00Tcm4TlvDq8ikWAM", // Rachel
    },
    DefaultPersona {
        id: "narrator",
        label: "Narrator",
        description: "Rich storytelling voice for long-form content",
        provider: "openai",
        profile: "A captivating narrator with a deep, resonant voice",
        style: "Dramatic, engaging, and expressive",
        accent: None,
        pacing: Some("Measured, with intentional pauses for emphasis"),
        openai_voice: "onyx",
        elevenlabs_voice: "29vD33N1CtxCmqQRPOHJ", // Drew
    },
    DefaultPersona {
        id: "casual",
        label: "Casual",
        description: "Relaxed and upbeat for informal conversations",
        provider: "openai",
        profile: "A laid-back, friendly personality. Like chatting with a friend.",
        style: "Relaxed, warm, occasionally playful",
        accent: None,
        pacing: Some("Quick and natural, like everyday conversation"),
        openai_voice: "nova",
        elevenlabs_voice: "EXAVITQu4vr4xnSDxMaL", // Bella
    },
    DefaultPersona {
        id: "professional",
        label: "Professional",
        description: "Authoritative and precise for business or technical content",
        provider: "openai",
        profile: "A polished professional. Confident and precise.",
        style: "Authoritative, clear, and structured",
        accent: Some("Neutral, broadcast-quality"),
        pacing: Some("Steady and deliberate"),
        openai_voice: "echo",
        elevenlabs_voice: "pNInz6obpgDQGcFmaJgB", // Adam
    },
];

/// Resolve which voice persona should be used for a TTS call.
///
/// Resolution chain (first match wins):
/// 1. Explicit persona ID passed in the request
/// 2. Session's agent has a `voice_persona_id` link
/// 3. Global active persona in the store
/// 4. None
pub async fn resolve_persona(
    store: &VoicePersonaStore,
    agent_persona_store: Option<&crate::agent_persona::AgentPersonaStore>,
    explicit_persona_id: Option<&str>,
    session_key: Option<&str>,
    session_metadata: Option<&moltis_sessions::metadata::SqliteSessionMetadata>,
) -> Option<VoicePersona> {
    // 1. Explicit persona ID from request params.
    if let Some(id) = explicit_persona_id
        && let Ok(Some(r)) = store.get(id).await
    {
        return Some(r.persona);
    }

    // 2. Session's agent → agent's voice_persona_id.
    if let (Some(key), Some(meta), Some(agent_store)) =
        (session_key, session_metadata, agent_persona_store)
    {
        let vp_id = resolve_agent_voice_persona_id(meta, agent_store, key).await;
        if let Some(ref id) = vp_id
            && let Ok(Some(r)) = store.get(id).await
        {
            return Some(r.persona);
        }
    }

    // 3. Global active persona.
    if let Ok(Some(r)) = store.get_active().await {
        return Some(r.persona);
    }

    None
}

/// Look up the session's agent and return its voice_persona_id (if set).
async fn resolve_agent_voice_persona_id(
    meta: &moltis_sessions::metadata::SqliteSessionMetadata,
    agent_store: &crate::agent_persona::AgentPersonaStore,
    session_key: &str,
) -> Option<String> {
    let entry = meta.get(session_key).await?;
    let agent_id = entry.agent_id.as_deref().filter(|s| !s.is_empty())?;
    let agent = agent_store.get(agent_id).await.ok().flatten()?;
    agent.voice_persona_id
}

/// Apply persona overrides to a `SynthesizeRequest`.
///
/// This is the core resolution logic: given an active persona and the
/// provider that will handle synthesis, merge the persona's bindings
/// and instructions into the request.
pub fn apply_persona_to_request(
    request: &mut SynthesizeRequest,
    persona: &VoicePersona,
    provider_id: TtsProviderId,
) -> std::result::Result<(), FallbackPolicy> {
    let binding = persona.binding_for(provider_id);

    match (binding, persona.fallback_policy) {
        (None, FallbackPolicy::Fail) => return Err(FallbackPolicy::Fail),
        (None, FallbackPolicy::ProviderDefaults) => return Ok(()),
        _ => {},
    }

    // Apply binding overrides (voice_id, model, speed, etc.).
    if let Some(b) = binding {
        if request.voice_id.is_none() {
            request.voice_id.clone_from(&b.voice_id);
        }
        if request.model.is_none() {
            request.model.clone_from(&b.model);
        }
        if request.speed.is_none() {
            request.speed = b.speed.or(b.speaking_rate);
        }
        if request.stability.is_none() {
            request.stability = b.stability;
        }
        if request.similarity_boost.is_none() {
            request.similarity_boost = b.similarity_boost;
        }
    }

    // Render and inject persona instructions.
    if request.instructions.is_none() {
        request.instructions = persona.render_instructions();
    }

    Ok(())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_persona_id() {
        assert!(validate_persona_id("alfred").is_ok());
        assert!(validate_persona_id("my-persona-1").is_ok());
        assert!(validate_persona_id("a").is_ok());

        assert!(validate_persona_id("").is_err());
        assert!(validate_persona_id("UPPER").is_err());
        assert!(validate_persona_id("has space").is_err());
        assert!(validate_persona_id("-leading").is_err());
        assert!(validate_persona_id("trailing-").is_err());
        assert!(validate_persona_id(&"a".repeat(51)).is_err());
    }

    #[test]
    fn test_apply_persona_binding_overrides() {
        let persona = VoicePersona {
            id: "alfred".into(),
            label: "Alfred".into(),
            description: None,
            provider: None,
            fallback_policy: FallbackPolicy::PreservePersona,
            prompt: VoicePersonaPrompt {
                profile: Some("A wise British butler".into()),
                style: Some("Measured".into()),
                ..Default::default()
            },
            provider_bindings: vec![VoicePersonaProviderBinding {
                provider: TtsProviderId::OpenAi,
                voice_id: Some("cedar".into()),
                model: Some("gpt-4o-mini-tts".into()),
                speed: Some(0.9),
                stability: None,
                similarity_boost: None,
                speaking_rate: None,
                pitch: None,
            }],
        };

        let mut request = SynthesizeRequest {
            text: "Good evening, sir.".into(),
            ..Default::default()
        };

        apply_persona_to_request(&mut request, &persona, TtsProviderId::OpenAi).unwrap();

        assert_eq!(request.voice_id.as_deref(), Some("cedar"));
        assert_eq!(request.model.as_deref(), Some("gpt-4o-mini-tts"));
        assert_eq!(request.speed, Some(0.9));
        assert!(request.instructions.is_some());
        assert!(request.instructions.as_ref().unwrap().contains("Alfred"));
    }

    #[test]
    fn test_apply_persona_no_binding_preserve() {
        let persona = VoicePersona {
            id: "narrator".into(),
            label: "Narrator".into(),
            description: None,
            provider: None,
            fallback_policy: FallbackPolicy::PreservePersona,
            prompt: VoicePersonaPrompt {
                profile: Some("Epic narrator".into()),
                ..Default::default()
            },
            provider_bindings: vec![],
        };

        let mut request = SynthesizeRequest {
            text: "Once upon a time.".into(),
            ..Default::default()
        };

        // No binding for Google, but PreservePersona → still inject instructions.
        apply_persona_to_request(&mut request, &persona, TtsProviderId::Google).unwrap();
        assert!(request.instructions.is_some());
    }

    #[test]
    fn test_apply_persona_no_binding_fail() {
        let persona = VoicePersona {
            id: "strict".into(),
            label: "Strict".into(),
            description: None,
            provider: None,
            fallback_policy: FallbackPolicy::Fail,
            prompt: VoicePersonaPrompt::default(),
            provider_bindings: vec![],
        };

        let mut request = SynthesizeRequest::default();
        let result = apply_persona_to_request(&mut request, &persona, TtsProviderId::Google);
        assert!(result.is_err());
    }

    #[test]
    fn test_apply_persona_no_binding_provider_defaults() {
        let persona = VoicePersona {
            id: "chill".into(),
            label: "Chill".into(),
            description: None,
            provider: None,
            fallback_policy: FallbackPolicy::ProviderDefaults,
            prompt: VoicePersonaPrompt {
                profile: Some("Relaxed".into()),
                ..Default::default()
            },
            provider_bindings: vec![],
        };

        let mut request = SynthesizeRequest::default();
        apply_persona_to_request(&mut request, &persona, TtsProviderId::Piper).unwrap();

        // ProviderDefaults → no instructions injected.
        assert!(request.instructions.is_none());
        assert!(request.voice_id.is_none());
    }

    #[test]
    fn test_apply_persona_does_not_override_explicit_params() {
        let persona = VoicePersona {
            id: "alfred".into(),
            label: "Alfred".into(),
            description: None,
            provider: None,
            fallback_policy: FallbackPolicy::PreservePersona,
            prompt: VoicePersonaPrompt::default(),
            provider_bindings: vec![VoicePersonaProviderBinding {
                provider: TtsProviderId::OpenAi,
                voice_id: Some("cedar".into()),
                model: Some("gpt-4o-mini-tts".into()),
                speed: None,
                stability: None,
                similarity_boost: None,
                speaking_rate: None,
                pitch: None,
            }],
        };

        let mut request = SynthesizeRequest {
            text: "test".into(),
            voice_id: Some("shimmer".into()),
            model: Some("tts-1".into()),
            ..Default::default()
        };

        apply_persona_to_request(&mut request, &persona, TtsProviderId::OpenAi).unwrap();

        // Explicit params are preserved — persona doesn't override.
        assert_eq!(request.voice_id.as_deref(), Some("shimmer"));
        assert_eq!(request.model.as_deref(), Some("tts-1"));
    }

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS voice_personas (
                id              TEXT PRIMARY KEY,
                label           TEXT NOT NULL,
                description     TEXT,
                provider        TEXT,
                fallback_policy TEXT NOT NULL DEFAULT 'preserve-persona',
                prompt_json     TEXT NOT NULL DEFAULT '{}',
                bindings_json   TEXT NOT NULL DEFAULT '[]',
                is_active       INTEGER NOT NULL DEFAULT 0,
                created_at      INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r"CREATE UNIQUE INDEX IF NOT EXISTS uix_voice_personas_active
              ON voice_personas (is_active) WHERE is_active = 1",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_crud_lifecycle() {
        let pool = test_pool().await;
        let store = VoicePersonaStore::new(pool);

        // Create
        let resp = store
            .create(CreateVoicePersonaParams {
                id: "alfred".into(),
                label: "Alfred".into(),
                description: Some("British butler".into()),
                provider: Some("openai".into()),
                fallback_policy: None,
                prompt: Some(VoicePersonaPrompt {
                    profile: Some("Wise butler".into()),
                    ..Default::default()
                }),
                provider_bindings: Some(vec![VoicePersonaProviderBinding {
                    provider: TtsProviderId::OpenAi,
                    voice_id: Some("cedar".into()),
                    model: None,
                    speed: None,
                    stability: None,
                    similarity_boost: None,
                    speaking_rate: None,
                    pitch: None,
                }]),
            })
            .await
            .unwrap();

        assert_eq!(resp.persona.id, "alfred");
        assert_eq!(resp.persona.label, "Alfred");
        assert!(!resp.is_active);

        // List
        let all = store.list().await.unwrap();
        assert_eq!(all.len(), 1);

        // Get
        let got = store.get("alfred").await.unwrap().unwrap();
        assert_eq!(got.persona.description.as_deref(), Some("British butler"));

        // Update
        let updated = store
            .update("alfred", UpdateVoicePersonaParams {
                label: Some("Alfred the Butler".into()),
                description: None,
                provider: None,
                fallback_policy: None,
                prompt: None,
                provider_bindings: None,
            })
            .await
            .unwrap();
        assert_eq!(updated.persona.label, "Alfred the Butler");

        // Delete
        store.delete("alfred").await.unwrap();
        assert!(store.get("alfred").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_set_active() {
        let pool = test_pool().await;
        let store = VoicePersonaStore::new(pool);

        store
            .create(CreateVoicePersonaParams {
                id: "a".into(),
                label: "A".into(),
                description: None,
                provider: None,
                fallback_policy: None,
                prompt: None,
                provider_bindings: None,
            })
            .await
            .unwrap();

        store
            .create(CreateVoicePersonaParams {
                id: "b".into(),
                label: "B".into(),
                description: None,
                provider: None,
                fallback_policy: None,
                prompt: None,
                provider_bindings: None,
            })
            .await
            .unwrap();

        // No active persona initially.
        assert!(store.get_active().await.unwrap().is_none());

        // Set A active.
        store.set_active(Some("a")).await.unwrap();
        let active = store.get_active().await.unwrap().unwrap();
        assert_eq!(active.persona.id, "a");

        // Switch to B.
        store.set_active(Some("b")).await.unwrap();
        let active = store.get_active().await.unwrap().unwrap();
        assert_eq!(active.persona.id, "b");

        // Deactivate all.
        store.set_active(None).await.unwrap();
        assert!(store.get_active().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_set_active_nonexistent() {
        let pool = test_pool().await;
        let store = VoicePersonaStore::new(pool);
        assert!(store.set_active(Some("nope")).await.is_err());
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let pool = test_pool().await;
        let store = VoicePersonaStore::new(pool);
        assert!(store.delete("nope").await.is_err());
    }

    #[tokio::test]
    async fn test_create_rejects_invalid_id() {
        let pool = test_pool().await;
        let store = VoicePersonaStore::new(pool);
        let result = store
            .create(CreateVoicePersonaParams {
                id: "INVALID".into(),
                label: "Test".into(),
                description: None,
                provider: None,
                fallback_policy: None,
                prompt: None,
                provider_bindings: None,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_seed_defaults() {
        let pool = test_pool().await;
        let store = VoicePersonaStore::new(pool);

        // First seed — should create all defaults.
        let seeded = store.seed_defaults().await.unwrap();
        assert_eq!(seeded, DEFAULT_PERSONAS.len());

        let all = store.list().await.unwrap();
        assert_eq!(all.len(), DEFAULT_PERSONAS.len());
        assert!(all.iter().any(|p| p.persona.id == "assistant"));
        assert!(all.iter().any(|p| p.persona.id == "narrator"));
        assert!(all.iter().any(|p| p.persona.id == "casual"));
        assert!(all.iter().any(|p| p.persona.id == "professional"));

        // Second seed — idempotent, no new rows.
        let seeded2 = store.seed_defaults().await.unwrap();
        assert_eq!(seeded2, 0);
        assert_eq!(store.list().await.unwrap().len(), DEFAULT_PERSONAS.len());
    }

    #[tokio::test]
    async fn test_seed_defaults_skips_existing() {
        let pool = test_pool().await;
        let store = VoicePersonaStore::new(pool);

        // Create one persona manually with the same ID as a default.
        store
            .create(CreateVoicePersonaParams {
                id: "assistant".into(),
                label: "My Custom Assistant".into(),
                description: None,
                provider: None,
                fallback_policy: None,
                prompt: None,
                provider_bindings: None,
            })
            .await
            .unwrap();

        // Seed — should skip "assistant" and create the rest.
        let seeded = store.seed_defaults().await.unwrap();
        assert_eq!(seeded, DEFAULT_PERSONAS.len() - 1);

        // Verify the custom one wasn't overwritten.
        let assistant = store.get("assistant").await.unwrap().unwrap();
        assert_eq!(assistant.persona.label, "My Custom Assistant");
    }
}
