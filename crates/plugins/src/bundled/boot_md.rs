//! `boot-md` hook: reads `BOOTSTRAP.md` / `BOOT.md` from the workspace on
//! `GatewayStart` and feeds them as startup user message content.

use std::path::PathBuf;

use {
    anyhow::Result,
    async_trait::async_trait,
    tracing::{debug, info},
};

use moltis_common::hooks::{HookAction, HookEvent, HookHandler, HookPayload};

/// Reads workspace startup markdown files and injects their content on startup.
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
        let bootstrap_path = self.workspace_dir.join("BOOTSTRAP.md");
        let boot_path = self.workspace_dir.join("BOOT.md");

        let bootstrap = read_non_empty_markdown(&bootstrap_path).await?;
        if let Some(content) = &bootstrap {
            info!(
                path = %bootstrap_path.display(),
                len = content.len(),
                "loaded BOOTSTRAP.md for startup injection"
            );
        } else {
            debug!(path = %bootstrap_path.display(), "no BOOTSTRAP.md found, skipping");
        }

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

        let mut startup_parts = Vec::new();
        if let Some(content) = bootstrap {
            startup_parts.push(content);
        }
        if let Some(content) = boot {
            startup_parts.push(content);
        }
        if startup_parts.is_empty() {
            return Ok(HookAction::Continue);
        }
        let startup_message = startup_parts.join("\n\n");

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
    let trimmed = content.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

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
    async fn boot_md_reads_bootstrap_and_boot() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("BOOTSTRAP.md"), "Bootstrap instructions").unwrap();
        std::fs::write(tmp.path().join("BOOT.md"), "Boot message").unwrap();

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
                assert_eq!(v["boot_message"], "Bootstrap instructions\n\nBoot message");
            },
            _ => panic!("expected ModifyPayload"),
        }
    }
}
