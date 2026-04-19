#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{pin::Pin, sync::Arc};

use {
    super::*,
    moltis_agents::model::{CompletionResponse, StreamEvent, Usage, UserContent},
    moltis_memory::{
        config::MemoryConfig, embeddings::EmbeddingProvider, manager::MemoryManager,
        schema::run_migrations, store_sqlite::SqliteMemoryStore,
    },
    sqlx::SqlitePool,
    tempfile::TempDir,
    tokio_stream::Stream,
};

const KEYWORDS: [&str; 4] = ["dark", "spicy", "duplicate", "forget"];
static DATA_DIR_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct DataDirGuard;

impl Drop for DataDirGuard {
    fn drop(&mut self) {
        moltis_config::clear_data_dir();
    }
}

struct MockEmbedder;

#[async_trait]
impl EmbeddingProvider for MockEmbedder {
    async fn embed(&self, text: &str) -> moltis_memory::Result<Vec<f32>> {
        let lower = text.to_lowercase();
        Ok(KEYWORDS
            .iter()
            .map(|keyword| {
                if lower.contains(keyword) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect())
    }

    fn model_name(&self) -> &str {
        "mock-model"
    }

    fn dimensions(&self) -> usize {
        KEYWORDS.len()
    }

    fn provider_key(&self) -> &str {
        "mock"
    }
}

#[derive(Deserialize)]
struct ForgetPromptCandidateOwned {
    chunk_id: String,
    text: String,
}

struct ForgetPlannerProvider {
    needle: String,
}

#[async_trait]
impl LlmProvider for ForgetPlannerProvider {
    fn name(&self) -> &str {
        "mock-memory-forget"
    }

    fn id(&self) -> &str {
        "mock-memory-forget"
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        _tools: &[Value],
    ) -> anyhow::Result<CompletionResponse> {
        let user_text = messages
            .iter()
            .find_map(|message| match message {
                ChatMessage::User {
                    content: UserContent::Text(text),
                    ..
                } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or_default();
        let candidate_json = user_text
            .split("Candidate chunks:\n")
            .nth(1)
            .unwrap_or("[]");
        let candidates: Vec<ForgetPromptCandidateOwned> = serde_json::from_str(candidate_json)?;
        let actions: Vec<Value> = candidates
            .iter()
            .filter(|candidate| candidate.text.contains(&self.needle))
            .map(|candidate| {
                json!({
                    "chunk_id": candidate.chunk_id.clone(),
                    "reason": format!("matched '{}'", self.needle),
                })
            })
            .collect();

        Ok(CompletionResponse {
            text: Some(
                json!({
                    "needs_confirmation": false,
                    "rationale": format!("selected chunks containing '{}'", self.needle),
                    "actions": actions,
                })
                .to_string(),
            ),
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

struct NamedTool(&'static str);

#[async_trait]
impl AgentTool for NamedTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "stub"
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _params: Value) -> anyhow::Result<Value> {
        Ok(json!({}))
    }
}

async fn setup_agent_memory(
    agent_id: &str,
    content: &str,
    chunk_size: usize,
) -> (
    moltis_memory::runtime::DynMemoryRuntime,
    TempDir,
    std::path::PathBuf,
) {
    let tmp = TempDir::new().unwrap();
    moltis_config::set_data_dir(tmp.path().to_path_buf());

    let workspace = moltis_config::agent_workspace_dir(agent_id);
    std::fs::create_dir_all(workspace.join("memory")).unwrap();
    let memory_path = workspace.join("MEMORY.md");
    std::fs::write(&memory_path, content).unwrap();

    let pool = SqlitePool::connect(":memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let config = MemoryConfig {
        db_path: ":memory:".into(),
        data_dir: Some(tmp.path().to_path_buf()),
        memory_dirs: vec![workspace.join("MEMORY.md"), workspace.join("memory")],
        chunk_size,
        chunk_overlap: 0,
        vector_weight: 0.7,
        keyword_weight: 0.3,
        ..Default::default()
    };

    let manager = Arc::new(MemoryManager::new(
        config,
        Box::new(SqliteMemoryStore::new(pool)),
        Box::new(MockEmbedder),
    ));
    manager.sync().await.unwrap();
    (manager, tmp, memory_path)
}

#[tokio::test]
async fn memory_forget_deletes_selected_scoped_chunk() {
    let _lock = DATA_DIR_TEST_LOCK.lock().await;
    let _guard = DataDirGuard;
    let (manager, _tmp, memory_path) = setup_agent_memory(
        "writer",
        "Color preference dark mode\nFood preference spicy food\n",
        4,
    )
    .await;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(NamedTool("memory_forget")));
    install_agent_scoped_memory_tools(
        &mut registry,
        &manager,
        Arc::new(ForgetPlannerProvider {
            needle: "dark mode".to_string(),
        }),
        "writer",
        MemoryStyle::Hybrid,
        AgentMemoryWriteMode::Hybrid,
    );

    let tool = registry.get("memory_forget").unwrap();
    let result = tool
        .execute(json!({ "request": "forget that I prefer dark mode" }))
        .await
        .unwrap();

    assert_eq!(result["deleted"], json!(true));
    assert_eq!(result["needs_confirmation"], json!(false));
    assert_eq!(
        result["planned_matches"]
            .as_array()
            .map(|items| items.len()),
        Some(1)
    );
    assert_eq!(
        result["checkpointIds"].as_array().map(|items| items.len()),
        Some(1)
    );
    assert!(result["planned_matches"][0].get("path").is_none());

    let updated = std::fs::read_to_string(memory_path).unwrap();
    assert!(!updated.contains("dark mode"));
    assert!(updated.contains("spicy food"));
}

#[tokio::test]
async fn memory_forget_refuses_ambiguous_exact_text() {
    let _lock = DATA_DIR_TEST_LOCK.lock().await;
    let _guard = DataDirGuard;
    let (manager, _tmp, memory_path) = setup_agent_memory(
        "writer",
        "duplicate memory line\nduplicate memory line\n",
        4,
    )
    .await;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(NamedTool("memory_forget")));
    install_agent_scoped_memory_tools(
        &mut registry,
        &manager,
        Arc::new(ForgetPlannerProvider {
            needle: "duplicate".to_string(),
        }),
        "writer",
        MemoryStyle::Hybrid,
        AgentMemoryWriteMode::Hybrid,
    );

    let tool = registry.get("memory_forget").unwrap();
    let result = tool
        .execute(json!({ "request": "forget the duplicate memory line" }))
        .await
        .unwrap();

    assert_eq!(result["deleted"], json!(false));
    assert_eq!(result["needs_confirmation"], json!(true));
    assert!(!result["issues"].as_array().unwrap().is_empty());

    let updated = std::fs::read_to_string(memory_path).unwrap();
    assert_eq!(updated, "duplicate memory line\nduplicate memory line\n");
}

#[test]
fn count_exact_occurrences_accepts_line_ending_variants() {
    assert_eq!(count_exact_occurrences("alpha\r\nbeta\r\n", "alpha\n"), 1);
    assert_eq!(count_exact_occurrences("alpha\nbeta\n", "alpha\r\n"), 1);
}

#[tokio::test]
async fn memory_forget_reports_unreadable_files_as_issues() {
    let _lock = DATA_DIR_TEST_LOCK.lock().await;
    let _guard = DataDirGuard;
    let (manager, _tmp, memory_path) =
        setup_agent_memory("writer", "Color preference dark mode\n", 4).await;

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(NamedTool("memory_forget")));
    install_agent_scoped_memory_tools(
        &mut registry,
        &manager,
        Arc::new(ForgetPlannerProvider {
            needle: "dark mode".to_string(),
        }),
        "writer",
        MemoryStyle::Hybrid,
        AgentMemoryWriteMode::Hybrid,
    );

    std::fs::remove_file(&memory_path).unwrap();

    let tool = registry.get("memory_forget").unwrap();
    let result = tool
        .execute(json!({ "request": "forget that I prefer dark mode" }))
        .await
        .unwrap();

    assert_eq!(result["deleted"], json!(false));
    assert_eq!(result["needs_confirmation"], json!(true));
    assert!(result["planned_matches"].as_array().unwrap().is_empty());
    assert!(!result["issues"].as_array().unwrap().is_empty());
}
