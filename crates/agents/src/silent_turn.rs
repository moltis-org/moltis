/// Silent agentic turn for pre-compaction memory flush.
///
/// Before compacting a session, runs a hidden LLM turn that reviews the conversation
/// and writes important memories to disk. The LLM's response text is discarded (not
/// shown to the user). This matches OpenClaw's approach to long-term memory creation.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use {
    anyhow::Result,
    tracing::{debug, info, warn},
};

use crate::{
    model::{ChatMessage, LlmProvider},
    runner::run_agent_loop,
    tool_registry::{AgentTool, ToolRegistry},
};

const MEMORY_FLUSH_SYSTEM_PROMPT: &str = r#"You are a memory management agent. Your job is to review the conversation below and save any important information to memory files using the write_file tool.

Save information that would be useful in future conversations:
- User preferences and working style
- Key decisions and their reasoning
- Project context, architecture choices, and conventions
- Important facts, names, dates, and relationships
- Recurring topics or patterns
- Technical setup details (tools, languages, frameworks)

Write to these paths:
- `MEMORY.md` — Long-term facts and preferences (append new content, don't overwrite existing)
- `memory/YYYY-MM-DD.md` — Daily session log with what was done and decided today

Format files as clean Markdown. Be concise but preserve important context.
Do NOT respond to the user. Only use the write_file tool to save memories."#;

/// A restricted write_file tool for the silent memory turn.
struct MemoryWriteFileTool {
    workspace_dir: PathBuf,
    written_paths: std::sync::Mutex<Vec<PathBuf>>,
}

impl MemoryWriteFileTool {
    fn new(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir,
            written_paths: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn take_written_paths(&self) -> Vec<PathBuf> {
        std::mem::take(&mut *self.written_paths.lock().unwrap())
    }
}

#[async_trait::async_trait]
impl AgentTool for MemoryWriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file. Use this to save important memories and context."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path relative to workspace (e.g. 'MEMORY.md' or 'memory/2024-01-15.md')"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                },
                "append": {
                    "type": "boolean",
                    "description": "If true, append to the file instead of overwriting",
                    "default": false
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let path_str = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'path' parameter"))?;
        let content = params["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'content' parameter"))?;
        let append = params["append"].as_bool().unwrap_or(false);

        // Resolve relative to workspace
        let full_path = self.workspace_dir.join(path_str);

        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        if append && full_path.exists() {
            let existing = tokio::fs::read_to_string(&full_path)
                .await
                .unwrap_or_default();
            let combined = format!("{existing}\n\n{content}");
            tokio::fs::write(&full_path, combined).await?;
        } else {
            tokio::fs::write(&full_path, content).await?;
        }

        self.written_paths.lock().unwrap().push(full_path.clone());

        debug!(path = %full_path.display(), "silent memory turn: wrote file");
        Ok(serde_json::json!({ "ok": true, "path": full_path.to_string_lossy() }))
    }
}

/// Run a silent memory turn before compaction.
///
/// Gives the LLM a special system prompt asking it to save important memories
/// from the conversation using `write_file`. The LLM's response text is discarded.
///
/// Returns the list of file paths that were written.
pub async fn run_silent_memory_turn(
    provider: Arc<dyn LlmProvider>,
    conversation: &[ChatMessage],
    workspace_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let write_tool = Arc::new(MemoryWriteFileTool::new(workspace_dir.to_path_buf()));

    let mut tools = ToolRegistry::new();
    // We need to register a non-Arc version. Use a wrapper.
    struct ToolWrapper(Arc<MemoryWriteFileTool>);

    #[async_trait::async_trait]
    impl AgentTool for ToolWrapper {
        fn name(&self) -> &str {
            self.0.name()
        }

        fn description(&self) -> &str {
            self.0.description()
        }

        fn parameters_schema(&self) -> serde_json::Value {
            self.0.parameters_schema()
        }

        async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
            self.0.execute(params).await
        }
    }

    tools.register(Box::new(ToolWrapper(Arc::clone(&write_tool))));

    // Format the conversation for the user message
    let mut conversation_text = String::new();
    for msg in conversation {
        let (role, content) = match msg {
            ChatMessage::System { content } => ("system", content.as_str()),
            ChatMessage::User {
                content: crate::model::UserContent::Text(t),
            } => ("user", t.as_str()),
            ChatMessage::User {
                content: crate::model::UserContent::Multimodal(_),
            } => ("user", "[multimodal content]"),
            ChatMessage::Assistant { content, .. } => {
                ("assistant", content.as_deref().unwrap_or(""))
            },
            ChatMessage::Tool { content, .. } => ("tool", content.as_str()),
        };
        // Skip very long messages (tool results, etc.)
        let truncated = if content.len() > 2000 {
            &content[..2000]
        } else {
            content
        };
        conversation_text.push_str(&format!("{role}: {truncated}\n\n"));
    }

    info!(
        messages = conversation.len(),
        "running silent memory turn before compaction"
    );

    let user_content = crate::model::UserContent::Text(conversation_text);
    let result = run_agent_loop(
        provider,
        &tools,
        MEMORY_FLUSH_SYSTEM_PROMPT,
        &user_content,
        None, // no event callbacks — silent
        None, // no history
    )
    .await;

    match result {
        Ok(run_result) => {
            let paths = write_tool.take_written_paths();
            info!(
                files_written = paths.len(),
                tool_calls = run_result.tool_calls_made,
                "silent memory turn complete"
            );
            Ok(paths)
        },
        Err(e) => {
            warn!(error = %e, "silent memory turn failed");
            Ok(Vec::new()) // Don't fail compaction if memory flush fails
        },
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::model::{ChatMessage, CompletionResponse, StreamEvent, ToolCall, Usage},
        std::pin::Pin,
        tokio_stream::Stream,
    };

    /// Mock provider that makes one write_file call then returns.
    struct MemoryWritingProvider {
        call_count: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl LlmProvider for MemoryWritingProvider {
        fn name(&self) -> &str {
            "mock"
        }

        fn id(&self) -> &str {
            "mock-model"
        }

        fn supports_tools(&self) -> bool {
            true
        }

        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[serde_json::Value],
        ) -> Result<CompletionResponse> {
            let count = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if count == 0 {
                Ok(CompletionResponse {
                    text: None,
                    tool_calls: vec![ToolCall {
                        id: "call_1".into(),
                        name: "write_file".into(),
                        arguments: serde_json::json!({
                            "path": "MEMORY.md",
                            "content": "# Memories\n\nUser prefers Rust over Python."
                        }),
                    }],
                    usage: Usage {
                        input_tokens: 100,
                        output_tokens: 50,
                        ..Default::default()
                    },
                })
            } else {
                Ok(CompletionResponse {
                    text: Some("NO_REPLY".into()),
                    tool_calls: vec![],
                    usage: Usage {
                        input_tokens: 50,
                        output_tokens: 5,
                        ..Default::default()
                    },
                })
            }
        }

        fn stream(
            &self,
            _messages: Vec<ChatMessage>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            Box::pin(tokio_stream::empty())
        }
    }

    #[tokio::test]
    async fn test_silent_memory_turn_writes_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let provider = Arc::new(MemoryWritingProvider {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });

        let conversation = vec![
            ChatMessage::user("I prefer Rust over Python."),
            ChatMessage::assistant("Noted! Rust is a great choice."),
        ];

        let paths = run_silent_memory_turn(provider, &conversation, tmp.path())
            .await
            .unwrap();

        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("MEMORY.md"));

        let content = std::fs::read_to_string(&paths[0]).unwrap();
        assert!(content.contains("Rust"));
        assert!(content.contains("Python"));
    }

    #[tokio::test]
    async fn test_silent_memory_turn_no_crash_on_empty_conversation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let provider = Arc::new(MemoryWritingProvider {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });

        let paths = run_silent_memory_turn(provider, &[], tmp.path())
            .await
            .unwrap();

        // Should succeed even with empty conversation (provider still writes)
        assert!(!paths.is_empty());
    }
}
