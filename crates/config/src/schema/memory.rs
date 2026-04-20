use {
    secrecy::Secret,
    serde::{Deserialize, Deserializer, Serialize},
    std::collections::HashMap,
};

/// Memory embedding provider configuration.
///
/// Controls which embedding provider the memory system uses.
/// If not configured, the system auto-detects from available providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryEmbeddingConfig {
    /// High-level memory orchestration style.
    pub style: MemoryStyle,
    /// Where agent-authored memory writes are allowed to land.
    pub agent_write_mode: AgentMemoryWriteMode,
    /// How Moltis writes the managed `USER.md` profile surface.
    pub user_profile_write_mode: UserProfileWriteMode,
    /// Memory backend used for search, retrieval, and indexing.
    pub backend: MemoryBackend,
    /// Embedding provider: "local", "ollama", "openai", "custom", or None for auto-detect.
    #[serde(alias = "embedding_provider")]
    pub provider: Option<MemoryProvider>,
    /// Disable RAG embeddings and force keyword-only memory search.
    #[serde(default)]
    pub disable_rag: bool,
    /// Base URL for the embedding API (e.g. "http://localhost:11434/v1" for Ollama).
    #[serde(alias = "embedding_base_url")]
    pub base_url: Option<String>,
    /// Model name (e.g. "nomic-embed-text" for Ollama, "text-embedding-3-small" for OpenAI).
    #[serde(alias = "embedding_model")]
    pub model: Option<String>,
    /// API key (optional for local endpoints like Ollama).
    #[serde(
        default,
        alias = "embedding_api_key",
        serialize_with = "crate::schema::serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_key: Option<Secret<String>>,
    /// Citation mode for memory search results.
    pub citations: MemoryCitationsMode,
    /// Enable LLM reranking for hybrid search results.
    #[serde(default)]
    pub llm_reranking: bool,
    /// Merge strategy for hybrid search results.
    pub search_merge_strategy: MemorySearchMergeStrategy,
    /// How session transcripts are exported into searchable memory.
    #[serde(
        default = "default_session_export_mode",
        deserialize_with = "deserialize_session_export_mode"
    )]
    pub session_export: SessionExportMode,
    /// QMD-specific configuration (only used when backend = "qmd").
    #[serde(default)]
    pub qmd: QmdConfig,
    /// Prefetch relevant memories at the start of each turn and inject them
    /// into the system prompt as `<recalled_context>`. Default: true.
    #[serde(default = "default_true")]
    pub enable_prefetch: bool,
    /// Maximum number of memories to prefetch per turn. Default: 3.
    #[serde(default = "default_prefetch_limit")]
    pub prefetch_limit: usize,
    /// Run a background memory extraction every N turns (0 = disabled,
    /// only pre-compaction). Default: 5.
    #[serde(default = "default_auto_extract_interval")]
    pub auto_extract_interval: u32,
    /// Write a session summary to memory when a session ends
    /// (`/new`, `/reset`, or timeout). Default: true.
    #[serde(default = "default_true")]
    pub enable_session_summary: bool,
}

impl Default for MemoryEmbeddingConfig {
    fn default() -> Self {
        Self {
            style: MemoryStyle::default(),
            agent_write_mode: AgentMemoryWriteMode::default(),
            user_profile_write_mode: UserProfileWriteMode::default(),
            backend: MemoryBackend::default(),
            provider: None,
            disable_rag: false,
            base_url: None,
            model: None,
            api_key: None,
            citations: MemoryCitationsMode::default(),
            llm_reranking: false,
            search_merge_strategy: MemorySearchMergeStrategy::default(),
            session_export: default_session_export_mode(),
            qmd: QmdConfig::default(),
            enable_prefetch: true,
            prefetch_limit: 3,
            auto_extract_interval: 5,
            enable_session_summary: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_prefetch_limit() -> usize {
    3
}

fn default_auto_extract_interval() -> u32 {
    5
}

/// High-level orchestration style for prompt memory and memory tools.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryStyle {
    /// Current behavior: inject `MEMORY.md` into the prompt and expose memory tools.
    #[default]
    Hybrid,
    /// Inject `MEMORY.md` into the prompt, but hide memory tools.
    PromptOnly,
    /// Skip prompt injection and rely on memory tools for recall.
    SearchOnly,
    /// Disable both prompt memory injection and memory tools.
    Off,
}

/// Where agent-authored long-term memory writes can be stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentMemoryWriteMode {
    /// Allow both prompt-visible `MEMORY.md` writes and searchable `memory/*.md` notes.
    #[default]
    Hybrid,
    /// Restrict writes to prompt-visible `MEMORY.md`.
    PromptOnly,
    /// Restrict writes to searchable `memory/*.md` notes.
    SearchOnly,
    /// Disable agent-authored memory writes entirely.
    Off,
}

/// How Moltis writes the managed `USER.md` profile surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UserProfileWriteMode {
    /// Allow both explicit settings saves and silent browser/channel enrichment.
    #[default]
    ExplicitAndAuto,
    /// Allow explicit settings saves, but disable silent browser/channel enrichment.
    ExplicitOnly,
    /// Do not write `USER.md`; keep user profile only in `moltis.toml`.
    Off,
}

impl UserProfileWriteMode {
    #[must_use]
    pub fn allows_explicit_write(self) -> bool {
        !matches!(self, Self::Off)
    }

    #[must_use]
    pub fn allows_auto_write(self) -> bool {
        matches!(self, Self::ExplicitAndAuto)
    }
}

/// Citation mode for memory search results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryCitationsMode {
    /// Always include citations in memory search results.
    On,
    /// Never include citations in memory search results.
    Off,
    /// Include citations when results come from multiple files.
    #[default]
    Auto,
}

/// Embedding provider for memory/RAG features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryProvider {
    /// Built-in local GGUF embeddings.
    Local,
    /// Ollama embedding API.
    Ollama,
    /// OpenAI embedding API.
    #[serde(rename = "openai")]
    OpenAi,
    /// Generic OpenAI-compatible endpoint.
    Custom,
}

/// Strategy for merging keyword and vector search results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemorySearchMergeStrategy {
    /// Reciprocal rank fusion.
    #[default]
    Rrf,
    /// Linear blend of raw keyword and vector scores.
    Linear,
}

/// Backend implementation for long-term memory search and retrieval.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryBackend {
    /// Built-in SQLite-backed indexer and retriever.
    #[default]
    Builtin,
    /// External QMD CLI-backed index and search runtime.
    Qmd,
}

/// How chat sessions are exported into searchable memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionExportMode {
    /// Do not export session transcripts.
    Off,
    /// Export transcripts when the session is rolled with `/new` or `/reset`.
    #[default]
    OnNewOrReset,
}

fn default_session_export_mode() -> SessionExportMode {
    SessionExportMode::OnNewOrReset
}

fn deserialize_session_export_mode<'de, D>(deserializer: D) -> Result<SessionExportMode, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum SessionExportModeRepr {
        Mode(SessionExportMode),
        LegacyBool(bool),
    }

    Ok(match SessionExportModeRepr::deserialize(deserializer)? {
        SessionExportModeRepr::Mode(mode) => mode,
        SessionExportModeRepr::LegacyBool(enabled) => {
            if enabled {
                SessionExportMode::OnNewOrReset
            } else {
                SessionExportMode::Off
            }
        },
    })
}

/// QMD backend configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QmdConfig {
    /// Path to the qmd binary (default: "qmd").
    pub command: Option<String>,
    /// Named collections with paths and glob patterns.
    #[serde(default)]
    pub collections: HashMap<String, QmdCollection>,
    /// Maximum results to retrieve.
    pub max_results: Option<usize>,
    /// Search timeout in milliseconds.
    pub timeout_ms: Option<u64>,
}

/// A QMD collection configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QmdCollection {
    /// Paths to include in this collection.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Glob patterns to filter files.
    #[serde(default)]
    pub globs: Vec<String>,
}
