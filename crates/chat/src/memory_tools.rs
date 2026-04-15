//! Agent-scoped memory tools (search, get, save) and memory writer.

use std::{path::Path, sync::Arc};

use {async_trait::async_trait, serde_json::Value, tracing::warn};

use {
    moltis_agents::tool_registry::{AgentTool, ToolRegistry},
    moltis_config::{AgentMemoryWriteMode, MemoryStyle, ToolMode},
};

use crate::types::{
    default_agent_memory_file_for_mode, memory_style_allows_tools, memory_write_mode_allows_save,
    validate_agent_memory_target_for_mode,
};

pub(crate) const MAX_AGENT_MEMORY_WRITE_BYTES: usize = 50 * 1024;
pub(crate) const MEMORY_SEARCH_FETCH_MULTIPLIER: usize = 8;
pub(crate) const MEMORY_SEARCH_MIN_FETCH: usize = 25;

pub(crate) fn is_valid_agent_memory_leaf_name(name: &str) -> bool {
    if name.is_empty() || name.contains('/') || !name.ends_with(".md") {
        return false;
    }
    if name.chars().any(char::is_whitespace) {
        return false;
    }
    let stem = &name[..name.len() - 3];
    !(stem.is_empty() || stem.starts_with('.'))
}

pub(crate) fn resolve_agent_memory_target_path(
    agent_id: &str,
    file: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let trimmed = file.trim();
    if trimmed.is_empty() {
        anyhow::bail!("memory path cannot be empty");
    }

    let workspace = moltis_config::agent_workspace_dir(agent_id);
    if trimmed == "MEMORY.md" || trimmed == "memory.md" {
        return Ok(workspace.join("MEMORY.md"));
    }

    let Some(name) = trimmed.strip_prefix("memory/") else {
        anyhow::bail!(
            "invalid memory path '{trimmed}': allowed targets are MEMORY.md, memory.md, or memory/<name>.md"
        );
    };
    if !is_valid_agent_memory_leaf_name(name) {
        anyhow::bail!(
            "invalid memory path '{trimmed}': allowed targets are MEMORY.md, memory.md, or memory/<name>.md"
        );
    }
    Ok(workspace.join("memory").join(name))
}

pub(crate) fn is_path_in_agent_memory_scope(path: &Path, agent_id: &str) -> bool {
    let workspace = moltis_config::agent_workspace_dir(agent_id);
    let workspace_memory_dir = workspace.join("memory");
    if path == workspace.join("MEMORY.md")
        || path == workspace.join("memory.md")
        || path.starts_with(&workspace_memory_dir)
    {
        return true;
    }

    if agent_id != "main" {
        return false;
    }

    let data_dir = moltis_config::data_dir();
    let root_memory_dir = data_dir.join("memory");
    path == data_dir.join("MEMORY.md")
        || path == data_dir.join("memory.md")
        || path.starts_with(&root_memory_dir)
}

pub(crate) struct AgentScopedMemoryWriter {
    manager: moltis_memory::runtime::DynMemoryRuntime,
    agent_id: String,
    write_mode: AgentMemoryWriteMode,
    checkpoints: moltis_tools::checkpoints::CheckpointManager,
}

impl AgentScopedMemoryWriter {
    pub fn new(
        manager: moltis_memory::runtime::DynMemoryRuntime,
        agent_id: String,
        write_mode: AgentMemoryWriteMode,
    ) -> Self {
        Self {
            manager,
            agent_id,
            write_mode,
            checkpoints: moltis_tools::checkpoints::CheckpointManager::new(
                moltis_config::data_dir(),
            ),
        }
    }
}

#[async_trait]
impl moltis_agents::memory_writer::MemoryWriter for AgentScopedMemoryWriter {
    async fn write_memory(
        &self,
        file: &str,
        content: &str,
        append: bool,
    ) -> anyhow::Result<moltis_agents::memory_writer::MemoryWriteResult> {
        if content.len() > MAX_AGENT_MEMORY_WRITE_BYTES {
            anyhow::bail!(
                "content exceeds maximum size of {} bytes ({} bytes provided)",
                MAX_AGENT_MEMORY_WRITE_BYTES,
                content.len()
            );
        }

        validate_agent_memory_target_for_mode(self.write_mode, file)?;
        let path = resolve_agent_memory_target_path(&self.agent_id, file)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let checkpoint = self
            .checkpoints
            .checkpoint_path(&path, "memory_write")
            .await?;
        let final_content = if append && tokio::fs::try_exists(&path).await? {
            let existing = tokio::fs::read_to_string(&path).await?;
            format!("{existing}\n\n{content}")
        } else {
            content.to_string()
        };
        let bytes_written = final_content.len();

        tokio::fs::write(&path, &final_content).await?;
        if let Err(error) = self.manager.sync_path(&path).await {
            warn!(path = %path.display(), %error, "agent memory write re-index failed");
        }

        Ok(moltis_agents::memory_writer::MemoryWriteResult {
            location: path.to_string_lossy().into_owned(),
            bytes_written,
            checkpoint_id: Some(checkpoint.id),
        })
    }
}

struct AgentScopedMemorySearchTool {
    manager: moltis_memory::runtime::DynMemoryRuntime,
    agent_id: String,
}

impl AgentScopedMemorySearchTool {
    fn new(manager: moltis_memory::runtime::DynMemoryRuntime, agent_id: String) -> Self {
        Self { manager, agent_id }
    }
}

#[async_trait]
impl AgentTool for AgentScopedMemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search agent memory using hybrid vector + keyword search. Returns relevant chunks from daily logs and long-term memory files."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;
        let requested_limit = params.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize;
        let limit = requested_limit.clamp(1, 50);
        let search_limit = limit
            .saturating_mul(MEMORY_SEARCH_FETCH_MULTIPLIER)
            .max(MEMORY_SEARCH_MIN_FETCH)
            .max(limit);

        let mut results: Vec<moltis_memory::search::SearchResult> = self
            .manager
            .search(query, search_limit)
            .await?
            .into_iter()
            .filter(|result| is_path_in_agent_memory_scope(Path::new(&result.path), &self.agent_id))
            .collect();
        results.truncate(limit);

        let include_citations = moltis_memory::search::SearchResult::should_include_citations(
            &results,
            self.manager.citation_mode(),
        );
        let items: Vec<Value> = results
            .iter()
            .map(|result| {
                let text = if include_citations {
                    result.text_with_citation()
                } else {
                    result.text.clone()
                };
                serde_json::json!({
                    "chunk_id": result.chunk_id,
                    "path": result.path,
                    "source": result.source,
                    "start_line": result.start_line,
                    "end_line": result.end_line,
                    "score": result.score,
                    "text": text,
                    "citation": format!("{}#{}", result.path, result.start_line),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "results": items,
            "citations_enabled": include_citations
        }))
    }
}

struct AgentScopedMemoryGetTool {
    manager: moltis_memory::runtime::DynMemoryRuntime,
    agent_id: String,
}

impl AgentScopedMemoryGetTool {
    fn new(manager: moltis_memory::runtime::DynMemoryRuntime, agent_id: String) -> Self {
        Self { manager, agent_id }
    }
}

#[async_trait]
impl AgentTool for AgentScopedMemoryGetTool {
    fn name(&self) -> &str {
        "memory_get"
    }

    fn description(&self) -> &str {
        "Retrieve a specific memory chunk by its ID. Use this to get the full text of a chunk found via memory_search."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "chunk_id": {
                    "type": "string",
                    "description": "The chunk ID to retrieve"
                }
            },
            "required": ["chunk_id"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let chunk_id = params
            .get("chunk_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing 'chunk_id' parameter"))?;

        match self.manager.get_chunk(chunk_id).await? {
            Some(chunk)
                if is_path_in_agent_memory_scope(Path::new(&chunk.path), &self.agent_id) =>
            {
                Ok(serde_json::json!({
                    "chunk_id": chunk.id,
                    "path": chunk.path,
                    "source": chunk.source,
                    "start_line": chunk.start_line,
                    "end_line": chunk.end_line,
                    "text": chunk.text,
                }))
            },
            _ => Ok(serde_json::json!({
                "error": "chunk not found",
                "chunk_id": chunk_id,
            })),
        }
    }
}

struct AgentScopedMemorySaveTool {
    writer: AgentScopedMemoryWriter,
    write_mode: AgentMemoryWriteMode,
}

impl AgentScopedMemorySaveTool {
    fn new(
        manager: moltis_memory::runtime::DynMemoryRuntime,
        agent_id: String,
        write_mode: AgentMemoryWriteMode,
    ) -> Self {
        Self {
            writer: AgentScopedMemoryWriter::new(manager, agent_id, write_mode),
            write_mode,
        }
    }
}

#[async_trait]
impl AgentTool for AgentScopedMemorySaveTool {
    fn name(&self) -> &str {
        "memory_save"
    }

    fn description(&self) -> &str {
        "Save content to long-term memory. Writes to MEMORY.md or memory/<name>.md. Content persists across sessions and is searchable via memory_search."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The content to save to memory"
                },
                "file": {
                    "type": "string",
                    "description": "Target file: MEMORY.md, memory.md, or memory/<name>.md",
                    "default": "MEMORY.md"
                },
                "append": {
                    "type": "boolean",
                    "description": "Append to existing file (true) or overwrite (false)",
                    "default": true
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let content = params
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing 'content' parameter"))?;
        let file = params
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or_else(|| default_agent_memory_file_for_mode(self.write_mode));
        let append = params
            .get("append")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        use moltis_agents::memory_writer::MemoryWriter;
        let result = self.writer.write_memory(file, content, append).await?;

        Ok(serde_json::json!({
            "saved": true,
            "path": file,
            "bytes_written": result.bytes_written,
            "checkpointId": result.checkpoint_id,
        }))
    }
}

pub(crate) fn install_agent_scoped_memory_tools(
    registry: &mut ToolRegistry,
    manager: &moltis_memory::runtime::DynMemoryRuntime,
    agent_id: &str,
    style: MemoryStyle,
    write_mode: AgentMemoryWriteMode,
) {
    let had_search = registry.unregister("memory_search");
    let had_get = registry.unregister("memory_get");
    let had_save = registry.unregister("memory_save");

    if !memory_style_allows_tools(style) {
        return;
    }

    let agent_id_owned = agent_id.to_string();
    if had_search {
        registry.register(Box::new(AgentScopedMemorySearchTool::new(
            Arc::clone(manager),
            agent_id_owned.clone(),
        )));
    }
    if had_get {
        registry.register(Box::new(AgentScopedMemoryGetTool::new(
            Arc::clone(manager),
            agent_id_owned.clone(),
        )));
    }
    if had_save && memory_write_mode_allows_save(write_mode) {
        registry.register(Box::new(AgentScopedMemorySaveTool::new(
            Arc::clone(manager),
            agent_id_owned,
            write_mode,
        )));
    }
}

/// Resolve the effective tool mode for a provider.
///
/// Combines the provider's `tool_mode()` override with its `supports_tools()`
/// capability to determine how tools should be dispatched:
/// - `Native` -- provider handles tool schemas via API (OpenAI function calling, etc.)
/// - `Text` -- tools are described in the prompt; the runner parses tool calls from text
/// - `Off` -- no tools at all
pub(crate) fn effective_tool_mode(provider: &dyn moltis_agents::model::LlmProvider) -> ToolMode {
    match provider.tool_mode() {
        Some(ToolMode::Native) => ToolMode::Native,
        Some(ToolMode::Text) => ToolMode::Text,
        Some(ToolMode::Off) => ToolMode::Off,
        Some(ToolMode::Auto) | None => {
            if provider.supports_tools() {
                ToolMode::Native
            } else {
                ToolMode::Text
            }
        },
    }
}
