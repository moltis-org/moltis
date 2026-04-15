//! GitLab source profile.

use axum::http::HeaderMap;

use crate::types::{AuthMode, EventCatalogEntry, NormalizedPayload};

use super::SourceProfile;

pub struct GitLabProfile;

impl SourceProfile for GitLabProfile {
    fn id(&self) -> &str {
        "gitlab"
    }

    fn display_name(&self) -> &str {
        "GitLab"
    }

    fn default_auth_mode(&self) -> AuthMode {
        AuthMode::GitlabToken
    }

    fn event_catalog(&self) -> Vec<EventCatalogEntry> {
        vec![
            EventCatalogEntry {
                event_type: "merge_request.open".into(),
                description: "New merge request".into(),
                common_use_case: Some("Code review".into()),
            },
            EventCatalogEntry {
                event_type: "merge_request.update".into(),
                description: "MR updated".into(),
                common_use_case: Some("Re-review".into()),
            },
            EventCatalogEntry {
                event_type: "merge_request.merge".into(),
                description: "MR merged".into(),
                common_use_case: Some("Post-merge actions".into()),
            },
            EventCatalogEntry {
                event_type: "push".into(),
                description: "Commits pushed".into(),
                common_use_case: Some("CI trigger".into()),
            },
            EventCatalogEntry {
                event_type: "note".into(),
                description: "New comment".into(),
                common_use_case: Some("Respond to questions".into()),
            },
            EventCatalogEntry {
                event_type: "issue.open".into(),
                description: "New issue".into(),
                common_use_case: Some("Triage".into()),
            },
            EventCatalogEntry {
                event_type: "pipeline".into(),
                description: "Pipeline status change".into(),
                common_use_case: Some("Report failures".into()),
            },
            EventCatalogEntry {
                event_type: "release".into(),
                description: "New release".into(),
                common_use_case: Some("Changelog, announce".into()),
            },
        ]
    }

    fn parse_event_type(&self, headers: &HeaderMap, body: &[u8]) -> Option<String> {
        let event = headers
            .get("x-gitlab-event")
            .and_then(|v| v.to_str().ok())?;

        // Normalize GitLab event names: "Merge Request Hook" -> "merge_request"
        let normalized = match event {
            "Merge Request Hook" => "merge_request",
            "Push Hook" => "push",
            "Issue Hook" => "issue",
            "Note Hook" => "note",
            "Pipeline Hook" => "pipeline",
            "Release Hook" => "release",
            "Tag Push Hook" => "tag_push",
            other => other,
        };

        // Add action for finer granularity
        if let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(body)
            && let Some(action) = parsed
                .get("object_attributes")
                .and_then(|oa| oa.get("action"))
                .and_then(|a| a.as_str())
        {
            return Some(format!("{normalized}.{action}"));
        }
        Some(normalized.to_string())
    }

    fn parse_delivery_key(&self, headers: &HeaderMap, body: &[u8]) -> Option<String> {
        if let Some(key) = headers.get("idempotency-key").and_then(|v| v.to_str().ok()) {
            return Some(key.to_string());
        }
        // Fall back to body hash
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(body);
        Some(format!("sha256:{}", hex::encode(hash)))
    }

    fn entity_key(&self, event_type: &str, body: &serde_json::Value) -> Option<String> {
        let project = body
            .get("project")
            .and_then(|p| p.get("path_with_namespace"))
            .and_then(|n| n.as_str())?;

        if event_type.starts_with("merge_request") {
            let iid = body
                .get("object_attributes")
                .and_then(|oa| oa.get("iid"))
                .and_then(|n| n.as_u64())?;
            return Some(format!("gitlab:{project}:mr:{iid}"));
        }
        if event_type.starts_with("issue") {
            let iid = body
                .get("object_attributes")
                .and_then(|oa| oa.get("iid"))
                .and_then(|n| n.as_u64())?;
            return Some(format!("gitlab:{project}:issue:{iid}"));
        }
        None
    }

    fn normalize_payload(&self, event_type: &str, body: &serde_json::Value) -> NormalizedPayload {
        let mut summary = format!("GitLab event: {event_type}\n\n");

        if let Some(project) = body
            .get("project")
            .and_then(|p| p.get("path_with_namespace"))
            .and_then(|n| n.as_str())
        {
            summary.push_str(&format!("Project: {project}\n"));
        }

        if let Some(oa) = body.get("object_attributes") {
            if let Some(iid) = oa.get("iid").and_then(|n| n.as_u64()) {
                let title = oa
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("(untitled)");
                let kind = if event_type.starts_with("merge_request") {
                    "MR"
                } else {
                    "Issue"
                };
                summary.push_str(&format!("{kind} !{iid}: \"{title}\"\n"));
            }
            if let Some(url) = oa.get("url").and_then(|u| u.as_str()) {
                summary.push_str(&format!("  URL: {url}\n"));
            }
        }

        if let Some(user) = body
            .get("user")
            .and_then(|u| u.get("username"))
            .and_then(|n| n.as_str())
        {
            summary.push_str(&format!("  Author: @{user}\n"));
        }

        summary.push_str("\nFull payload available via webhook_get_full_payload tool.");

        NormalizedPayload {
            summary,
            full_payload: body.clone(),
        }
    }

    fn has_response_tools(&self) -> bool {
        true
    }

    fn setup_guide(&self) -> &str {
        "## GitLab Webhook Setup\n\n\
         1. Go to your project -> Settings -> Webhooks\n\
         2. URL: paste the webhook endpoint URL\n\
         3. Secret token: paste the generated token\n\
         4. Select trigger events\n\n\
         For response actions, create a GitLab Personal Access Token with `api` scope.\n\n\
         Reference: https://docs.gitlab.com/user/project/integrations/webhooks/"
    }
}
