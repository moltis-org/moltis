//! Agent-scoped memory tools (search, get, save) and memory writer.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    serde_json::{Value, json},
    tokio::sync::RwLock,
    tracing::warn,
};

use {
    moltis_agents::{
        json_repair::repair_json,
        model::{ChatMessage, LlmProvider},
        tool_registry::{AgentTool, ToolRegistry},
    },
    moltis_config::{AgentMemoryWriteMode, MemoryStyle, ToolMode},
    moltis_memory::writer::remove_exact_text,
    moltis_providers::ProviderRegistry,
    moltis_sessions::metadata::SqliteSessionMetadata,
};

use crate::types::{
    default_agent_memory_file_for_mode, memory_style_allows_tools, memory_write_mode_allows_save,
    validate_agent_memory_target_for_mode,
};

pub(crate) const MAX_AGENT_MEMORY_WRITE_BYTES: usize = 50 * 1024;
pub(crate) const MEMORY_SEARCH_FETCH_MULTIPLIER: usize = 8;
pub(crate) const MEMORY_SEARCH_MIN_FETCH: usize = 25;
pub(crate) const MEMORY_FORGET_DEFAULT_LIMIT: usize = 6;
pub(crate) const MEMORY_FORGET_MAX_LIMIT: usize = 12;

#[derive(Clone, Debug)]
struct ForgetCandidate {
    chunk_id: String,
    file: String,
    path: String,
    start_line: i64,
    end_line: i64,
    text: String,
}

#[derive(Debug)]
struct ForgetRequest {
    request: String,
    dry_run: bool,
    limit: usize,
    session_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct ForgetPromptCandidate<'a> {
    chunk_id: &'a str,
    file: &'a str,
    start_line: i64,
    end_line: i64,
    text: &'a str,
}

#[derive(Debug, Deserialize)]
struct ForgetPlan {
    #[serde(default)]
    needs_confirmation: bool,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    actions: Vec<ForgetPlanAction>,
}

#[derive(Debug, Deserialize)]
struct ForgetPlanAction {
    chunk_id: String,
    #[serde(default)]
    reason: String,
}

#[derive(Clone, Debug)]
struct ValidatedForgetAction {
    candidate: ForgetCandidate,
    reason: String,
}

fn count_exact_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    for variant in text_variants(needle) {
        let matches = haystack.match_indices(variant.as_str()).count();
        if matches > 0 {
            return matches;
        }
    }
    0
}

fn text_variants(snippet: &str) -> Vec<String> {
    let mut variants = vec![snippet.to_string()];
    if snippet.contains("\r\n") {
        let lf = snippet.replace("\r\n", "\n");
        if lf != snippet {
            variants.push(lf);
        }
    } else if snippet.contains('\n') {
        variants.push(snippet.replace('\n', "\r\n"));
    }
    variants
}

fn preview_text(text: &str) -> String {
    const PREVIEW_LIMIT: usize = 180;

    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.len() <= PREVIEW_LIMIT {
        return flattened;
    }

    let mut preview = flattened
        .chars()
        .take(PREVIEW_LIMIT.saturating_sub(3))
        .collect::<String>();
    preview.push_str("...");
    preview
}

fn strip_markdown_code_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let Some(end) = rest.rfind("```") else {
        return trimmed;
    };
    let inner = &rest[..end];
    inner
        .split_once('\n')
        .map(|(_, body)| body.trim())
        .unwrap_or(trimmed)
}

fn parse_forget_request(params: &Value) -> anyhow::Result<ForgetRequest> {
    let request = params
        .get("request")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing 'request' parameter"))?
        .trim()
        .to_string();
    if request.is_empty() {
        anyhow::bail!("'request' cannot be empty");
    }

    let requested_limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(MEMORY_FORGET_DEFAULT_LIMIT, |value| value as usize);

    Ok(ForgetRequest {
        request,
        dry_run: params
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        limit: requested_limit.clamp(1, MEMORY_FORGET_MAX_LIMIT),
        session_key: params
            .get("_session_key")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn memory_forget_parameters_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "request": {
                "type": "string",
                "description": "Natural-language description of what saved memory to forget."
            },
            "dry_run": {
                "type": "boolean",
                "description": "Preview what would be deleted without mutating any files.",
                "default": false
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of candidate memory chunks to inspect before planning deletions.",
                "default": MEMORY_FORGET_DEFAULT_LIMIT
            }
        },
        "required": ["request"]
    })
}

fn memory_file_label_from_root(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut components = relative.components();
    let first = components.next()?.as_os_str().to_str()?;

    match first {
        "MEMORY.md" | "memory.md" if components.next().is_none() => Some(first.to_string()),
        "memory" => {
            let leaf = components.next()?.as_os_str().to_str()?;
            if components.next().is_some() || !is_valid_agent_memory_leaf_name(leaf) {
                return None;
            }
            Some(format!("memory/{leaf}"))
        },
        _ => None,
    }
}

fn agent_memory_file_label_for_path(path: &Path, agent_id: &str) -> Option<String> {
    let workspace = moltis_config::agent_workspace_dir(agent_id);
    memory_file_label_from_root(&workspace, path).or_else(|| {
        if agent_id == "main" {
            memory_file_label_from_root(&moltis_config::data_dir(), path)
        } else {
            None
        }
    })
}

fn global_memory_file_label_for_path(
    manager: &dyn moltis_memory::runtime::MemoryRuntime,
    path: &Path,
) -> Option<String> {
    memory_file_label_from_root(manager.data_dir()?, path)
}

fn forget_planned_match_json(candidate: &ForgetCandidate, reason: &str) -> Value {
    json!({
        "chunk_id": candidate.chunk_id,
        "file": candidate.file,
        "start_line": candidate.start_line,
        "end_line": candidate.end_line,
        "reason": reason,
        "text_preview": preview_text(&candidate.text),
    })
}

async fn collect_forget_candidates<F>(
    manager: &moltis_memory::runtime::DynMemoryRuntime,
    request: &str,
    limit: usize,
    mut map_path_to_file: F,
) -> anyhow::Result<Vec<ForgetCandidate>>
where
    F: FnMut(&Path) -> Option<String>,
{
    let search_limit = limit
        .saturating_mul(MEMORY_SEARCH_FETCH_MULTIPLIER)
        .max(MEMORY_SEARCH_MIN_FETCH)
        .max(limit);
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    for result in manager.search(request, search_limit).await? {
        if !seen.insert(result.chunk_id.clone()) {
            continue;
        }

        let Some(chunk) = manager.get_chunk(&result.chunk_id).await? else {
            continue;
        };
        let chunk_path = Path::new(&chunk.path);
        let Some(file) = map_path_to_file(chunk_path) else {
            continue;
        };

        candidates.push(ForgetCandidate {
            chunk_id: chunk.id,
            file,
            path: chunk.path,
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            text: chunk.text,
        });

        if candidates.len() >= limit {
            break;
        }
    }

    Ok(candidates)
}

async fn plan_memory_forget(
    provider: &dyn LlmProvider,
    request: &str,
    candidates: &[ForgetCandidate],
) -> anyhow::Result<ForgetPlan> {
    let prompt_candidates: Vec<ForgetPromptCandidate<'_>> = candidates
        .iter()
        .map(|candidate| ForgetPromptCandidate {
            chunk_id: &candidate.chunk_id,
            file: &candidate.file,
            start_line: candidate.start_line,
            end_line: candidate.end_line,
            text: &candidate.text,
        })
        .collect();

    let candidate_json = serde_json::to_string_pretty(&prompt_candidates)?;
    let messages = vec![
        ChatMessage::system(concat!(
            "You plan safe long-term memory deletions.\n",
            "Return JSON only, no markdown.\n",
            "Schema:\n",
            "{\n",
            "  \"needs_confirmation\": boolean,\n",
            "  \"rationale\": string,\n",
            "  \"actions\": [{ \"chunk_id\": string, \"reason\": string }]\n",
            "}\n",
            "Rules:\n",
            "- Only use chunk_id values from the provided candidates.\n",
            "- Select only chunks whose full text should be deleted exactly as stored.\n",
            "- If the request is ambiguous, stale, or could delete the wrong fact, set needs_confirmation=true.\n",
            "- Prefer zero actions over guessing.\n",
            "- Do not invent files, text, or chunk ids."
        )),
        ChatMessage::user(format!(
            "Forget request:\n{request}\n\nCandidate chunks:\n{candidate_json}"
        )),
    ];

    let response = provider.complete(&messages, &[]).await?;
    let raw = response
        .text
        .ok_or_else(|| anyhow::anyhow!("memory_forget planner returned no text"))?;
    let cleaned = strip_markdown_code_fences(&raw);
    let parsed = serde_json::from_str::<ForgetPlan>(cleaned).or_else(|_| {
        repair_json(cleaned)
            .ok_or_else(|| serde_json::Error::io(std::io::Error::other("repair failed")))
            .and_then(serde_json::from_value)
    })?;
    Ok(parsed)
}

async fn validate_forget_actions(
    actions: &[ForgetPlanAction],
    candidates: &[ForgetCandidate],
) -> anyhow::Result<(Vec<ValidatedForgetAction>, Vec<String>)> {
    let candidate_map: HashMap<&str, &ForgetCandidate> = candidates
        .iter()
        .map(|candidate| (candidate.chunk_id.as_str(), candidate))
        .collect();
    let mut seen = HashSet::new();
    let mut valid = Vec::new();
    let mut issues = Vec::new();
    let mut file_cache: HashMap<String, String> = HashMap::new();

    for action in actions {
        if !seen.insert(action.chunk_id.clone()) {
            continue;
        }

        let Some(candidate) = candidate_map.get(action.chunk_id.as_str()) else {
            issues.push(format!(
                "planner referenced unknown chunk_id '{}'",
                action.chunk_id
            ));
            continue;
        };

        let content = if let Some(existing) = file_cache.get(candidate.path.as_str()) {
            existing.clone()
        } else {
            let loaded = match tokio::fs::read_to_string(&candidate.path).await {
                Ok(loaded) => loaded,
                Err(error) => {
                    issues.push(format!(
                        "could not read file for chunk '{}': {error}",
                        candidate.chunk_id
                    ));
                    continue;
                },
            };
            file_cache.insert(candidate.path.clone(), loaded.clone());
            loaded
        };

        let match_count = count_exact_occurrences(&content, &candidate.text);
        if match_count != 1 {
            issues.push(format!(
                "chunk '{}' in '{}' is not uniquely deletable ({} exact matches)",
                candidate.chunk_id, candidate.file, match_count
            ));
            continue;
        }

        valid.push(ValidatedForgetAction {
            candidate: (*candidate).clone(),
            reason: action.reason.trim().to_string(),
        });
    }

    Ok((valid, issues))
}

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
        return Ok(workspace.join(trimmed));
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

pub struct AgentScopedMemoryWriter {
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

    async fn checkpoint_memory_path(
        &self,
        file: &str,
        reason: &str,
    ) -> anyhow::Result<(
        std::path::PathBuf,
        moltis_tools::checkpoints::CheckpointRecord,
    )> {
        validate_agent_memory_target_for_mode(self.write_mode, file)?;
        let path = resolve_agent_memory_target_path(&self.agent_id, file)?;
        let checkpoint = self.checkpoints.checkpoint_path(&path, reason).await?;
        Ok((path, checkpoint))
    }

    async fn delete_memory(
        &self,
        file: &str,
        text: Option<&str>,
        delete_file: bool,
        all_matches: bool,
        delete_if_empty: bool,
    ) -> anyhow::Result<AgentScopedMemoryDeleteResult> {
        if delete_file == text.is_some() {
            anyhow::bail!("provide either 'text' or delete_file=true");
        }

        let (path, checkpoint) = self.checkpoint_memory_path(file, "memory_delete").await?;

        if delete_file {
            let file_existed = tokio::fs::try_exists(&path).await?;
            if file_existed {
                tokio::fs::remove_file(&path).await?;
            }
            let index_removed = self.manager.remove_path(&path).await?;
            return Ok(AgentScopedMemoryDeleteResult {
                file_deleted: file_existed,
                file_existed,
                matches_removed: 0,
                bytes_written: 0,
                index_removed,
                checkpoint_id: checkpoint.id,
            });
        }

        let existing = tokio::fs::read_to_string(&path).await.map_err(|error| {
            anyhow::anyhow!("failed to read memory file '{}': {error}", path.display())
        })?;
        let removal = remove_exact_text(&existing, text.unwrap_or_default(), all_matches)?;
        let file_deleted = delete_if_empty && removal.content.trim().is_empty();

        let index_removed = if file_deleted {
            tokio::fs::remove_file(&path).await?;
            self.manager.remove_path(&path).await?
        } else {
            tokio::fs::write(&path, &removal.content).await?;
            self.manager.sync_path(&path).await?;
            false
        };

        Ok(AgentScopedMemoryDeleteResult {
            file_deleted,
            file_existed: true,
            matches_removed: removal.matches_removed,
            bytes_written: if file_deleted {
                0
            } else {
                removal.content.len()
            },
            index_removed,
            checkpoint_id: checkpoint.id,
        })
    }
}

struct AgentScopedMemoryDeleteResult {
    file_deleted: bool,
    file_existed: bool,
    matches_removed: usize,
    bytes_written: usize,
    index_removed: bool,
    checkpoint_id: String,
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

struct AgentScopedMemoryDeleteTool {
    writer: AgentScopedMemoryWriter,
}

impl AgentScopedMemoryDeleteTool {
    fn new(
        manager: moltis_memory::runtime::DynMemoryRuntime,
        agent_id: String,
        write_mode: AgentMemoryWriteMode,
    ) -> Self {
        Self {
            writer: AgentScopedMemoryWriter::new(manager, agent_id, write_mode),
        }
    }
}

#[async_trait]
impl AgentTool for AgentScopedMemoryDeleteTool {
    fn name(&self) -> &str {
        "memory_delete"
    }

    fn description(&self) -> &str {
        "Forget saved memory by removing exact text from a memory file or deleting the file entirely. Updates the index immediately."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Target file: MEMORY.md, memory.md, or memory/<name>.md"
                },
                "text": {
                    "type": "string",
                    "description": "Exact text snippet to remove from the file. Required unless delete_file is true."
                },
                "delete_file": {
                    "type": "boolean",
                    "description": "Delete the whole file instead of removing exact text.",
                    "default": false
                },
                "all_matches": {
                    "type": "boolean",
                    "description": "Remove every exact match of text instead of only the first match.",
                    "default": false
                },
                "delete_if_empty": {
                    "type": "boolean",
                    "description": "Delete the file if removing text leaves only whitespace.",
                    "default": true
                }
            },
            "required": ["file"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let file = params
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing 'file' parameter"))?;
        let delete_file = params
            .get("delete_file")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = params.get("text").and_then(Value::as_str);
        let all_matches = params
            .get("all_matches")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let delete_if_empty = params
            .get("delete_if_empty")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let result = self
            .writer
            .delete_memory(file, text, delete_file, all_matches, delete_if_empty)
            .await?;

        Ok(serde_json::json!({
            "deleted": true,
            "path": file,
            "file_deleted": result.file_deleted,
            "file_existed": result.file_existed,
            "matches_removed": result.matches_removed,
            "bytes_written": result.bytes_written,
            "index_removed": result.index_removed,
            "checkpointId": result.checkpoint_id,
        }))
    }
}

struct AgentScopedMemoryForgetTool {
    manager: moltis_memory::runtime::DynMemoryRuntime,
    writer: AgentScopedMemoryWriter,
    provider: Arc<dyn LlmProvider>,
    agent_id: String,
}

impl AgentScopedMemoryForgetTool {
    fn new(
        manager: moltis_memory::runtime::DynMemoryRuntime,
        provider: Arc<dyn LlmProvider>,
        agent_id: String,
        write_mode: AgentMemoryWriteMode,
    ) -> Self {
        Self {
            manager: Arc::clone(&manager),
            writer: AgentScopedMemoryWriter::new(manager, agent_id.clone(), write_mode),
            provider,
            agent_id,
        }
    }
}

#[async_trait]
impl AgentTool for AgentScopedMemoryForgetTool {
    fn name(&self) -> &str {
        "memory_forget"
    }

    fn description(&self) -> &str {
        "Use the model to identify which saved memory chunk(s) match a forget request, then delete the exact stored text safely."
    }

    fn parameters_schema(&self) -> Value {
        memory_forget_parameters_schema()
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let request = parse_forget_request(&params)?;
        let candidates =
            collect_forget_candidates(&self.manager, &request.request, request.limit, |path| {
                if is_path_in_agent_memory_scope(path, &self.agent_id) {
                    agent_memory_file_label_for_path(path, &self.agent_id)
                } else {
                    None
                }
            })
            .await?;

        if candidates.is_empty() {
            return Ok(json!({
                "deleted": false,
                "dry_run": request.dry_run,
                "needs_confirmation": false,
                "rationale": "No relevant saved memory chunks matched the forget request.",
                "candidate_count": 0,
                "planned_matches": [],
                "issues": [],
                "results": [],
                "checkpointIds": [],
            }));
        }

        let plan = plan_memory_forget(&*self.provider, &request.request, &candidates).await?;
        let (validated_actions, issues) =
            validate_forget_actions(&plan.actions, &candidates).await?;
        let planned_matches: Vec<Value> = validated_actions
            .iter()
            .map(|action| forget_planned_match_json(&action.candidate, &action.reason))
            .collect();

        if request.dry_run || plan.needs_confirmation || !issues.is_empty() {
            return Ok(json!({
                "deleted": false,
                "dry_run": request.dry_run,
                "needs_confirmation": plan.needs_confirmation || !issues.is_empty(),
                "rationale": plan.rationale,
                "candidate_count": candidates.len(),
                "planned_matches": planned_matches,
                "issues": issues,
                "results": [],
                "checkpointIds": [],
            }));
        }

        let mut results = Vec::new();
        let mut checkpoint_ids = Vec::new();
        for action in validated_actions {
            let result = self
                .writer
                .delete_memory(
                    &action.candidate.file,
                    Some(&action.candidate.text),
                    false,
                    false,
                    true,
                )
                .await?;
            checkpoint_ids.push(result.checkpoint_id.clone());
            results.push(json!({
                "chunk_id": action.candidate.chunk_id,
                "reason": action.reason,
                "path": action.candidate.file,
                "file_deleted": result.file_deleted,
                "file_existed": result.file_existed,
                "matches_removed": result.matches_removed,
                "bytes_written": result.bytes_written,
                "index_removed": result.index_removed,
                "checkpointId": result.checkpoint_id,
            }));
        }

        Ok(json!({
            "deleted": !results.is_empty(),
            "dry_run": false,
            "needs_confirmation": false,
            "rationale": plan.rationale,
            "candidate_count": candidates.len(),
            "planned_matches": planned_matches,
            "issues": issues,
            "results": results,
            "checkpointIds": checkpoint_ids,
        }))
    }
}

pub struct MemoryForgetTool {
    manager: moltis_memory::runtime::DynMemoryRuntime,
    providers: Arc<RwLock<ProviderRegistry>>,
    session_metadata: Arc<SqliteSessionMetadata>,
}

impl MemoryForgetTool {
    pub fn new(
        manager: moltis_memory::runtime::DynMemoryRuntime,
        providers: Arc<RwLock<ProviderRegistry>>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            manager,
            providers,
            session_metadata,
        }
    }

    async fn resolve_provider(
        &self,
        session_key: Option<&str>,
    ) -> anyhow::Result<Arc<dyn LlmProvider>> {
        let session_model = if let Some(session_key) = session_key {
            self.session_metadata
                .get(session_key)
                .await
                .and_then(|entry| entry.model)
        } else {
            None
        };

        let registry = self.providers.read().await;
        if let Some(model) = session_model {
            if let Some(provider) = registry.get(&model) {
                return Ok(provider);
            }
            warn!(
                session_key,
                model, "memory_forget could not resolve session model, falling back"
            );
        }

        registry
            .first_with_tools()
            .or_else(|| registry.first())
            .ok_or_else(|| anyhow::anyhow!("no LLM provider is configured for memory_forget"))
    }
}

#[async_trait]
impl AgentTool for MemoryForgetTool {
    fn name(&self) -> &str {
        "memory_forget"
    }

    fn description(&self) -> &str {
        "Use the model to identify which saved memory chunk(s) match a forget request, then delete the exact stored text safely."
    }

    fn parameters_schema(&self) -> Value {
        memory_forget_parameters_schema()
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let request = parse_forget_request(&params)?;
        let provider = self
            .resolve_provider(request.session_key.as_deref())
            .await?;
        let candidates =
            collect_forget_candidates(&self.manager, &request.request, request.limit, |path| {
                global_memory_file_label_for_path(&*self.manager, path)
            })
            .await?;

        if candidates.is_empty() {
            return Ok(json!({
                "deleted": false,
                "dry_run": request.dry_run,
                "needs_confirmation": false,
                "rationale": "No relevant saved memory chunks matched the forget request.",
                "candidate_count": 0,
                "planned_matches": [],
                "issues": [],
                "results": [],
                "checkpointIds": [],
            }));
        }

        let plan = plan_memory_forget(&*provider, &request.request, &candidates).await?;
        let (validated_actions, issues) =
            validate_forget_actions(&plan.actions, &candidates).await?;
        let planned_matches: Vec<Value> = validated_actions
            .iter()
            .map(|action| forget_planned_match_json(&action.candidate, &action.reason))
            .collect();

        if request.dry_run || plan.needs_confirmation || !issues.is_empty() {
            return Ok(json!({
                "deleted": false,
                "dry_run": request.dry_run,
                "needs_confirmation": plan.needs_confirmation || !issues.is_empty(),
                "rationale": plan.rationale,
                "candidate_count": candidates.len(),
                "planned_matches": planned_matches,
                "issues": issues,
                "results": [],
                "checkpointIds": [],
            }));
        }

        let delete_tool = moltis_memory::tools::MemoryDeleteTool::new(Arc::clone(&self.manager));
        let mut results = Vec::new();
        let mut checkpoint_ids = Vec::new();
        for action in validated_actions {
            let result = delete_tool
                .execute(json!({
                    "file": &action.candidate.file,
                    "text": &action.candidate.text,
                    "delete_if_empty": true,
                }))
                .await?;
            if let Some(checkpoint_id) = result.get("checkpointId").and_then(Value::as_str) {
                checkpoint_ids.push(checkpoint_id.to_string());
            }
            results.push(json!({
                "chunk_id": action.candidate.chunk_id,
                "reason": action.reason,
                "delete_result": result,
            }));
        }

        Ok(json!({
            "deleted": !results.is_empty(),
            "dry_run": false,
            "needs_confirmation": false,
            "rationale": plan.rationale,
            "candidate_count": candidates.len(),
            "planned_matches": planned_matches,
            "issues": issues,
            "results": results,
            "checkpointIds": checkpoint_ids,
        }))
    }
}

pub(crate) fn install_agent_scoped_memory_tools(
    registry: &mut ToolRegistry,
    manager: &moltis_memory::runtime::DynMemoryRuntime,
    provider: Arc<dyn LlmProvider>,
    agent_id: &str,
    style: MemoryStyle,
    write_mode: AgentMemoryWriteMode,
) {
    let had_search = registry.unregister("memory_search");
    let had_get = registry.unregister("memory_get");
    let had_save = registry.unregister("memory_save");
    let had_delete = registry.unregister("memory_delete");
    let had_forget = registry.unregister("memory_forget");

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
            agent_id_owned.clone(),
            write_mode,
        )));
    }
    if had_delete && memory_write_mode_allows_save(write_mode) {
        registry.register(Box::new(AgentScopedMemoryDeleteTool::new(
            Arc::clone(manager),
            agent_id_owned,
            write_mode,
        )));
    }
    if had_forget && memory_write_mode_allows_save(write_mode) {
        registry.register(Box::new(AgentScopedMemoryForgetTool::new(
            Arc::clone(manager),
            provider,
            agent_id.to_string(),
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
pub(crate) fn effective_tool_mode(provider: &dyn LlmProvider) -> ToolMode {
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

#[cfg(test)]
mod tests;
