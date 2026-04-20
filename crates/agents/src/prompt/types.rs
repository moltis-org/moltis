use serde::Serialize;

/// Broad model family classification, used to tune text-based tool prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    Llama,
    Qwen,
    Mistral,
    DeepSeek,
    Gemma,
    Phi,
    Unknown,
}

impl ModelFamily {
    /// Detect the model family from a model identifier string.
    #[must_use]
    pub fn from_model_id(id: &str) -> Self {
        let lower = id.to_ascii_lowercase();
        if lower.contains("llama") {
            Self::Llama
        } else if lower.contains("qwen") {
            Self::Qwen
        } else if lower.contains("mistral") || lower.contains("mixtral") {
            Self::Mistral
        } else if lower.contains("deepseek") {
            Self::DeepSeek
        } else if lower.contains("gemma") {
            Self::Gemma
        } else if lower.contains("phi") {
            Self::Phi
        } else {
            Self::Unknown
        }
    }
}

/// Runtime context for the host process running the current agent turn.
#[derive(Debug, Clone, Default)]
pub struct PromptHostRuntimeContext {
    pub host: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub shell: Option<String>,
    pub time: Option<String>,
    pub today: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub session_key: Option<String>,
    pub surface: Option<String>,
    pub session_kind: Option<String>,
    pub channel_type: Option<String>,
    pub channel_account_id: Option<String>,
    pub channel_chat_id: Option<String>,
    pub channel_chat_type: Option<String>,
    pub channel_sender_id: Option<String>,
    pub data_dir: Option<String>,
    pub sudo_non_interactive: Option<bool>,
    pub sudo_status: Option<String>,
    pub timezone: Option<String>,
    pub accept_language: Option<String>,
    pub remote_ip: Option<String>,
    pub location: Option<String>,
}

/// Runtime context for sandbox execution routing used by the `exec` tool.
#[derive(Debug, Clone, Default)]
pub struct PromptSandboxRuntimeContext {
    pub exec_sandboxed: bool,
    pub mode: Option<String>,
    pub backend: Option<String>,
    pub scope: Option<String>,
    pub image: Option<String>,
    pub home: Option<String>,
    pub workspace_mount: Option<String>,
    pub workspace_path: Option<String>,
    pub no_network: Option<bool>,
    pub session_override: Option<bool>,
}

/// Info about a single connected remote node, injected into the system prompt.
#[derive(Debug, Clone)]
pub struct PromptNodeInfo {
    pub node_id: String,
    pub display_name: Option<String>,
    pub platform: String,
    pub capabilities: Vec<String>,
    pub cpu_count: Option<u32>,
    pub mem_total: Option<u64>,
    pub runtimes: Vec<String>,
    pub providers: Vec<(String, Vec<String>)>,
}

/// Runtime context about connected remote nodes.
#[derive(Debug, Clone, Default)]
pub struct PromptNodesRuntimeContext {
    pub nodes: Vec<PromptNodeInfo>,
    pub default_node_id: Option<String>,
}

/// Combined runtime context injected into the system prompt.
#[derive(Debug, Clone, Default)]
pub struct PromptRuntimeContext {
    pub host: PromptHostRuntimeContext,
    pub sandbox: Option<PromptSandboxRuntimeContext>,
    pub nodes: Option<PromptNodesRuntimeContext>,
}

#[derive(Debug, Clone, Copy)]
pub struct PromptBuildLimits {
    pub workspace_file_max_chars: usize,
    /// Inject skill self-improvement guidance into the system prompt.
    pub enable_skill_self_improvement: bool,
}

impl Default for PromptBuildLimits {
    fn default() -> Self {
        Self {
            workspace_file_max_chars: DEFAULT_WORKSPACE_FILE_MAX_CHARS,
            enable_skill_self_improvement: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceFilePromptStatus {
    pub name: String,
    pub original_chars: usize,
    pub included_chars: usize,
    pub limit_chars: usize,
    pub truncated_chars: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PromptBuildMetadata {
    pub workspace_files: Vec<WorkspaceFilePromptStatus>,
}

impl PromptBuildMetadata {
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.workspace_files.iter().any(|file| file.truncated)
    }
}

#[derive(Debug, Clone)]
pub struct PromptBuildOutput {
    pub prompt: String,
    pub metadata: PromptBuildMetadata,
}

/// Suffix appended to the system prompt when the user's reply medium is voice.
pub const VOICE_REPLY_SUFFIX: &str = "\n\n\
## Voice Reply Mode\n\n\
The user is speaking to you via voice messages. Their messages are transcribed from \
speech-to-text, so treat this as a spoken conversation. You will hear their words as \
text, and your response will be converted to spoken audio for them.\n\n\
Write for speech, not for reading:\n\
- Use natural, conversational sentences. No bullet lists, numbered lists, or headings.\n\
- NEVER include raw URLs. Instead describe the resource by name \
(e.g. \"the Rust documentation website\" instead of \"https://doc.rust-lang.org\").\n\
- No markdown formatting: no bold, italic, headers, code fences, or inline backticks.\n\
- Spell out abbreviations that a text-to-speech engine might mispronounce \
(e.g. \"API\" → \"A-P-I\", \"CLI\" → \"C-L-I\").\n\
- Keep responses concise — two to three short paragraphs at most.\n\
- Use complete sentences and natural transitions between ideas.\n";

/// Maximum number of characters from each workspace file (`AGENTS.md`, `TOOLS.md`) injected into the prompt.
pub const DEFAULT_WORKSPACE_FILE_MAX_CHARS: usize = 32_000;
