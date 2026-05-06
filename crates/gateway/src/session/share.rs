use super::*;

impl LiveSessionService {
    pub(super) async fn share_create_impl(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        let visibility = params
            .get("visibility")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<ShareVisibility>().ok())
            .unwrap_or(ShareVisibility::Public);

        let share_store = self
            .share_store
            .as_ref()
            .ok_or_else(|| "session share store not configured".to_string())?;

        let entry = self
            .metadata
            .get(key)
            .await
            .ok_or_else(|| format!("session '{key}' not found"))?;
        let history = self.store.read(key).await.map_err(ServiceError::message)?;

        let snapshot = ShareSnapshot {
            session_key: key.to_string(),
            session_label: entry.label.clone(),
            cutoff_message_count: history.len() as u32,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            messages: {
                let mut shared_messages = Vec::new();
                for msg in &history {
                    if let Some(shared) = to_shared_message(msg, key, self.store.as_ref()).await {
                        shared_messages.push(shared);
                    }
                }
                shared_messages
            },
        };
        let snapshot_json = serde_json::to_string(&snapshot)?;

        let created = share_store
            .create_or_replace(
                key,
                visibility,
                snapshot_json,
                snapshot.cutoff_message_count,
            )
            .await
            .map_err(ServiceError::message)?;

        // Persist a UI-only notice in the source session so users can see
        // the exact cutoff marker without affecting future LLM context.
        let boundary_notice = PersistedMessage::Notice {
            content: SHARE_BOUNDARY_NOTICE.to_string(),
            created_at: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            ),
        };
        if let Err(e) = self.store.append(key, &boundary_notice.to_value()).await {
            warn!(
                session_key = key,
                share_id = created.share.id,
                error = %e,
                "failed to persist share boundary notice; revoking share"
            );
            let _ = share_store.revoke(&created.share.id).await;
            return Err(format!("failed to persist share boundary notice: {e}").into());
        }
        match self.store.count(key).await {
            Ok(message_count) => {
                self.metadata.touch(key, message_count).await;
            },
            Err(e) => {
                warn!(session_key = key, error = %e, "failed to update session message count");
            },
        }

        Ok(serde_json::json!({
            "id": created.share.id,
            "sessionKey": created.share.session_key,
            "visibility": created.share.visibility.as_str(),
            "path": format!("/share/{}", created.share.id),
            "createdAt": created.share.created_at,
            "views": created.share.views,
            "snapshotMessageCount": created.share.snapshot_message_count,
            "accessKey": created.access_key,
            "notice": SHARE_BOUNDARY_NOTICE,
        }))
    }

    pub(super) async fn share_list_impl(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        let share_store = self
            .share_store
            .as_ref()
            .ok_or_else(|| "session share store not configured".to_string())?;

        let shares = share_store
            .list_for_session(key)
            .await
            .map_err(ServiceError::message)?;

        let items: Vec<Value> = shares
            .into_iter()
            .map(|share| {
                serde_json::json!({
                    "id": share.id,
                    "sessionKey": share.session_key,
                    "visibility": share.visibility.as_str(),
                    "path": format!("/share/{}", share.id),
                    "views": share.views,
                    "createdAt": share.created_at,
                    "revokedAt": share.revoked_at,
                })
            })
            .collect();
        Ok(serde_json::json!(items))
    }

    pub(super) async fn share_revoke_impl(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id' parameter".to_string())?;

        let share_store = self
            .share_store
            .as_ref()
            .ok_or_else(|| "session share store not configured".to_string())?;

        let revoked = share_store
            .revoke(id)
            .await
            .map_err(ServiceError::message)?;

        // Remove pre-rendered static files.
        let shares_dir = moltis_config::data_dir().join("shares");
        let _ = std::fs::remove_file(shares_dir.join(format!("{id}.html")));
        let _ = std::fs::remove_file(shares_dir.join(format!("{id}-og.svg")));

        Ok(serde_json::json!({ "revoked": revoked }))
    }
}
