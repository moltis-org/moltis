//! `boot-md` hook: reads `BOOT.md` from the workspace on `GatewayStart` and
//! feeds it as startup user message content.

use std::path::PathBuf;

use {
    anyhow::Result,
    async_trait::async_trait,
    tracing::{debug, info},
};

use moltis_common::hooks::{HookAction, HookEvent, HookHandler, HookPayload};

/// Reads workspace BOOT.md and injects its content on startup.
pub struct BootMdHook {
    workspace_dir: PathBuf,
}

impl BootMdHook {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl HookHandler for BootMdHook {
    fn name(&self) -> &str {
        "boot-md"
    }

    fn events(&self) -> &[HookEvent] {
        &[HookEvent::GatewayStart]
    }

    fn priority(&self) -> i32 {
        100 // Run early
    }

    async fn handle(&self, _event: HookEvent, _payload: &HookPayload) -> Result<HookAction> {
        let boot_path = self.workspace_dir.join("BOOT.md");
        let boot = read_non_empty_markdown(&boot_path).await?;
        if let Some(content) = &boot {
            info!(
                path = %boot_path.display(),
                len = content.len(),
                "loaded BOOT.md for startup injection"
            );
        } else {
            debug!(path = %boot_path.display(), "no BOOT.md found, skipping");
        }

        let Some(startup_message) = boot else {
            return Ok(HookAction::Continue);
        };

        // Return the content as a ModifyPayload so the gateway can inject it.
        Ok(HookAction::ModifyPayload(serde_json::json!({
            "boot_message": startup_message,
        })))
    }
}

async fn read_non_empty_markdown(path: &std::path::Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = tokio::fs::read_to_string(path).await?;
    let trimmed = strip_leading_html_comments(&content).trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn strip_leading_html_comments(content: &str) -> &str {
    let mut rest = content;
    loop {
        let trimmed = rest.trim_start();
        if !trimmed.starts_with("<!--") {
            return trimmed;
        }
        let Some(end) = trimmed.find("-->") else {
            return "";
        };
        rest = &trimmed[end + 3..];
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn boot_md_reads_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("BOOT.md"), "Hello from BOOT.md").unwrap();

        let hook = BootMdHook::new(tmp.path().to_path_buf());
        let payload = HookPayload::GatewayStart {
            address: "127.0.0.1:8080".into(),
        };
        let result = hook
            .handle(HookEvent::GatewayStart, &payload)
            .await
            .unwrap();
        match result {
            HookAction::ModifyPayload(v) => {
                assert_eq!(v["boot_message"], "Hello from BOOT.md");
            },
            _ => panic!("expected ModifyPayload"),
        }
    }

    #[tokio::test]
    async fn boot_md_missing_file_continues() {
        let tmp = tempfile::tempdir().unwrap();
        let hook = BootMdHook::new(tmp.path().to_path_buf());
        let payload = HookPayload::GatewayStart {
            address: "127.0.0.1:8080".into(),
        };
        let result = hook
            .handle(HookEvent::GatewayStart, &payload)
            .await
            .unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[tokio::test]
    async fn boot_md_empty_file_continues() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("BOOT.md"), "  \n  ").unwrap();

        let hook = BootMdHook::new(tmp.path().to_path_buf());
        let payload = HookPayload::GatewayStart {
            address: "127.0.0.1:8080".into(),
        };
        let result = hook
            .handle(HookEvent::GatewayStart, &payload)
            .await
            .unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[tokio::test]
    async fn boot_md_comment_only_file_continues() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("BOOT.md"), "<!-- startup notes -->").unwrap();

        let hook = BootMdHook::new(tmp.path().to_path_buf());
        let payload = HookPayload::GatewayStart {
            address: "127.0.0.1:8080".into(),
        };
        let result = hook
            .handle(HookEvent::GatewayStart, &payload)
            .await
            .unwrap();
        assert!(matches!(result, HookAction::Continue));
    }
}
