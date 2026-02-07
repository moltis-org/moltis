//! One-time migration from separate plugins system to unified skills system.
//!
//! On startup, checks for `plugins-manifest.json` and migrates entries into
//! `skills-manifest.json`, moving directories from `installed-plugins/` to
//! `installed-skills/`. This is idempotent and non-fatal.

use std::path::Path;

use crate::manifest::ManifestStore;

/// Migrate plugins data into the unified skills system.
///
/// - Merges repos from `plugins-manifest.json` into `skills-manifest.json`
/// - Moves directories from `installed-plugins/` to `installed-skills/`
/// - Preserves all fields: `trusted`, `enabled`, `commit_sha`, `format`, `relative_path`
/// - Skips entries already in skills manifest (idempotent)
/// - Deletes old manifest + empty old dir after successful migration
///
/// Non-fatal: logs a warning if migration fails.
pub async fn migrate_plugins_to_skills(data_dir: &Path) {
    if let Err(e) = try_migrate(data_dir).await {
        tracing::warn!(%e, "plugins-to-skills migration failed (non-fatal)");
    }
}

async fn try_migrate(data_dir: &Path) -> anyhow::Result<()> {
    let plugins_manifest_path = data_dir.join("plugins-manifest.json");
    if !plugins_manifest_path.exists() {
        return Ok(());
    }

    tracing::info!("migrating plugins to unified skills system");

    let plugins_store = ManifestStore::new(plugins_manifest_path.clone());
    let plugins_manifest = plugins_store.load()?;

    if plugins_manifest.repos.is_empty() {
        // Empty manifest — just clean up.
        cleanup_old_files(&plugins_manifest_path, data_dir).await;
        return Ok(());
    }

    let skills_manifest_path = data_dir.join("skills-manifest.json");
    let skills_store = ManifestStore::new(skills_manifest_path);
    let mut skills_manifest = skills_store.load()?;

    let plugins_dir = data_dir.join("installed-plugins");
    let skills_dir = data_dir.join("installed-skills");
    tokio::fs::create_dir_all(&skills_dir).await?;

    let mut migrated = 0usize;
    for repo in &plugins_manifest.repos {
        // Skip if already in skills manifest.
        if skills_manifest.find_repo(&repo.source).is_some() {
            tracing::debug!(source = %repo.source, "already in skills manifest, skipping");
            continue;
        }

        // Move directory from installed-plugins/ to installed-skills/.
        let src_dir = plugins_dir.join(&repo.repo_name);
        let dst_dir = skills_dir.join(&repo.repo_name);

        if src_dir.is_dir()
            && !dst_dir.exists()
            && let Err(e) = tokio::fs::rename(&src_dir, &dst_dir).await
        {
            // rename fails across filesystems; fall back to copy + remove.
            tracing::debug!(%e, "rename failed, trying copy");
            copy_dir_recursive(&src_dir, &dst_dir).await?;
            let _ = tokio::fs::remove_dir_all(&src_dir).await;
        }

        skills_manifest.add_repo(repo.clone());
        migrated += 1;
    }

    if migrated > 0 {
        skills_store.save(&skills_manifest)?;
        tracing::info!(migrated, "migrated plugin repos into skills manifest");
    }

    cleanup_old_files(&plugins_manifest_path, data_dir).await;
    Ok(())
}

async fn cleanup_old_files(plugins_manifest_path: &Path, data_dir: &Path) {
    let _ = tokio::fs::remove_file(plugins_manifest_path).await;
    let plugins_dir = data_dir.join("installed-plugins");
    // Only remove if empty.
    if plugins_dir.is_dir()
        && let Ok(mut entries) = tokio::fs::read_dir(&plugins_dir).await
        && entries.next_entry().await.ok().flatten().is_none()
    {
        let _ = tokio::fs::remove_dir(&plugins_dir).await;
    }
}

async fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(dst).await?;
    let mut entries = tokio::fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            Box::pin(copy_dir_recursive(&src_path, &dst_path)).await?;
        } else {
            tokio::fs::copy(&src_path, &dst_path).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        formats::PluginFormat,
        types::{RepoEntry, SkillState, SkillsManifest},
    };

    use super::*;

    #[tokio::test]
    async fn test_migration_moves_repos() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();

        // Set up plugins manifest.
        let plugins_manifest = SkillsManifest {
            version: 1,
            repos: vec![RepoEntry {
                source: "anthropics/claude-plugins-official".into(),
                repo_name: "anthropics-claude-plugins-official".into(),
                installed_at_ms: 1000,
                commit_sha: Some("abc123def456".into()),
                format: PluginFormat::ClaudeCode,
                skills: vec![SkillState {
                    name: "pr-review-toolkit:code-reviewer".into(),
                    relative_path: "anthropics-claude-plugins-official".into(),
                    trusted: false,
                    enabled: true,
                }],
            }],
        };
        let plugins_store = ManifestStore::new(data_dir.join("plugins-manifest.json"));
        plugins_store.save(&plugins_manifest).unwrap();

        // Set up installed-plugins directory.
        let plugins_dir = data_dir.join("installed-plugins/anthropics-claude-plugins-official");
        std::fs::create_dir_all(plugins_dir.join(".claude-plugin")).unwrap();
        std::fs::write(
            plugins_dir.join(".claude-plugin/plugin.json"),
            r#"{"name":"test"}"#,
        )
        .unwrap();

        // Set up empty skills manifest.
        let skills_store = ManifestStore::new(data_dir.join("skills-manifest.json"));
        skills_store.save(&SkillsManifest::default()).unwrap();

        // Run migration.
        try_migrate(data_dir).await.unwrap();

        // Verify skills manifest has the migrated repo.
        let skills_manifest = skills_store.load().unwrap();
        assert_eq!(skills_manifest.repos.len(), 1);
        let repo = &skills_manifest.repos[0];
        assert_eq!(repo.source, "anthropics/claude-plugins-official");
        assert_eq!(repo.commit_sha.as_deref(), Some("abc123def456"));
        assert_eq!(repo.format, PluginFormat::ClaudeCode);
        assert!(!repo.skills[0].trusted);
        assert!(repo.skills[0].enabled);

        // Verify directory moved.
        assert!(
            data_dir
                .join(
                    "installed-skills/anthropics-claude-plugins-official/.claude-plugin/plugin.json"
                )
                .exists()
        );
        assert!(
            !data_dir
                .join("installed-plugins/anthropics-claude-plugins-official")
                .exists()
        );

        // Verify old manifest deleted.
        assert!(!data_dir.join("plugins-manifest.json").exists());
    }

    #[tokio::test]
    async fn test_migration_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();

        let repo = RepoEntry {
            source: "owner/repo".into(),
            repo_name: "owner-repo".into(),
            installed_at_ms: 500,
            commit_sha: None,
            format: PluginFormat::ClaudeCode,
            skills: vec![SkillState {
                name: "plugin:skill".into(),
                relative_path: "owner-repo".into(),
                trusted: true,
                enabled: true,
            }],
        };

        // Plugins manifest has the repo.
        let plugins_manifest = SkillsManifest {
            version: 1,
            repos: vec![repo.clone()],
        };
        let plugins_store = ManifestStore::new(data_dir.join("plugins-manifest.json"));
        plugins_store.save(&plugins_manifest).unwrap();

        // Skills manifest already has it too.
        let mut skills_manifest = SkillsManifest::default();
        skills_manifest.add_repo(repo);
        let skills_store = ManifestStore::new(data_dir.join("skills-manifest.json"));
        skills_store.save(&skills_manifest).unwrap();

        // Run migration — should not duplicate.
        try_migrate(data_dir).await.unwrap();

        let loaded = skills_store.load().unwrap();
        assert_eq!(loaded.repos.len(), 1);
    }

    #[tokio::test]
    async fn test_migration_noop_when_no_plugins_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        // No plugins-manifest.json — migration should be a no-op.
        try_migrate(tmp.path()).await.unwrap();
    }

    #[tokio::test]
    async fn test_migration_empty_manifest_cleans_up() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();

        // Empty plugins manifest.
        let plugins_store = ManifestStore::new(data_dir.join("plugins-manifest.json"));
        plugins_store.save(&SkillsManifest::default()).unwrap();
        assert!(data_dir.join("plugins-manifest.json").exists());

        // Empty plugins dir.
        std::fs::create_dir_all(data_dir.join("installed-plugins")).unwrap();

        try_migrate(data_dir).await.unwrap();

        // Old files cleaned up.
        assert!(!data_dir.join("plugins-manifest.json").exists());
        assert!(!data_dir.join("installed-plugins").exists());
    }
}
