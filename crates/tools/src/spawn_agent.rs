//! Sub-agent tool: lets the LLM delegate tasks to a child agent loop.

use std::{collections::HashSet, sync::Arc};

use {async_trait::async_trait, tracing::info};

use crate::{
    error::Error,
    params::{bool_param, str_param, u64_param},
};

use {
    moltis_agents::{
        model::LlmProvider,
        runner::{RunnerEvent, run_agent_loop_with_context},
        tool_registry::{AgentTool, ToolRegistry},
    },
    moltis_config::schema::{AgentPresetConfig, AgentsConfig},
    moltis_providers::ProviderRegistry,
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

pub struct SpawnAgentTool {
    provider_registry: Arc<tokio::sync::RwLock<ProviderRegistry>>,
    default_provider: Arc<dyn LlmProvider>,
    tool_registry: Arc<ToolRegistry>,
    agents_config: Option<Arc<tokio::sync::RwLock<AgentsConfig>>>,
    on_event: Option<OnSpawnEvent>,
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

    async fn resolve_preset(
        &self,
        params: &serde_json::Value,
    ) -> crate::Result<(Option<String>, Option<AgentPresetConfig>)> {
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
                    "description": "Optional spawn preset from config.agents.presets."
                },
                "model": {
                    "type": "string",
                    "description": "Model ID to use (e.g. a cheaper model). If not specified, uses the parent's current model."
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
                .map(|p| p.allow_tools.clone())
                .unwrap_or_default()
        } else {
            explicit_allow_tools
        };

        let explicit_deny_tools = Self::parse_tool_name_array(&params, "deny_tools")?;
        let deny_tools = if explicit_deny_tools.is_empty() {
            preset
                .as_ref()
                .map(|p| p.deny_tools.clone())
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

        // Resolve provider.
        let provider = if let Some(id) = model_id {
            let reg = self.provider_registry.read().await;
            reg.get(&id)
                .ok_or_else(|| Error::message(format!("unknown model: {id}")))?
        } else {
            Arc::clone(&self.default_provider)
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
        let sub_tools = self.build_sub_tools(&allow_tools, &deny_tools, delegate_only);

        // Build system prompt.
        let mut system_prompt = if context.is_empty() {
            format!(
                "You are a sub-agent spawned to handle a specific task. \
                 Complete the task thoroughly and return a clear result.\n\n\
                 Task: {task}"
            )
        } else {
            format!(
                "You are a sub-agent spawned to handle a specific task. \
                 Complete the task thoroughly and return a clear result.\n\n\
                Task: {task}\n\nContext: {context}"
            )
        };
        if let Some(extra) = preset
            .as_ref()
            .and_then(|p| p.system_prompt_suffix.as_ref())
            .map(|suffix| suffix.trim())
            .filter(|v| !v.is_empty())
        {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(extra);
        }

        // Build tool context with incremented depth and propagated session key.
        let mut tool_context = serde_json::json!({
            SPAWN_DEPTH_KEY: depth + 1,
        });
        if let Some(session_key) = params.get("_session_key") {
            tool_context["_session_key"] = session_key.clone();
        }

        // Run the sub-agent loop (no event forwarding, no hooks, no history).
        let user_content = moltis_agents::UserContent::text(task);
        let result = run_agent_loop_with_context(
            provider,
            &sub_tools,
            &system_prompt,
            &user_content,
            None,
            None, // no history
            Some(tool_context),
            None, // no hooks for sub-agents
        )
        .await;

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
            "sub-agent completed"
        );

        Ok(serde_json::json!({
            "text": result.text,
            "iterations": result.iterations,
            "tool_calls_made": result.tool_calls_made,
            "model": model_id,
        }))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        moltis_agents::model::{ChatMessage, CompletionResponse, StreamEvent, Usage},
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
        Arc::new(tokio::sync::RwLock::new(
            ProviderRegistry::from_env_with_config(&Default::default()),
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
        presets: &[(&str, AgentPresetConfig)],
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
            AgentPresetConfig {
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
            AgentPresetConfig {
                allow_tools: vec!["task_list".to_string()],
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
                .map(|p| p.allow_tools.clone())
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
}
