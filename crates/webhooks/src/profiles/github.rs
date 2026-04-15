//! GitHub source profile.

use axum::http::HeaderMap;

use crate::types::{AuthMode, EventCatalogEntry, NormalizedPayload};

use super::SourceProfile;

pub struct GitHubProfile;

impl SourceProfile for GitHubProfile {
    fn id(&self) -> &str {
        "github"
    }

    fn display_name(&self) -> &str {
        "GitHub"
    }

    fn default_auth_mode(&self) -> AuthMode {
        AuthMode::GithubHmacSha256
    }

    fn event_catalog(&self) -> Vec<EventCatalogEntry> {
        vec![
            EventCatalogEntry {
                event_type: "pull_request.opened".into(),
                description: "New pull request".into(),
                common_use_case: Some("Code review, labeling".into()),
            },
            EventCatalogEntry {
                event_type: "pull_request.synchronize".into(),
                description: "PR updated with new commits".into(),
                common_use_case: Some("Re-review".into()),
            },
            EventCatalogEntry {
                event_type: "pull_request.closed".into(),
                description: "PR closed or merged".into(),
                common_use_case: Some("Cleanup, changelog".into()),
            },
            EventCatalogEntry {
                event_type: "push".into(),
                description: "Commits pushed".into(),
                common_use_case: Some("CI trigger, deploy check".into()),
            },
            EventCatalogEntry {
                event_type: "issues.opened".into(),
                description: "New issue".into(),
                common_use_case: Some("Triage, auto-respond".into()),
            },
            EventCatalogEntry {
                event_type: "issues.labeled".into(),
                description: "Issue labeled".into(),
                common_use_case: Some("Route to agent".into()),
            },
            EventCatalogEntry {
                event_type: "issue_comment.created".into(),
                description: "New comment on issue/PR".into(),
                common_use_case: Some("Respond to questions".into()),
            },
            EventCatalogEntry {
                event_type: "pull_request_review.submitted".into(),
                description: "PR review submitted".into(),
                common_use_case: Some("Respond to review".into()),
            },
            EventCatalogEntry {
                event_type: "release.published".into(),
                description: "New release".into(),
                common_use_case: Some("Announce, post-release tasks".into()),
            },
            EventCatalogEntry {
                event_type: "check_suite.completed".into(),
                description: "CI finished".into(),
                common_use_case: Some("Report results".into()),
            },
            EventCatalogEntry {
                event_type: "workflow_run.completed".into(),
                description: "GitHub Actions workflow done".into(),
                common_use_case: Some("Post-CI analysis".into()),
            },
            EventCatalogEntry {
                event_type: "ping".into(),
                description: "Webhook configuration test".into(),
                common_use_case: Some("Verify setup".into()),
            },
        ]
    }

    fn parse_event_type(&self, headers: &HeaderMap, body: &[u8]) -> Option<String> {
        let event = headers
            .get("x-github-event")
            .and_then(|v| v.to_str().ok())?;

        // Combine event + action for finer granularity
        if let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(body)
            && let Some(action) = parsed.get("action").and_then(|a| a.as_str())
        {
            return Some(format!("{event}.{action}"));
        }
        Some(event.to_string())
    }

    fn parse_delivery_key(&self, headers: &HeaderMap, _body: &[u8]) -> Option<String> {
        headers
            .get("x-github-delivery")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
    }

    fn entity_key(&self, event_type: &str, body: &serde_json::Value) -> Option<String> {
        let repo = body
            .get("repository")
            .and_then(|r| r.get("full_name"))
            .and_then(|n| n.as_str())?;

        if event_type.starts_with("pull_request") {
            let number = body
                .get("pull_request")
                .or_else(|| body.get("number"))
                .and_then(|n| {
                    n.get("number")
                        .and_then(|nn| nn.as_u64())
                        .or_else(|| n.as_u64())
                })?;
            return Some(format!("github:{repo}:pr:{number}"));
        }
        if event_type.starts_with("issues") || event_type.starts_with("issue_comment") {
            let number = body
                .get("issue")
                .and_then(|i| i.get("number"))
                .and_then(|n| n.as_u64())?;
            return Some(format!("github:{repo}:issue:{number}"));
        }
        None
    }

    fn normalize_payload(&self, event_type: &str, body: &serde_json::Value) -> NormalizedPayload {
        let mut summary = format!("GitHub event: {event_type}\n\n");

        if let Some(repo) = body
            .get("repository")
            .and_then(|r| r.get("full_name"))
            .and_then(|n| n.as_str())
        {
            summary.push_str(&format!("Repository: {repo}\n"));
        }

        // PR-specific normalization
        if let Some(pr) = body.get("pull_request") {
            if let Some(number) = pr.get("number").and_then(|n| n.as_u64()) {
                let title = pr
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("(untitled)");
                summary.push_str(&format!("PR #{number}: \"{title}\"\n"));
            }
            if let Some(user) = pr
                .get("user")
                .and_then(|u| u.get("login"))
                .and_then(|l| l.as_str())
            {
                summary.push_str(&format!("  Author: @{user}\n"));
            }
            if let (Some(head), Some(base)) = (
                pr.get("head")
                    .and_then(|h| h.get("ref"))
                    .and_then(|r| r.as_str()),
                pr.get("base")
                    .and_then(|b| b.get("ref"))
                    .and_then(|r| r.as_str()),
            ) {
                summary.push_str(&format!("  Branch: {head} -> {base}\n"));
            }
            if let Some(url) = pr.get("html_url").and_then(|u| u.as_str()) {
                summary.push_str(&format!("  URL: {url}\n"));
            }
            if let Some(draft) = pr.get("draft").and_then(|d| d.as_bool()) {
                summary.push_str(&format!("  Draft: {draft}\n"));
            }
            if let Some(body_text) = pr.get("body").and_then(|b| b.as_str())
                && !body_text.is_empty()
            {
                let truncated = if body_text.len() > 500 {
                    format!("{}...", crate::normalize::truncate_str(body_text, 500))
                } else {
                    body_text.to_string()
                };
                summary.push_str(&format!("\nDescription:\n  {truncated}\n"));
            }
            if let (Some(add), Some(del)) = (
                pr.get("additions").and_then(|a| a.as_u64()),
                pr.get("deletions").and_then(|d| d.as_u64()),
            ) {
                let changed = pr
                    .get("changed_files")
                    .and_then(|c| c.as_u64())
                    .unwrap_or(0);
                summary.push_str(&format!("\nChanged files: {changed} (+{add} / -{del})\n"));
            }
        }

        // Issue-specific normalization
        if let Some(issue) = body.get("issue") {
            if let Some(number) = issue.get("number").and_then(|n| n.as_u64()) {
                let title = issue
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("(untitled)");
                summary.push_str(&format!("Issue #{number}: \"{title}\"\n"));
            }
            if let Some(user) = issue
                .get("user")
                .and_then(|u| u.get("login"))
                .and_then(|l| l.as_str())
            {
                summary.push_str(&format!("  Author: @{user}\n"));
            }
            if let Some(url) = issue.get("html_url").and_then(|u| u.as_str()) {
                summary.push_str(&format!("  URL: {url}\n"));
            }
        }

        // Comment normalization
        if let Some(comment) = body.get("comment") {
            if let Some(user) = comment
                .get("user")
                .and_then(|u| u.get("login"))
                .and_then(|l| l.as_str())
            {
                summary.push_str(&format!("\nComment by @{user}:\n"));
            }
            if let Some(body_text) = comment.get("body").and_then(|b| b.as_str()) {
                let truncated = if body_text.len() > 1000 {
                    format!("{}...", crate::normalize::truncate_str(body_text, 1000))
                } else {
                    body_text.to_string()
                };
                summary.push_str(&format!("  {truncated}\n"));
            }
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
        "## GitHub Webhook Setup\n\n\
         1. Go to your repository -> Settings -> Webhooks -> Add webhook\n\
         2. Payload URL: paste the webhook endpoint URL\n\
         3. Content type: `application/json`\n\
         4. Secret: paste the generated secret\n\
         5. Select events to trigger the webhook\n\n\
         For response actions (posting comments, reviews), create a GitHub Personal Access Token \
         with `repo` scope and add it in the Response Actions tab.\n\n\
         Reference: https://docs.github.com/en/webhooks"
    }
}
