//! Sub-agent tool: lets the LLM delegate tasks to a child agent loop.

use std::{collections::HashSet, sync::Arc};

use {async_trait::async_trait, tracing::info};

use crate::{
    error::Error,
    params::{bool_param, str_param, u64_param},
};

use {
    moltis_agents::{
        AgentRunError,
        model::LlmProvider,
        runner::{RunnerEvent, run_agent_loop_with_context},
        tool_registry::{AgentTool, ToolRegistry},
    },
    moltis_config::schema::{AgentPreset, AgentsConfig},
    moltis_providers::ProviderRegistry,
    moltis_sessions::{metadata::SqliteSessionMetadata, store::SessionStore},
};

use crate::sessions_communicate::{
    SendToSessionFn, SessionAccessPolicy, SessionsHistoryTool, SessionsListTool, SessionsSendTool,
};

/// Maximum nesting depth for sub-agents (prevents infinite recursion).
const MAX_SPAWN_DEPTH: u64 = 3;

/// Tool parameter injected via `tool_context` to track nesting depth.
const SPAWN_DEPTH_KEY: &str = "_spawn_depth";

/// Minimal delegate-only toolset for coordinator-style sub-agents.
const DELEGATE_TOOLS: &[&str] = &[
    "spawn_agent",
    "sessions_list",
    "sessions_history",
    "sessions_send",
    "task_list",
];

/// A tool that spawns a sub-agent running its own agent loop.
///
/// The sub-agent executes synchronously (blocks until done) and its result
/// is returned as the tool output. Sub-agents get a filtered copy of the
/// parent's tool registry (without the `spawn_agent` tool itself) and a
/// focused system prompt.
/// Callback for emitting events from the sub-agent back to the parent UI.
pub type OnSpawnEvent = Arc<dyn Fn(RunnerEvent) + Send + Sync>;

/// Dependencies for building policy-aware session tools in sub-agents.
#[derive(Clone)]
pub struct SessionDeps {
    pub session_metadata: Arc<SqliteSessionMetadata>,
    pub session_store: Arc<SessionStore>,
    pub send_to_session: SendToSessionFn,
}

pub struct SpawnAgentTool {
    provider_registry: Arc<tokio::sync::RwLock<ProviderRegistry>>,
    default_provider: Arc<dyn LlmProvider>,
    tool_registry: Arc<ToolRegistry>,
    agents_config: Option<Arc<tokio::sync::RwLock<AgentsConfig>>>,
    on_event: Option<OnSpawnEvent>,
    session_deps: Option<SessionDeps>,
}

impl SpawnAgentTool {
    pub fn new(
        provider_registry: Arc<tokio::sync::RwLock<ProviderRegistry>>,
        default_provider: Arc<dyn LlmProvider>,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            provider_registry,
            default_provider,
            tool_registry,
            agents_config: None,
            on_event: None,
            session_deps: None,
        }
    }

    /// Set an event callback so sub-agent activity is visible to the UI.
    pub fn with_on_event(mut self, on_event: OnSpawnEvent) -> Self {
        self.on_event = Some(on_event);
        self
    }

    /// Attach agent preset config for `preset` lookups.
    pub fn with_agents_config(
        mut self,
        agents_config: Arc<tokio::sync::RwLock<AgentsConfig>>,
    ) -> Self {
        self.agents_config = Some(agents_config);
        self
    }

    /// Provide session dependencies so sub-agents can get policy-aware session tools.
    pub fn with_session_deps(mut self, deps: SessionDeps) -> Self {
        self.session_deps = Some(deps);
        self
    }

    fn emit(&self, event: RunnerEvent) {
        if let Some(ref cb) = self.on_event {
            cb(event);
        }
    }

    fn parse_tool_name_array(params: &serde_json::Value, key: &str) -> crate::Result<Vec<String>> {
        let Some(raw) = params.get(key) else {
            return Ok(Vec::new());
        };
        let arr = raw
            .as_array()
            .ok_or_else(|| Error::message(format!("parameter '{key}' must be an array")))?;
        let mut out = Vec::new();
        for (idx, item) in arr.iter().enumerate() {
            let name = item.as_str().ok_or_else(|| {
                Error::message(format!("parameter '{key}[{idx}]' must be a string"))
            })?;
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(Error::message(format!(
                    "parameter '{key}[{idx}]' cannot be empty"
                )));
            }
            out.push(trimmed.to_string());
        }
        Ok(out)
    }

    fn build_sub_tools(
        &self,
        allow_tools: &[String],
        deny_tools: &[String],
        delegate_only: bool,
    ) -> ToolRegistry {
        let mut sub_tools = if delegate_only {
            let allowed: HashSet<&str> = DELEGATE_TOOLS.iter().copied().collect();
            self.tool_registry
                .clone_allowed_by(|name| allowed.contains(name))
        } else if !allow_tools.is_empty() {
            let allowed: HashSet<&str> = allow_tools.iter().map(String::as_str).collect();
            self.tool_registry
                .clone_allowed_by(|name| name != "spawn_agent" && allowed.contains(name))
        } else {
            // Default behavior preserves old semantics.
            self.tool_registry.clone_without(&["spawn_agent"])
        };

        if !deny_tools.is_empty() {
            let deny: HashSet<&str> = deny_tools.iter().map(String::as_str).collect();
            sub_tools = sub_tools.clone_allowed_by(|name| !deny.contains(name));
        }

        sub_tools
    }

    /// Apply reasoning_effort from the agent preset to the provider, if set.
    ///
    /// Returns the original provider if reasoning_effort is not configured or
    /// the provider doesn't support it. Warns when the preset overrides a
    /// reasoning effort already set via the model ID suffix.
    fn maybe_apply_reasoning_effort(
        provider: Arc<dyn LlmProvider>,
        preset: Option<&AgentPreset>,
    ) -> Arc<dyn LlmProvider> {
        let Some(effort) = preset.and_then(|p| p.reasoning_effort) else {
            return provider;
        };
        // Warn if the provider already has a (different) reasoning effort from
        // the model ID suffix — the preset value will take precedence.
        if let Some(existing) = provider.reasoning_effort()
            && existing != effort
        {
            tracing::warn!(
                model = %provider.id(),
                existing = ?existing,
                preset = ?effort,
                "preset reasoning_effort overrides model-ID suffix; using preset value"
            );
        }
        let cloned = Arc::clone(&provider);
        let new_provider = cloned.with_reasoning_effort(effort);
        if new_provider.is_none() {
            info!(
                model = %provider.id(),
                ?effort,
                "provider does not support reasoning effort; ignoring preset setting"
            );
        }
        new_provider.unwrap_or(provider)
    }

    async fn resolve_preset(
        &self,
        params: &serde_json::Value,
    ) -> crate::Result<(Option<String>, Option<AgentPreset>)> {
        let explicit_name = str_param(params, "preset").map(String::from);

        let Some(ref agents_config) = self.agents_config else {
            if explicit_name.is_some() {
                return Err(Error::message(
                    "spawn preset requested but agents presets are not configured",
                ));
            }
            return Ok((None, None));
        };

        let agents = agents_config.read().await;
        let preset_name = explicit_name.or_else(|| agents.default_preset.clone());
        let Some(preset_name) = preset_name else {
            return Ok((None, None));
        };
        let preset = agents.get_preset(&preset_name).cloned().ok_or_else(|| {
            Error::message(format!(
                "spawn preset '{preset_name}' not found in config.agents.presets"
            ))
        })?;
        Ok((Some(preset_name), Some(preset)))
    }
}

/// Resolve the memory directory for a preset based on its scope.
fn resolve_memory_dir(
    preset_name: &str,
    scope: &moltis_config::schema::MemoryScope,
) -> std::path::PathBuf {
    use moltis_config::schema::MemoryScope;
    match scope {
        MemoryScope::User => {
            let data_dir = moltis_config::data_dir();
            data_dir.join("agent-memory").join(preset_name)
        },
        MemoryScope::Project => std::path::PathBuf::from(".moltis")
            .join("agent-memory")
            .join(preset_name),
        MemoryScope::Local => std::path::PathBuf::from(".moltis")
            .join("agent-memory-local")
            .join(preset_name),
    }
}

/// Load the first N lines of MEMORY.md from the agent's memory directory.
/// Returns `None` if the file doesn't exist or is empty.
fn load_memory_context(
    preset_name: &str,
    config: &moltis_config::schema::PresetMemoryConfig,
) -> Option<String> {
    let dir = resolve_memory_dir(preset_name, &config.scope);
    load_memory_from_dir(&dir, config.max_lines)
}

/// Load memory content from a specific directory.
fn load_memory_from_dir(dir: &std::path::Path, max_lines: usize) -> Option<String> {
    let memory_path = dir.join("MEMORY.md");

    // Create directory if missing so agents can write to it later.
    let _ = std::fs::create_dir_all(dir);

    let content = std::fs::read_to_string(&memory_path).ok()?;
    if content.trim().is_empty() {
        return None;
    }

    let lines: Vec<&str> = content.lines().take(max_lines).collect();
    Some(lines.join("\n"))
}

/// Build the system prompt for a sub-agent, incorporating preset customizations.
fn build_sub_agent_prompt(
    task: &str,
    context: &str,
    preset: Option<&AgentPreset>,
    preset_name: Option<&str>,
) -> String {
    let mut prompt = String::new();

    // Add preset identity if available.
    if let Some(p) = preset {
        if let Some(ref name) = p.identity.name {
            prompt.push_str(&format!("You are {name}"));
            if let Some(ref emoji) = p.identity.emoji {
                prompt.push_str(&format!(" ({emoji})"));
            }
            prompt.push_str(". ");
        }
        if let Some(ref theme) = p.identity.theme {
            prompt.push_str(&format!("Your style is {theme}. "));
        }
    }

    // Add base instruction.
    if prompt.is_empty() {
        prompt.push_str("You are a sub-agent spawned to handle a specific task. ");
    }
    prompt.push_str("Complete the task thoroughly and return a clear result.\n\n");

    // Inject persistent memory if configured.
    if let Some(p) = preset
        && let Some(ref mem_config) = p.memory
        && let Some(name) = preset_name
        && let Some(memory_content) = load_memory_context(name, mem_config)
    {
        prompt.push_str("# Agent Memory\n\n");
        prompt.push_str(&memory_content);
        prompt.push_str("\n\n");
    }

    // Add task.
    prompt.push_str(&format!("Task: {task}"));

    // Add context if provided.
    if !context.is_empty() {
        prompt.push_str(&format!("\n\nContext: {context}"));
    }

    // Add preset system prompt suffix.
    if let Some(extra) = preset
        .and_then(|p| p.system_prompt_suffix.as_ref())
        .map(|s| s.trim())
        .filter(|v| !v.is_empty())
    {
        prompt.push_str("\n\n");
        prompt.push_str(extra);
    }

    prompt
}

#[async_trait]
impl AgentTool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a complex, multi-step task autonomously. \
         The sub-agent runs its own agent loop with access to tools and returns \
         the result when done. Use this to delegate tasks that require multiple \
         tool calls or independent reasoning. Supports optional tool policy controls."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The task to delegate to the sub-agent"
                },
                "context": {
                    "type": "string",
                    "description": "Additional context for the sub-agent (optional)"
                },
                "preset": {
                    "type": "string",
                    "description": "Agent preset name (e.g. 'researcher', 'coder'). Presets define model, tool policies, and behavior."
                },
                "model": {
                    "type": "string",
                    "description": "Model ID to use (e.g. a cheaper model). If not specified, uses preset model or parent's model."
                },
                "allow_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional whitelist of tool names for the sub-agent. spawn_agent is always excluded unless delegate_only is true."
                },
                "deny_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional blacklist of tool names for the sub-agent."
                },
                "delegate_only": {
                    "type": "boolean",
                    "description": "If true, sub-agent is restricted to delegation/session/task tools."
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let task = str_param(&params, "task")
            .ok_or_else(|| Error::message("missing required parameter: task"))?;
        let context = str_param(&params, "context").unwrap_or("");
        let (preset_name, preset) = self.resolve_preset(&params).await?;
        let explicit_model = str_param(&params, "model").map(String::from);
        let model_id = explicit_model
            .clone()
            .or_else(|| preset.as_ref().and_then(|p| p.model.clone()));

        let explicit_allow_tools = Self::parse_tool_name_array(&params, "allow_tools")?;
        let allow_tools = if explicit_allow_tools.is_empty() {
            preset
                .as_ref()
                .map(|p| p.tools.allow.clone())
                .unwrap_or_default()
        } else {
            explicit_allow_tools
        };

        let explicit_deny_tools = Self::parse_tool_name_array(&params, "deny_tools")?;
        let deny_tools = if explicit_deny_tools.is_empty() {
            preset
                .as_ref()
                .map(|p| p.tools.deny.clone())
                .unwrap_or_default()
        } else {
            explicit_deny_tools
        };

        let delegate_only = bool_param(
            &params,
            "delegate_only",
            preset.as_ref().map(|p| p.delegate_only).unwrap_or(false),
        );

        // Check nesting depth.
        let depth = u64_param(&params, SPAWN_DEPTH_KEY, 0);
        if depth >= MAX_SPAWN_DEPTH {
            return Err(Error::message(format!(
                "maximum sub-agent nesting depth ({MAX_SPAWN_DEPTH}) exceeded"
            ))
            .into());
        }

        // Resolve provider (and apply reasoning_effort from preset if set).
        let provider = if let Some(id) = model_id {
            let reg = self.provider_registry.read().await;
            let base_provider = reg
                .get(&id)
                .ok_or_else(|| Error::message(format!("unknown model: {id}")))?;
            Self::maybe_apply_reasoning_effort(base_provider, preset.as_ref())
        } else {
            let base = Arc::clone(&self.default_provider);
            Self::maybe_apply_reasoning_effort(base, preset.as_ref())
        };

        // Capture model ID before provider is moved into the sub-agent loop.
        let model_id = provider.id().to_string();

        info!(
            task = %task,
            depth = depth,
            model = %model_id,
            preset = ?preset_name,
            "spawning sub-agent"
        );

        self.emit(RunnerEvent::SubAgentStart {
            task: task.to_string(),
            model: model_id.clone(),
            depth,
        });

        // Build filtered tool registry from policy knobs.
        let mut sub_tools = self.build_sub_tools(&allow_tools, &deny_tools, delegate_only);

        // Apply session access policy if the preset configures one.
        if let Some(ref p) = preset
            && let Some(ref session_config) = p.sessions
            && let Some(ref deps) = self.session_deps
        {
            let policy = SessionAccessPolicy::from(session_config);
            sub_tools.replace(Box::new(
                SessionsListTool::new(Arc::clone(&deps.session_metadata))
                    .with_policy(policy.clone()),
            ));
            sub_tools.replace(Box::new(
                SessionsHistoryTool::new(
                    Arc::clone(&deps.session_store),
                    Arc::clone(&deps.session_metadata),
                )
                .with_policy(policy.clone()),
            ));
            sub_tools.replace(Box::new(
                SessionsSendTool::new(
                    Arc::clone(&deps.session_metadata),
                    Arc::clone(&deps.send_to_session),
                )
                .with_policy(policy),
            ));
        }

        // Build system prompt with identity injection and memory.
        let system_prompt =
            build_sub_agent_prompt(task, context, preset.as_ref(), preset_name.as_deref());

        // Build tool context with incremented depth and propagated session key.
        let mut tool_context = serde_json::json!({
            SPAWN_DEPTH_KEY: depth + 1,
        });
        if let Some(session_key) = params.get("_session_key") {
            tool_context["_session_key"] = session_key.clone();
        }

        // Run the sub-agent loop, optionally with a timeout.
        let user_content = moltis_agents::UserContent::text(task);
        let agent_future = run_agent_loop_with_context(
            provider,
            &sub_tools,
            &system_prompt,
            &user_content,
            None,
            None, // no history
            Some(tool_context),
            None, // no hooks for sub-agents
        );

        let result = if let Some(timeout_secs) = preset.as_ref().and_then(|p| p.timeout_secs) {
            let duration = std::time::Duration::from_secs(timeout_secs);
            match tokio::time::timeout(duration, agent_future).await {
                Ok(r) => r,
                Err(_) => Err(AgentRunError::Other(anyhow::anyhow!(
                    "sub-agent timed out after {timeout_secs}s"
                ))),
            }
        } else {
            agent_future.await
        };

        // Emit SubAgentEnd regardless of success/failure.
        let (iterations, tool_calls_made) = match &result {
            Ok(r) => (r.iterations, r.tool_calls_made),
            Err(_) => (0, 0),
        };
        self.emit(RunnerEvent::SubAgentEnd {
            task: task.to_string(),
            model: model_id.clone(),
            depth,
            iterations,
            tool_calls_made,
        });

        let result = result?;

        info!(
            task = %task,
            depth = depth,
            iterations = result.iterations,
            tool_calls = result.tool_calls_made,
            preset = ?preset_name,
            "sub-agent completed"
        );

        Ok(serde_json::json!({
            "text": result.text,
            "iterations": result.iterations,
            "tool_calls_made": result.tool_calls_made,
            "model": model_id,
            "preset": preset_name,
        }))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        moltis_agents::model::{ChatMessage, CompletionResponse, StreamEvent, Usage},
        moltis_config::schema::{AgentIdentity, PresetToolPolicy},
        std::pin::Pin,
        tokio_stream::Stream,
    };

    /// Mock provider that returns a fixed response.
    struct MockProvider {
        response: String,
        model_id: String,
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        fn id(&self) -> &str {
            &self.model_id
        }

        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[serde_json::Value],
        ) -> anyhow::Result<CompletionResponse> {
            Ok(CompletionResponse {
                text: Some(self.response.clone()),
                tool_calls: vec![],
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
            })
        }

        fn stream(
            &self,
            _messages: Vec<ChatMessage>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            Box::pin(tokio_stream::empty())
        }
    }

    fn make_empty_provider_registry() -> Arc<tokio::sync::RwLock<ProviderRegistry>> {
        let env_overrides = std::collections::HashMap::new();
        Arc::new(tokio::sync::RwLock::new(
            ProviderRegistry::from_config_with_static_catalogs(&Default::default(), &env_overrides),
        ))
    }

    struct DummyNamedTool {
        name: String,
    }

    #[async_trait]
    impl AgentTool for DummyNamedTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "dummy"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }

        async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(params)
        }
    }

    fn registry_with_tools(names: &[&str]) -> Arc<ToolRegistry> {
        let mut registry = ToolRegistry::new();
        for name in names {
            registry.register(Box::new(DummyNamedTool {
                name: (*name).to_string(),
            }));
        }
        Arc::new(registry)
    }

    fn agents_config_with_presets(
        default_preset: Option<&str>,
        presets: &[(&str, AgentPreset)],
    ) -> Arc<tokio::sync::RwLock<AgentsConfig>> {
        let mut cfg = AgentsConfig {
            default_preset: default_preset.map(String::from),
            ..Default::default()
        };
        for (name, preset) in presets {
            cfg.presets.insert((*name).to_string(), preset.clone());
        }
        Arc::new(tokio::sync::RwLock::new(cfg))
    }

    #[tokio::test]
    async fn test_sub_agent_runs_and_returns_result() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "Sub-agent result".into(),
            model_id: "mock-model".into(),
        });
        let tool_registry = Arc::new(ToolRegistry::new());
        let spawn_tool = SpawnAgentTool::new(
            make_empty_provider_registry(),
            Arc::clone(&provider),
            tool_registry,
        );

        let params = serde_json::json!({ "task": "do something" });
        let result = spawn_tool.execute(params).await.unwrap();

        assert_eq!(result["text"], "Sub-agent result");
        assert_eq!(result["iterations"], 1);
        assert_eq!(result["tool_calls_made"], 0);
        assert_eq!(result["model"], "mock-model");
    }

    #[tokio::test]
    async fn test_depth_limit_rejects() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "nope".into(),
            model_id: "mock".into(),
        });
        let tool_registry = Arc::new(ToolRegistry::new());
        let spawn_tool =
            SpawnAgentTool::new(make_empty_provider_registry(), provider, tool_registry);

        let params = serde_json::json!({
            "task": "do something",
            "_spawn_depth": MAX_SPAWN_DEPTH,
        });
        let result = spawn_tool.execute(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nesting depth"));
    }

    #[tokio::test]
    async fn test_spawn_agent_excluded_from_sub_registry() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "ok".into(),
            model_id: "mock".into(),
        });

        // Create a registry with spawn_agent in it.
        let mut registry = ToolRegistry::new();

        struct DummyTool;
        #[async_trait]
        impl AgentTool for DummyTool {
            fn name(&self) -> &str {
                "spawn_agent"
            }

            fn description(&self) -> &str {
                "dummy"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
                Ok(serde_json::json!("dummy"))
            }
        }

        struct EchoTool;
        #[async_trait]
        impl AgentTool for EchoTool {
            fn name(&self) -> &str {
                "echo"
            }

            fn description(&self) -> &str {
                "echo"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            async fn execute(&self, p: serde_json::Value) -> anyhow::Result<serde_json::Value> {
                Ok(p)
            }
        }

        registry.register(Box::new(DummyTool));
        registry.register(Box::new(EchoTool));

        let filtered = registry.clone_without(&["spawn_agent"]);
        assert!(filtered.get("spawn_agent").is_none());
        assert!(filtered.get("echo").is_some());

        // Also verify schemas don't include spawn_agent.
        let schemas = filtered.list_schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0]["name"], "echo");

        // Ensure original is unaffected.
        assert!(registry.get("spawn_agent").is_some());

        // The SpawnAgentTool itself should work with the filtered registry.
        let spawn_tool =
            SpawnAgentTool::new(make_empty_provider_registry(), provider, Arc::new(registry));
        let result = spawn_tool
            .execute(serde_json::json!({ "task": "test" }))
            .await
            .unwrap();
        assert_eq!(result["text"], "ok");
    }

    #[tokio::test]
    async fn test_context_passed_to_sub_agent() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "done with context".into(),
            model_id: "mock".into(),
        });
        let spawn_tool = SpawnAgentTool::new(
            make_empty_provider_registry(),
            provider,
            Arc::new(ToolRegistry::new()),
        );

        let params = serde_json::json!({
            "task": "analyze code",
            "context": "The code is in src/main.rs",
        });
        let result = spawn_tool.execute(params).await.unwrap();
        assert_eq!(result["text"], "done with context");
    }

    #[tokio::test]
    async fn test_missing_task_parameter() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "nope".into(),
            model_id: "mock".into(),
        });
        let spawn_tool = SpawnAgentTool::new(
            make_empty_provider_registry(),
            provider,
            Arc::new(ToolRegistry::new()),
        );

        let result = spawn_tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("task"));
    }

    #[tokio::test]
    async fn test_build_sub_tools_applies_allow_and_deny() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "ok".into(),
            model_id: "mock".into(),
        });
        let spawn_tool = SpawnAgentTool::new(
            make_empty_provider_registry(),
            provider,
            registry_with_tools(&["spawn_agent", "exec", "web_fetch", "task_list"]),
        );

        let filtered = spawn_tool.build_sub_tools(
            &[
                "exec".to_string(),
                "task_list".to_string(),
                "spawn_agent".to_string(),
            ],
            &["task_list".to_string()],
            false,
        );
        assert!(filtered.get("exec").is_some());
        assert!(filtered.get("task_list").is_none());
        assert!(filtered.get("spawn_agent").is_none());
        assert!(filtered.get("web_fetch").is_none());
    }

    #[tokio::test]
    async fn test_build_sub_tools_delegate_only_uses_delegate_set() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "ok".into(),
            model_id: "mock".into(),
        });
        let spawn_tool = SpawnAgentTool::new(
            make_empty_provider_registry(),
            provider,
            registry_with_tools(&[
                "spawn_agent",
                "sessions_list",
                "sessions_history",
                "sessions_send",
                "task_list",
                "exec",
            ]),
        );

        let filtered = spawn_tool.build_sub_tools(&[], &[], true);
        assert!(filtered.get("spawn_agent").is_some());
        assert!(filtered.get("sessions_list").is_some());
        assert!(filtered.get("sessions_history").is_some());
        assert!(filtered.get("sessions_send").is_some());
        assert!(filtered.get("task_list").is_some());
        assert!(filtered.get("exec").is_none());
    }

    #[tokio::test]
    async fn test_resolve_preset_uses_explicit_name() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "ok".into(),
            model_id: "mock".into(),
        });
        let spawn_tool = SpawnAgentTool::new(
            make_empty_provider_registry(),
            provider,
            Arc::new(ToolRegistry::new()),
        )
        .with_agents_config(agents_config_with_presets(Some("default"), &[(
            "research",
            AgentPreset {
                delegate_only: true,
                ..Default::default()
            },
        )]));

        let (name, preset) = spawn_tool
            .resolve_preset(&serde_json::json!({ "preset": "research" }))
            .await
            .expect("resolve preset");
        assert_eq!(name.as_deref(), Some("research"));
        assert_eq!(preset.as_ref().map(|p| p.delegate_only), Some(true));
    }

    #[tokio::test]
    async fn test_resolve_preset_uses_default_when_missing() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "ok".into(),
            model_id: "mock".into(),
        });
        let spawn_tool = SpawnAgentTool::new(
            make_empty_provider_registry(),
            provider,
            Arc::new(ToolRegistry::new()),
        )
        .with_agents_config(agents_config_with_presets(Some("default"), &[(
            "default",
            AgentPreset {
                tools: PresetToolPolicy {
                    allow: vec!["task_list".to_string()],
                    ..Default::default()
                },
                ..Default::default()
            },
        )]));

        let (name, preset) = spawn_tool
            .resolve_preset(&serde_json::json!({}))
            .await
            .expect("resolve default preset");
        assert_eq!(name.as_deref(), Some("default"));
        assert_eq!(
            preset
                .as_ref()
                .map(|p| p.tools.allow.clone())
                .unwrap_or_default(),
            vec!["task_list".to_string()]
        );
    }

    #[tokio::test]
    async fn test_resolve_preset_errors_when_name_missing() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider {
            response: "ok".into(),
            model_id: "mock".into(),
        });
        let spawn_tool = SpawnAgentTool::new(
            make_empty_provider_registry(),
            provider,
            Arc::new(ToolRegistry::new()),
        )
        .with_agents_config(agents_config_with_presets(None, &[]));

        let result = spawn_tool
            .resolve_preset(&serde_json::json!({ "preset": "missing" }))
            .await;
        assert!(result.is_err());
        assert!(
            result
                .err()
                .map(|e| e.to_string().contains("not found"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_identity_injected_into_system_prompt() {
        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("scout".into()),
                emoji: Some("🔍".into()),
                theme: Some("thorough".into()),
            },
            system_prompt_suffix: Some("Focus on accuracy.".into()),
            ..Default::default()
        };

        let prompt =
            build_sub_agent_prompt("find bugs", "in main.rs", Some(&preset), Some("scout"));

        assert!(prompt.contains("You are scout (🔍)"));
        assert!(prompt.contains("Your style is thorough"));
        assert!(prompt.contains("Task: find bugs"));
        assert!(prompt.contains("Context: in main.rs"));
        assert!(prompt.contains("Focus on accuracy."));
    }

    #[test]
    fn test_no_identity_uses_default_prompt() {
        let prompt = build_sub_agent_prompt("do work", "", None, None);

        assert!(prompt.contains("You are a sub-agent"));
        assert!(prompt.contains("Task: do work"));
        assert!(!prompt.contains("Context:"));
    }

    #[test]
    fn test_memory_injected_into_system_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let memory_dir = dir.path().join("agent-memory").join("researcher");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(
            memory_dir.join("MEMORY.md"),
            "- Always check edge cases\n- Prefer iterators",
        )
        .unwrap();

        // Load memory directly to verify the function works.
        let content = load_memory_from_dir(&memory_dir, 200);
        assert!(content.is_some());
        let content = content.unwrap();
        assert!(content.contains("Always check edge cases"));
        assert!(content.contains("Prefer iterators"));
    }

    #[test]
    fn test_load_memory_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let memory_dir = dir.path();
        let lines: Vec<String> = (0..10).map(|i| format!("Line {i}")).collect();
        std::fs::write(memory_dir.join("MEMORY.md"), lines.join("\n")).unwrap();

        let content = load_memory_from_dir(memory_dir, 3).unwrap();
        assert!(content.contains("Line 0"));
        assert!(content.contains("Line 2"));
        assert!(!content.contains("Line 3"));
    }

    #[test]
    fn test_load_memory_empty_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("MEMORY.md"), "   \n  \n").unwrap();

        let content = load_memory_from_dir(dir.path(), 200);
        assert!(content.is_none());
    }

    #[test]
    fn test_load_memory_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let content = load_memory_from_dir(dir.path(), 200);
        assert!(content.is_none());
    }

    #[tokio::test]
    async fn test_timeout_cancels_long_running_agent() {
        // Mock provider with a slow response.
        struct SlowProvider;

        #[async_trait]
        impl LlmProvider for SlowProvider {
            fn name(&self) -> &str {
                "slow"
            }

            fn id(&self) -> &str {
                "slow-model"
            }

            async fn complete(
                &self,
                _messages: &[ChatMessage],
                _tools: &[serde_json::Value],
            ) -> anyhow::Result<CompletionResponse> {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                Ok(CompletionResponse {
                    text: Some("too late".into()),
                    tool_calls: vec![],
                    usage: Usage::default(),
                })
            }

            fn stream(
                &self,
                _messages: Vec<ChatMessage>,
            ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
                Box::pin(tokio_stream::empty())
            }
        }

        let provider: Arc<dyn LlmProvider> = Arc::new(SlowProvider);
        let spawn_tool = SpawnAgentTool::new(
            make_empty_provider_registry(),
            provider,
            Arc::new(ToolRegistry::new()),
        )
        .with_agents_config(agents_config_with_presets(None, &[("slow", AgentPreset {
            timeout_secs: Some(1),
            ..Default::default()
        })]));

        let params = serde_json::json!({
            "task": "do something slowly",
            "preset": "slow",
        });
        let result = spawn_tool.execute(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }
}
