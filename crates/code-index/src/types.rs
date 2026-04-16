//! Core types for codebase indexing.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Programming language hint derived from file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    C,
    Cpp,
    CSharp,
    Css,
    Go,
    Html,
    Java,
    JavaScript,
    Json,
    Kotlin,
    Markdown,
    Nix,
    Php,
    Python,
    Ruby,
    Rust,
    Scala,
    Shell,
    Sql,
    Swift,
    Toml,
    TypeScript,
    Yaml,
    Zig,
    /// Unknown or unsupported language.
    Unknown,
}

impl Language {
    /// Derive the language from a file extension (without the leading dot).
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "sh" | "bash" => Self::Shell,
            "c" => Self::C,
            "cpp" | "cxx" | "cc" | "hpp" | "h" => Self::Cpp,
            "cs" => Self::CSharp,
            "css" | "scss" | "less" => Self::Css,
            "go" => Self::Go,
            "html" | "htm" => Self::Html,
            "java" => Self::Java,
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "json" => Self::Json,
            "kt" | "kts" => Self::Kotlin,
            "md" | "markdown" | "mdx" => Self::Markdown,
            "nix" => Self::Nix,
            "php" => Self::Php,
            "py" | "pyi" => Self::Python,
            "rb" => Self::Ruby,
            "rs" => Self::Rust,
            "scala" => Self::Scala,
            "sql" => Self::Sql,
            "swift" => Self::Swift,
            "toml" => Self::Toml,
            "ts" | "tsx" | "mts" | "cts" => Self::TypeScript,
            "yaml" | "yml" => Self::Yaml,
            "zig" => Self::Zig,
            _ => Self::Unknown,
        }
    }

    /// Derive the language from a file path by inspecting its extension.
    pub fn from_path(path: &std::path::Path) -> Self {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        Self::from_extension(ext)
    }

    /// Return the file extension typically associated with this language.
    /// Used primarily for display and debugging.
    pub fn primary_extension(&self) -> &'static str {
        match self {
            Self::Shell => "sh",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "cs",
            Self::Css => "css",
            Self::Go => "go",
            Self::Html => "html",
            Self::Java => "java",
            Self::JavaScript => "js",
            Self::Json => "json",
            Self::Kotlin => "kt",
            Self::Markdown => "md",
            Self::Nix => "nix",
            Self::Php => "php",
            Self::Python => "py",
            Self::Ruby => "rb",
            Self::Rust => "rs",
            Self::Scala => "scala",
            Self::Sql => "sql",
            Self::Swift => "swift",
            Self::Toml => "toml",
            Self::TypeScript => "ts",
            Self::Yaml => "yaml",
            Self::Zig => "zig",
            Self::Unknown => "txt",
        }
    }
}

/// A file discovered in a git repository, after filtering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Absolute path to the file on disk.
    pub path: PathBuf,
    /// Relative path from the repository root.
    pub relative_path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// SHA-256 content hash.
    pub content_hash: String,
    /// Detected language.
    pub language: Language,
    /// Whether the file is tracked by git.
    pub git_tracked: bool,
    /// Epoch millis when this file was last indexed (0 if never).
    pub last_indexed: u64,
}

/// A file that passed all filters and is ready for indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilteredFile {
    /// Absolute path to the file on disk.
    pub path: PathBuf,
    /// Relative path from the repository root.
    pub relative_path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Detected language.
    pub language: Language,
}

/// A chunk of code discovered during indexing (metadata only, no embedding).
///
/// This is distinct from [`crate::store::CodeChunk`] which is the storage-level
/// representation with embedding support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredChunk {
    /// Unique identifier: `{relative_path}:{start_line}:{end_line}`.
    pub id: String,
    /// Relative path from the repository root.
    pub relative_path: PathBuf,
    /// 1-based start line.
    pub start_line: usize,
    /// 1-based end line (inclusive).
    pub end_line: usize,
    /// The chunk text content.
    pub text: String,
    /// Detected language of the source file.
    pub language: Language,
    /// SHA-256 hash of the chunk text (for cache discrimination).
    pub content_hash: String,
}

/// Status of the code index for a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStatus {
    /// Project identifier.
    pub project_id: String,
    /// Total number of tracked, filtered files.
    pub total_files: usize,
    /// Total number of chunks across all indexed files.
    pub total_chunks: usize,
    /// Epoch millis of the last successful full sync.
    pub last_sync_ms: Option<u64>,
    /// Embedding model in use (if any).
    pub embedding_model: Option<String>,
    /// Index backend in use (e.g. "qmd", "builtin").
    pub backend: String,
}

/// A search result from the code index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Chunk identifier.
    pub chunk_id: String,
    /// Relative path from the repository root.
    pub path: String,
    /// 1-based start line.
    pub start_line: usize,
    /// 1-based end line (inclusive).
    pub end_line: usize,
    /// Relevance score (0.0–1.0, higher is better).
    pub score: f32,
    /// Matched text content.
    pub text: String,
    /// Source of the result (e.g. "qmd", "builtin").
    pub source: String,
}
