use std::sync::Arc;

use {
    async_trait::async_trait,
    moltis_external_agents::{AcpPermissionHandler, AcpPermissionOptionKind, AcpPermissionRequest},
};

use moltis_tools::approval::{ApprovalDecision, ApprovalManager};

pub(super) struct GatewayAcpPermissionHandler {
    approval_manager: Arc<ApprovalManager>,
}

impl GatewayAcpPermissionHandler {
    pub(super) fn new(approval_manager: Arc<ApprovalManager>) -> Self {
        Self { approval_manager }
    }
}

#[async_trait]
impl AcpPermissionHandler for GatewayAcpPermissionHandler {
    async fn select_option(&self, request: AcpPermissionRequest) -> anyhow::Result<Option<String>> {
        let command = format!(
            "ACP permission requested for {} [{}]",
            request.tool_call,
            request
                .options
                .iter()
                .map(|option| option.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let session_key = request.moltis_session_key.as_deref();
        let (request_id, decision_rx) = self
            .approval_manager
            .create_request(&command, session_key)
            .await;
        if let Some(session_key) = request.moltis_session_key.as_deref() {
            tracing::info!(request_id, session_key, "ACP permission request is pending");
        }
        match self.approval_manager.wait_for_decision(decision_rx).await {
            ApprovalDecision::Approved => Ok(select_allowed_acp_option(&request)),
            ApprovalDecision::Denied | ApprovalDecision::Timeout => {
                Ok(select_rejected_acp_option(&request))
            },
        }
    }
}

pub(super) fn select_allowed_acp_option(request: &AcpPermissionRequest) -> Option<String> {
    request
        .options
        .iter()
        .find(|option| option.kind == AcpPermissionOptionKind::AllowOnce)
        .or_else(|| {
            request
                .options
                .iter()
                .find(|option| option.kind == AcpPermissionOptionKind::AllowAlways)
        })
        .map(|option| option.id.clone())
}

pub(super) fn select_rejected_acp_option(request: &AcpPermissionRequest) -> Option<String> {
    request
        .options
        .iter()
        .find(|option| option.kind == AcpPermissionOptionKind::RejectOnce)
        .or_else(|| {
            request
                .options
                .iter()
                .find(|option| option.kind == AcpPermissionOptionKind::RejectAlways)
        })
        .map(|option| option.id.clone())
}
