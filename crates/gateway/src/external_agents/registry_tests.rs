#![allow(clippy::expect_used)]

use {super::*, moltis_sessions::metadata::SqliteSessionMetadata, sqlx::sqlite::SqlitePoolOptions};

#[tokio::test]
async fn default_registry_includes_minimax_code_acp() {
    let pool = SqlitePoolOptions::new()
        .connect_lazy("sqlite::memory:")
        .expect("create in-memory SQLite pool");
    let metadata = Arc::new(SqliteSessionMetadata::new(pool));
    let service = GatewayExternalAgentService::new(
        ExternalAgentsConfig::default(),
        metadata,
        Arc::new(ApprovalManager::default()),
    );

    assert!(
        service
            .registry
            .has_kind(AgentTransportKind::AcpMinimaxCode)
    );
    let agents = service.registry.list_agents().await;
    let minimax_code = agents
        .iter()
        .find(|agent| agent.kind == AgentTransportKind::AcpMinimaxCode)
        .expect("MiniMax Code ACP transport should be registered");
    assert_eq!(minimax_code.name, "ACP: MiniMax Code");
    assert!(minimax_code.is_acp);
}
