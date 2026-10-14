use {
    super::*,
    moltis_agents::model::{ChatMessage, CompletionResponse, StreamEvent, Usage},
    moltis_config::schema::{AgentIdentity, PresetSpawnPolicy, PresetToolPolicy},
    std::{
        path::PathBuf,
        pin::Pin,
        sync::{Mutex, MutexGuard},
    },
    tokio::sync::Notify,
    tokio_stream::Stream,
};

use crate::spawn_agent_tasks::{SpawnCancelTool, SpawnResultTool, SpawnStatusTool};

/// Process-wide data-dir override for tests that touch workspace files.
struct TestDataDirState {
    path: Option<PathBuf>,
}

/// Serialize tests that override the process-wide data directory.
/// The mutex guards the override path itself (not a void sentinel).
static DATA_DIR_TEST_LOCK: Mutex<TestDataDirState> = Mutex::new(TestDataDirState { path: None });

struct DataDirOverrideGuard {
    _lock: MutexGuard<'static, TestDataDirState>,
}

impl DataDirOverrideGuard {
    fn acquire(path: PathBuf) -> Self {
        let mut lock = DATA_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        moltis_config::set_data_dir(path.clone());
        lock.path = Some(path);
        Self { _lock: lock }
    }
}

impl Drop for DataDirOverrideGuard {
    fn drop(&mut self) {
        self._lock.path = None;
        moltis_config::clear_data_dir();
    }
}

/// Mock provider that returns a fixed response.
struct MockProvider {
    response: String,
    model_id: String,
    seen_tool_names: Arc<Mutex<Vec<String>>>,
}

struct NotifyProvider {
    response: String,
    notify: Arc<Notify>,
}

#[async_trait]
impl LlmProvider for NotifyProvider {
    fn name(&self) -> &str {
        "notify"
    }

    fn id(&self) -> &str {
        "notify-model"
    }

    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        self.notify.notified().await;
        Ok(CompletionResponse {
            text: Some(self.response.clone()),
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

struct FailingProvider;

#[async_trait]
impl LlmProvider for FailingProvider {
    fn name(&self) -> &str {
        "failing"
    }

    fn id(&self) -> &str {
        "failing-model"
    }

    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        Err(anyhow::anyhow!("provider failed"))
    }

    fn stream(
        &self,
        _messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(tokio_stream::empty())
    }
}

impl MockProvider {
    fn with_capture(
        response: impl Into<String>,
        model_id: impl Into<String>,
    ) -> (Arc<dyn LlmProvider>, Arc<Mutex<Vec<String>>>) {
        let seen_tool_names = Arc::new(Mutex::new(Vec::new()));
        let provider: Arc<dyn LlmProvider> = Arc::new(Self {
            response: response.into(),
            model_id: model_id.into(),
            seen_tool_names: Arc::clone(&seen_tool_names),
        });
        (provider, seen_tool_names)
    }
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
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let tool_names = tools
            .iter()
            .filter_map(|tool| {
                tool.get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        *self.seen_tool_names.lock().unwrap() = tool_names;
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

    fn supports_tools(&self) -> bool {
        true
    }
}

fn make_empty_provider_registry() -> Arc<tokio::sync::RwLock<ProviderRegistry>> {
    let env_overrides = std::collections::HashMap::new();
    Arc::new(tokio::sync::RwLock::new(
        ProviderRegistry::from_config_with_static_catalogs(
            &Default::default(),
            &env_overrides,
            std::collections::HashMap::new(),
        ),
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
    let (provider, _) = MockProvider::with_capture("Sub-agent result", "mock-model");
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
    let (provider, _) = MockProvider::with_capture("nope", "mock");
    let tool_registry = Arc::new(ToolRegistry::new());
    let spawn_tool = SpawnAgentTool::new(make_empty_provider_registry(), provider, tool_registry);

    let params = serde_json::json!({
        "task": "do something",
        "_spawn_depth": MAX_SPAWN_DEPTH,
    });
    let result = spawn_tool.execute(params).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("nesting depth"));
}

#[tokio::test]
async fn test_nonblocking_returns_task_handle_before_completion() {
    let notify = Arc::new(Notify::new());
    let provider: Arc<dyn LlmProvider> = Arc::new(NotifyProvider {
        response: "background result".to_string(),
        notify: Arc::clone(&notify),
    });
    let store = Arc::new(SpawnTaskStore::default());
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        Arc::new(ToolRegistry::new()),
    )
    .with_task_store(Arc::clone(&store));

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        spawn_tool.execute(serde_json::json!({
            "task": "do something later",
            "nonblocking": true,
            "_session_key": "session-a",
        })),
    )
    .await
    .expect("nonblocking spawn should return before provider completes")
    .unwrap();

    let task_id = result["task_id"].as_str().unwrap();
    assert_eq!(result["status"], "running");

    let status_tool = SpawnStatusTool::new(Arc::clone(&store));
    let status = status_tool
        .execute(serde_json::json!({
            "task_id": task_id,
            "_session_key": "session-a",
        }))
        .await
        .unwrap();
    assert_eq!(status["status"], "running");

    notify.notify_one();
    let result_tool = SpawnResultTool::new(store);
    let mut final_result = serde_json::Value::Null;
    for _ in 0..20 {
        final_result = result_tool
            .execute(serde_json::json!({
                "task_id": task_id,
                "_session_key": "session-a",
            }))
            .await
            .unwrap();
        if final_result["status"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(final_result["status"], "completed");
    assert_eq!(final_result["text"], "background result");
    assert_eq!(final_result["iterations"], 1);
}

#[tokio::test]
async fn test_nonblocking_result_enforces_session_key() {
    let (provider, _) = MockProvider::with_capture("done", "mock");
    let store = Arc::new(SpawnTaskStore::default());
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        Arc::new(ToolRegistry::new()),
    )
    .with_task_store(Arc::clone(&store));

    let result = spawn_tool
        .execute(serde_json::json!({
            "task": "private task",
            "nonblocking": true,
            "_session_key": "session-a",
        }))
        .await
        .unwrap();
    let task_id = result["task_id"].as_str().unwrap();
    let status_tool = SpawnStatusTool::new(store);
    let denied = status_tool
        .execute(serde_json::json!({
            "task_id": task_id,
            "_session_key": "session-b",
        }))
        .await;

    assert!(denied.is_err());
    assert!(denied.unwrap_err().to_string().contains("access denied"));
}

#[tokio::test]
async fn test_nonblocking_failure_is_persisted() {
    let provider: Arc<dyn LlmProvider> = Arc::new(FailingProvider);
    let store = Arc::new(SpawnTaskStore::default());
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        Arc::new(ToolRegistry::new()),
    )
    .with_task_store(Arc::clone(&store));

    let result = spawn_tool
        .execute(serde_json::json!({
            "task": "fail in background",
            "nonblocking": true,
        }))
        .await
        .unwrap();
    let task_id = result["task_id"].as_str().unwrap();
    let result_tool = SpawnResultTool::new(store);

    let mut final_result = serde_json::Value::Null;
    for _ in 0..20 {
        final_result = result_tool
            .execute(serde_json::json!({ "task_id": task_id }))
            .await
            .unwrap();
        if final_result["status"] == "failed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(final_result["status"], "failed");
    assert!(
        final_result["error"]
            .as_str()
            .unwrap()
            .contains("provider failed")
    );
}

#[test]
fn test_nonblocking_and_companion_tool_schemas() {
    let (provider, _) = MockProvider::with_capture("ok", "mock");
    let store = Arc::new(SpawnTaskStore::default());
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        Arc::new(ToolRegistry::new()),
    )
    .with_task_store(Arc::clone(&store));

    assert!(
        spawn_tool.parameters_schema()["properties"]
            .get("nonblocking")
            .is_some()
    );
    assert_eq!(
        SpawnStatusTool::new(Arc::clone(&store)).parameters_schema()["required"][0],
        "task_id"
    );
    assert_eq!(
        SpawnResultTool::new(store).parameters_schema()["required"][0],
        "task_id"
    );
    assert_eq!(
        SpawnCancelTool::new(Arc::new(SpawnTaskStore::default())).parameters_schema()["required"]
            [0],
        "task_id"
    );
}

#[tokio::test]
async fn test_spawn_agent_excluded_from_sub_registry() {
    let (provider, _) = MockProvider::with_capture("ok", "mock");

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
    let (provider, _) = MockProvider::with_capture("done with context", "mock");
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
async fn test_null_optional_array_params_are_treated_as_absent() {
    let (provider, seen_tool_names) = MockProvider::with_capture("done", "mock");
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        registry_with_tools(&["spawn_agent", "exec", "web_fetch", "task_list"]),
    );

    let params = serde_json::json!({
        "task": "analyze code",
        "allow_tools": null,
        "deny_tools": null,
        "context": null,
        "model": null,
        "preset": null,
        "delegate_only": null,
    });
    let result = spawn_tool.execute(params).await.unwrap();
    assert_eq!(result["text"], "done");
    let mut seen = seen_tool_names.lock().unwrap().clone();
    seen.sort();
    assert_eq!(seen, vec![
        "exec".to_string(),
        "task_list".to_string(),
        "web_fetch".to_string(),
    ]);
}

#[tokio::test]
async fn test_missing_task_parameter() {
    let (provider, _) = MockProvider::with_capture("nope", "mock");
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
    let (provider, _) = MockProvider::with_capture("ok", "mock");
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
        None,
    );
    assert!(filtered.get("exec").is_some());
    assert!(filtered.get("task_list").is_none());
    assert!(filtered.get("spawn_agent").is_none());
    assert!(filtered.get("web_fetch").is_none());
}

#[tokio::test]
async fn test_build_sub_tools_delegate_only_uses_delegate_set() {
    let (provider, _) = MockProvider::with_capture("ok", "mock");
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        registry_with_tools(&[
            "spawn_agent",
            "spawn_status",
            "spawn_result",
            "spawn_list",
            "cancel_spawn",
            "sessions_list",
            "sessions_history",
            "sessions_send",
            "task_list",
            "exec",
        ]),
    );

    let filtered = spawn_tool.build_sub_tools(&[], &[], true, None);
    assert!(filtered.get("spawn_agent").is_some());
    assert!(filtered.get("spawn_status").is_some());
    assert!(filtered.get("spawn_result").is_some());
    assert!(filtered.get("spawn_list").is_some());
    assert!(filtered.get("cancel_spawn").is_some());
    assert!(filtered.get("sessions_list").is_some());
    assert!(filtered.get("sessions_history").is_some());
    assert!(filtered.get("sessions_send").is_some());
    assert!(filtered.get("task_list").is_some());
    assert!(filtered.get("exec").is_none());
}

#[tokio::test]
async fn test_delegate_only_injects_spawn_agent_with_shared_task_store() {
    let (provider, _) = MockProvider::with_capture("nested result", "mock");
    let store = Arc::new(SpawnTaskStore::default());
    let registry = registry_with_tools(&["spawn_status", "spawn_result", "spawn_list"]);
    let spawn_tool = SpawnAgentTool::new(make_empty_provider_registry(), provider, registry)
        .with_task_store(Arc::clone(&store));

    let filtered = spawn_tool.build_sub_tools(&[], &[], true, None);
    let nested_spawn = filtered.get("spawn_agent").expect("injected spawn_agent");
    let result = nested_spawn
        .execute(serde_json::json!({
            "task": "nested background task",
            "nonblocking": true,
            "_session_key": "session-a",
        }))
        .await
        .unwrap();
    let task_id = result["task_id"].as_str().unwrap();

    let result_tool = SpawnResultTool::new(store);
    let mut final_result = serde_json::Value::Null;
    for _ in 0..20 {
        final_result = result_tool
            .execute(serde_json::json!({
                "task_id": task_id,
                "_session_key": "session-a",
            }))
            .await
            .unwrap();
        if final_result["status"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    assert_eq!(final_result["status"], "completed");
    assert_eq!(final_result["text"], "nested result");
}

#[tokio::test]
async fn test_resolve_preset_uses_explicit_name() {
    let (provider, _) = MockProvider::with_capture("ok", "mock");
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
    let (provider, _) = MockProvider::with_capture("ok", "mock");
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
    let (provider, _) = MockProvider::with_capture("ok", "mock");
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

    let prompt = build_sub_agent_prompt(
        "find bugs",
        "in main.rs",
        Some(&preset),
        Some("scout"),
        true,
    );

    assert!(prompt.contains("You are scout (🔍)"));
    assert!(prompt.contains("Your style is thorough"));
    assert!(prompt.contains("Task: find bugs"));
    assert!(prompt.contains("Context: in main.rs"));
    assert!(prompt.contains("Focus on accuracy."));
}

#[test]
fn test_workspace_persona_injected_into_system_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let _guard = DataDirOverrideGuard::acquire(dir.path().to_path_buf());

    let workspace = moltis_config::agent_workspace_dir("research");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("SOUL.md"),
        "SOUL_MARKER_PARITY_7F3A — be curious.\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("AGENTS.md"),
        "<!-- mion:role -->\nROLE_MARKER_PARITY_7F3A\n<!-- /mion:role -->\nGUIDE_MARKER_PARITY_7F3A\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("GUIDELINES.md"),
        "Extra guidelines body for spawn parity.\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("IDENTITY.md"),
        "---\nname: Research Bot\nemoji: 🔬\ntheme: precise\n---\n",
    )
    .unwrap();

    let preset = AgentPreset {
        identity: AgentIdentity {
            name: Some("fallback-name".into()),
            emoji: None,
            theme: None,
        },
        system_prompt_suffix: Some("SUFFIX_MARKER_PARITY_7F3A".into()),
        ..Default::default()
    };

    let prompt = build_sub_agent_prompt(
        "summarize findings",
        "",
        Some(&preset),
        Some("research"),
        true,
    );

    assert!(prompt.contains("You are Research Bot (🔬)"));
    assert!(prompt.contains("Your style is precise"));
    assert!(prompt.contains("## Soul"));
    assert!(prompt.contains("SOUL_MARKER_PARITY_7F3A"));
    assert!(prompt.contains("## Workspace (AGENTS.md)"));
    assert!(prompt.contains("ROLE_MARKER_PARITY_7F3A"));
    assert!(prompt.contains("GUIDE_MARKER_PARITY_7F3A"));
    assert!(prompt.contains("## Guidelines"));
    assert!(prompt.contains("Extra guidelines body for spawn parity."));
    assert!(prompt.contains("SUFFIX_MARKER_PARITY_7F3A"));
    assert!(!prompt.contains("fallback-name"));
}

#[test]
fn test_workspace_persona_skipped_without_private_context() {
    let dir = tempfile::tempdir().unwrap();
    let _guard = DataDirOverrideGuard::acquire(dir.path().to_path_buf());

    let workspace = moltis_config::agent_workspace_dir("research");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("SOUL.md"), "SOUL_PRIVATE_LEAK\n").unwrap();
    std::fs::write(workspace.join("AGENTS.md"), "AGENTS_PRIVATE_LEAK\n").unwrap();
    std::fs::write(workspace.join("GUIDELINES.md"), "GUIDE_PRIVATE_LEAK\n").unwrap();
    // Root fallback that must also stay out of shared-channel spawns.
    std::fs::write(dir.path().join("AGENTS.md"), "ROOT_AGENTS_PRIVATE_LEAK\n").unwrap();
    std::fs::write(
        workspace.join("IDENTITY.md"),
        "---\nname: Research Bot\nemoji: 🔬\ntheme: precise\n---\n",
    )
    .unwrap();

    let preset = AgentPreset {
        identity: AgentIdentity {
            name: Some("public-name".into()),
            emoji: Some("📎".into()),
            theme: Some("brief".into()),
        },
        system_prompt_suffix: Some("SUFFIX_PRIVATE_LEAK".into()),
        ..Default::default()
    };

    let prompt = build_sub_agent_prompt(
        "summarize findings",
        "",
        Some(&preset),
        Some("research"),
        false,
    );

    assert!(prompt.contains("You are public-name (📎)"));
    assert!(prompt.contains("Your style is brief"));
    assert!(!prompt.contains("Research Bot"));
    assert!(!prompt.contains("## Soul"));
    assert!(!prompt.contains("SOUL_PRIVATE_LEAK"));
    assert!(!prompt.contains("## Workspace (AGENTS.md)"));
    assert!(!prompt.contains("AGENTS_PRIVATE_LEAK"));
    assert!(!prompt.contains("ROOT_AGENTS_PRIVATE_LEAK"));
    assert!(!prompt.contains("## Guidelines"));
    assert!(!prompt.contains("GUIDE_PRIVATE_LEAK"));
    assert!(!prompt.contains("SUFFIX_PRIVATE_LEAK"));
}

#[test]
fn test_workspace_agents_md_respects_size_cap() {
    let dir = tempfile::tempdir().unwrap();
    let _guard = DataDirOverrideGuard::acquire(dir.path().to_path_buf());

    let workspace = moltis_config::agent_workspace_dir("bulky");
    std::fs::create_dir_all(&workspace).unwrap();
    let max = moltis_config::discover_and_load()
        .chat
        .workspace_file_max_chars
        .max(1);
    let huge = "X".repeat(max + 64);
    std::fs::write(workspace.join("AGENTS.md"), &huge).unwrap();

    let prompt = build_sub_agent_prompt("trim guides", "", None, Some("bulky"), true);
    assert!(prompt.contains("## Workspace (AGENTS.md)"));
    assert!(prompt.contains("AGENTS.md truncated for prompt size"));
    assert!(prompt.contains(&"X".repeat(32)));
    assert!(!prompt.contains(&huge));
}

#[test]
fn test_no_identity_uses_default_prompt() {
    let _lock = DATA_DIR_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let managed_files = moltis_config::managed_files_dir().display().to_string();
    let prompt = build_sub_agent_prompt("do work", "", None, None, true);

    assert!(prompt.contains("You are a sub-agent"));
    assert!(prompt.contains("Task: do work"));
    assert!(prompt.contains("Managed Files: use $MOLTIS_FILES_DIR"));
    assert!(prompt.contains(crate::sandbox::SANDBOX_FILES_DIR));
    assert!(prompt.contains(&managed_files));
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

#[tokio::test]
async fn test_resolve_preset_rejects_target_outside_spawn_allow() {
    let (provider, _) = MockProvider::with_capture("ok", "mock");
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        Arc::new(ToolRegistry::new()),
    )
    .with_agents_config(agents_config_with_presets(None, &[
        ("coordinator", AgentPreset {
            spawn: PresetSpawnPolicy {
                allow: Some(vec!["research".to_string()]),
            },
            ..Default::default()
        }),
        ("research", AgentPreset::default()),
        ("writer", AgentPreset::default()),
    ]))
    .with_caller_agent_id("coordinator");

    let denied = spawn_tool
        .resolve_preset(&serde_json::json!({
            "preset": "writer",
            "_caller_agent_id": "coordinator",
        }))
        .await;
    assert!(denied.is_err());
    let message = denied.unwrap_err().to_string();
    assert!(message.contains("spawn_agent denied"), "{message}");
    assert!(message.contains("writer"), "{message}");
    assert!(message.contains("research"), "{message}");

    let allowed = spawn_tool
        .resolve_preset(&serde_json::json!({
            "preset": "research",
            "_caller_agent_id": "coordinator",
        }))
        .await
        .expect("research is allowlisted");
    assert_eq!(allowed.0.as_deref(), Some("research"));
}

#[tokio::test]
async fn test_resolve_preset_restricted_caller_rejects_missing_preset() {
    let (provider, _) = MockProvider::with_capture("ok", "mock");
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        Arc::new(ToolRegistry::new()),
    )
    .with_agents_config(agents_config_with_presets(None, &[
        ("solo", AgentPreset {
            spawn: PresetSpawnPolicy {
                allow: Some(vec![]),
            },
            ..Default::default()
        }),
        ("coordinator", AgentPreset {
            spawn: PresetSpawnPolicy {
                allow: Some(vec!["research".to_string()]),
            },
            ..Default::default()
        }),
        ("research", AgentPreset::default()),
    ]));

    let deny_all = spawn_tool
        .resolve_preset(&serde_json::json!({
            "_caller_agent_id": "solo",
        }))
        .await;
    assert!(deny_all.is_err());
    let deny_all_msg = deny_all.unwrap_err().to_string();
    assert!(
        deny_all_msg.contains("spawn_agent denied"),
        "{deny_all_msg}"
    );
    assert!(deny_all_msg.contains("spawn.allow = []"), "{deny_all_msg}");

    let allowlist = spawn_tool
        .resolve_preset(&serde_json::json!({
            "_caller_agent_id": "coordinator",
        }))
        .await;
    assert!(allowlist.is_err());
    let allowlist_msg = allowlist.unwrap_err().to_string();
    assert!(
        allowlist_msg.contains("requires an allowlisted preset"),
        "{allowlist_msg}"
    );

    // Unrestricted callers may still omit preset when default_preset is unset.
    let unrestricted = spawn_tool
        .resolve_preset(&serde_json::json!({
            "_caller_agent_id": "research",
        }))
        .await
        .expect("absent spawn.allow stays unrestricted without a named preset");
    assert_eq!(unrestricted.0, None);
}

#[tokio::test]
async fn test_resolve_preset_empty_spawn_allow_rejects_all() {
    let (provider, _) = MockProvider::with_capture("ok", "mock");
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        Arc::new(ToolRegistry::new()),
    )
    .with_agents_config(agents_config_with_presets(None, &[
        ("solo", AgentPreset {
            spawn: PresetSpawnPolicy {
                allow: Some(vec![]),
            },
            ..Default::default()
        }),
        ("research", AgentPreset::default()),
    ]));

    let denied = spawn_tool
        .resolve_preset(&serde_json::json!({
            "preset": "research",
            "_caller_agent_id": "solo",
        }))
        .await;
    assert!(denied.is_err());
    assert!(denied.unwrap_err().to_string().contains("spawn.allow = []"));
}

#[tokio::test]
async fn test_resolve_preset_absent_spawn_allow_unrestricted() {
    let (provider, _) = MockProvider::with_capture("ok", "mock");
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        Arc::new(ToolRegistry::new()),
    )
    .with_agents_config(agents_config_with_presets(None, &[
        ("legacy", AgentPreset::default()),
        ("research", AgentPreset::default()),
    ]));

    let (name, _) = spawn_tool
        .resolve_preset(&serde_json::json!({
            "preset": "research",
            "_caller_agent_id": "legacy",
        }))
        .await
        .expect("absent spawn.allow stays unrestricted");
    assert_eq!(name.as_deref(), Some("research"));
}

#[tokio::test]
async fn test_spawn_allow_bound_schema_lists_only_allowed_targets() {
    let (provider, _) = MockProvider::with_capture("ok", "mock");
    let agents = agents_config_with_presets(None, &[
        ("coordinator", AgentPreset {
            spawn: PresetSpawnPolicy {
                allow: Some(vec!["research".to_string()]),
            },
            ..Default::default()
        }),
        ("research", AgentPreset {
            identity: AgentIdentity {
                name: Some("Researcher".into()),
                emoji: None,
                theme: Some("thorough".into()),
            },
            ..Default::default()
        }),
        ("writer", AgentPreset::default()),
    ]);
    let agents_snapshot = agents.read().await.clone();
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        Arc::new(ToolRegistry::new()),
    )
    .with_agents_config(Arc::clone(&agents))
    .with_caller_agent_id("coordinator");

    let description = spawn_tool.description();
    assert!(description.contains("research"), "{description}");
    assert!(description.contains("only spawn"), "{description}");

    let schema = spawn_tool.parameters_schema();
    let preset_desc = schema["properties"]["preset"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(preset_desc.contains("research"), "{preset_desc}");
    assert!(
        !preset_desc.contains("writer") || preset_desc.contains("Allowed"),
        "{preset_desc}"
    );
    let enum_values = schema["properties"]["preset"]["enum"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(enum_values, vec![serde_json::Value::String(
        "research".into()
    )]);

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(spawn_tool));
    bind_spawn_agent_allowlist(&mut registry, "coordinator", &agents_snapshot);
    let bound = registry.get("spawn_agent").expect("bound spawn_agent");
    assert!(bound.description().contains("research"));
    assert_eq!(
        bound.parameters_schema()["properties"]["preset"]["enum"],
        serde_json::json!(["research"])
    );
}

#[tokio::test]
async fn test_execute_rejects_disallowed_preset_before_provider() {
    let (provider, _) = MockProvider::with_capture("should-not-run", "mock");
    let spawn_tool = SpawnAgentTool::new(
        make_empty_provider_registry(),
        provider,
        Arc::new(ToolRegistry::new()),
    )
    .with_agents_config(agents_config_with_presets(None, &[
        ("coordinator", AgentPreset {
            spawn: PresetSpawnPolicy {
                allow: Some(vec!["research".to_string()]),
            },
            ..Default::default()
        }),
        ("research", AgentPreset::default()),
        ("writer", AgentPreset::default()),
    ]));

    let result = spawn_tool
        .execute(serde_json::json!({
            "task": "write a blog post",
            "preset": "writer",
            "_caller_agent_id": "coordinator",
        }))
        .await;
    assert!(result.is_err());
    let message = result.unwrap_err().to_string();
    assert!(message.contains("spawn_agent denied"), "{message}");
}
