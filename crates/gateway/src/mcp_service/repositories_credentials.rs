use std::collections::BTreeSet;

use {serde_json::Value, tracing::instrument};

use crate::services::{ServiceError, ServiceResult};

use super::{
    CredentialCreateRequest, CredentialRemoveRequest, CredentialStore, CredentialUpdateRequest,
    LiveMcpService, parse_empty_params, parse_params, service_message,
};

impl LiveMcpService {
    #[instrument(skip(self, params))]
    pub(crate) async fn git_credentials_list_impl(&self, params: Value) -> ServiceResult {
        parse_empty_params(params)?;
        let store = self.credential_store().await?;
        let credentials = store
            .list_git_https_credentials()
            .await
            .map_err(service_message)?;
        let ssh_keys = store.list_ssh_keys().await.map_err(service_message)?;
        let ssh_targets = store
            .list_ssh_targets()
            .await
            .map_err(service_message)?
            .into_iter()
            .map(|target| serde_json::json!({
                "id": target.id,
                "label": target.label,
                "target": target.target,
                "port": target.port,
                "authMode": target.auth_mode,
                "keyId": target.key_id,
                "keyName": target.key_name,
                "hasKnownHost": target.known_host.as_deref().is_some_and(|pin| !pin.trim().is_empty()),
            }))
            .collect::<Vec<_>>();
        Ok(serde_json::json!({
            "credentials": credentials,
            "sshKeys": ssh_keys,
            "sshTargets": ssh_targets,
        }))
    }

    #[instrument(skip(self, params))]
    pub(crate) async fn git_credentials_create_impl(&self, params: Value) -> ServiceResult {
        let request: CredentialCreateRequest = parse_params(params)?;
        let _mutation = self.begin_managed_repository_mutation().await;
        let store = self.credential_store().await?;
        let id = store
            .create_git_https_credential(&request.host, &request.username, request.token)
            .await
            .map_err(service_message)?;
        credential_response(&store, id).await
    }

    #[instrument(skip(self, params))]
    pub(crate) async fn git_credentials_update_impl(&self, params: Value) -> ServiceResult {
        let request: CredentialUpdateRequest = parse_params(params)?;
        let _mutation = self.begin_managed_repository_mutation().await;
        let store = self.credential_store().await?;
        store
            .update_git_https_credential(
                request.id,
                &request.host,
                &request.username,
                request.token,
            )
            .await
            .map_err(service_message)?;
        credential_response(&store, request.id).await
    }

    #[instrument(skip(self, params))]
    pub(crate) async fn git_credentials_remove_impl(&self, params: Value) -> ServiceResult {
        let request: CredentialRemoveRequest = parse_params(params)?;
        let _mutation = self.begin_managed_repository_mutation().await;
        let references = self
            .manager
            .registry_snapshot()
            .await
            .repositories
            .values()
            .filter(|repository| repository.https_credential_id == Some(request.id))
            .count();
        self.credential_store()
            .await?
            .delete_git_https_credential(request.id, references)
            .await
            .map_err(service_message)?;
        Ok(serde_json::json!({ "removed": true }))
    }

    pub(crate) async fn managed_ssh_target_remove_impl(&self, id: i64) -> ServiceResult {
        let _mutation = self.begin_managed_repository_mutation().await;
        let references = self
            .manager
            .registry_snapshot()
            .await
            .repositories
            .values()
            .filter(|repository| repository.ssh_target_id == Some(id))
            .count();
        if references != 0 {
            return Err(ServiceError::invalid_request(
                "SSH target is still assigned to one or more managed MCP repositories",
            ));
        }
        self.credential_store()
            .await?
            .delete_ssh_target(id)
            .await
            .map_err(service_message)?;
        Ok(serde_json::json!({ "removed": true }))
    }

    pub(crate) async fn managed_ssh_key_remove_impl(&self, id: i64) -> ServiceResult {
        let _mutation = self.begin_managed_repository_mutation().await;
        let store = self.credential_store().await?;
        let referenced_targets = self
            .manager
            .registry_snapshot()
            .await
            .repositories
            .values()
            .filter_map(|repository| repository.ssh_target_id)
            .collect::<BTreeSet<_>>();
        let references = store
            .list_ssh_targets()
            .await
            .map_err(service_message)?
            .into_iter()
            .filter(|target| referenced_targets.contains(&target.id) && target.key_id == Some(id))
            .count();
        if references != 0 {
            return Err(ServiceError::invalid_request(
                "SSH key is still assigned to a managed MCP repository target",
            ));
        }
        store.delete_ssh_key(id).await.map_err(service_message)?;
        Ok(serde_json::json!({ "removed": true }))
    }
}

async fn credential_response(store: &CredentialStore, id: i64) -> ServiceResult {
    let entry = store
        .list_git_https_credentials()
        .await
        .map_err(service_message)?
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| ServiceError::message("HTTPS Git credential not found"))?;
    let warning = (!entry.encrypted).then_some(
        "credential is stored in plaintext because vault encryption is unavailable or sealed",
    );
    Ok(serde_json::json!({ "credential": entry, "storageWarning": warning }))
}
