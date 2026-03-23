use std::{convert::Infallible, fmt, path::PathBuf, str::FromStr};

/// Citation mode for memory search results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CitationMode {
    /// Always include citations in search results.
    On,
    /// Never include citations.
    Off,
    /// Auto: include citations when results come from multiple files.
    #[default]
    Auto,
}

impl FromStr for CitationMode {
    type Err = Infallible;

    /// Parse from string (case-insensitive). Never fails - defaults to Auto.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "on" | "true" | "yes" | "always" => Self::On,
            "off" | "false" | "no" | "never" => Self::Off,
            _ => Self::Auto,
        })
    }
}

/// Strategy for merging vector and keyword search results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Reciprocal Rank Fusion — rank-based, score-magnitude-agnostic.
    #[default]
    Rrf,
    /// Linear weighted blend of raw scores.
    Linear,
}

impl FromStr for MergeStrategy {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "linear" => Self::Linear,
            _ => Self::Rrf,
        })
    }
}

impl fmt::Display for MergeStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rrf => f.write_str("rrf"),
            Self::Linear => f.write_str("linear"),
        }
    }
}

/// Configuration for the memory subsystem.
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Path to the SQLite database file (or `:memory:` for tests).
    pub db_path: String,
    /// Root data directory for writing memory files (e.g. `~/.moltis/`).
    /// Required for `MemoryWriter` support. `None` disables writes.
    pub data_dir: Option<PathBuf>,
    /// Directories to scan for markdown files.
    pub memory_dirs: Vec<PathBuf>,
    /// Target chunk size in tokens (approximate, counted as whitespace-split words).
    pub chunk_size: usize,
    /// Overlap between consecutive chunks in tokens.
    pub chunk_overlap: usize,
    /// Weight for vector similarity in hybrid search (0.0–1.0).
    pub vector_weight: f32,
    /// Weight for keyword/FTS similarity in hybrid search (0.0–1.0).
    pub keyword_weight: f32,
    /// Path to a local GGUF model file for offline embeddings.
    /// If `None`, the default model path will be used when `local-embeddings` feature is enabled.
    pub local_model_path: Option<PathBuf>,
    /// Whether to enable batch embedding via the OpenAI batch API (opt-in).
    pub batch_embeddings: bool,
    /// Minimum number of texts before switching to batch API (default: 50).
    pub batch_threshold: usize,
    /// Citation mode for search results.
    pub citations: CitationMode,
    /// Whether to enable LLM reranking for hybrid search results.
    pub llm_reranking: bool,
    /// Strategy for merging vector and keyword search results.
    pub merge_strategy: MergeStrategy,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            db_path: "memory.db".into(),
            data_dir: None,
            memory_dirs: vec![PathBuf::from("memory")],
            chunk_size: 400,
            chunk_overlap: 80,
            vector_weight: 0.7,
            keyword_weight: 0.3,
            local_model_path: None,
            batch_embeddings: false,
            batch_threshold: 50,
            citations: CitationMode::default(),
            llm_reranking: false,
            merge_strategy: MergeStrategy::default(),
        }
    }
}
