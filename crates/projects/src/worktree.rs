use std::path::{Path, PathBuf};

use {
    anyhow::{Context, Result, bail},
    tokio::process::Command,
    tracing::{debug, warn},
};

// NOTE: This module intentionally shells out to `git` for worktree porcelain
// operations (`worktree add/remove/list`, branch delete) because these flows are
// not yet practical with stable in-process gitoxide APIs in this code path.

const WORKTREE_DIR: &str = ".moltis-worktrees";

/// Information about an active worktree.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
}

/// Manages git worktrees for session isolation.
pub struct WorktreeManager;

impl WorktreeManager {
    /// Create a new worktree for a session, branching from HEAD.
    pub async fn create(project_dir: &Path, session_id: &str) -> Result<PathBuf> {
        Self::ensure_git_repo(project_dir).await?;

        let branch = format!("moltis/{session_id}");
        let wt_dir = project_dir.join(WORKTREE_DIR).join(session_id);

        if wt_dir.exists() {
            return Ok(wt_dir);
        }

        let output = Command::new("git")
            .args(["worktree", "add", "-b", &branch])
            .arg(&wt_dir)
            .current_dir(project_dir)
            .output()
            .await
            .context("failed to run git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git worktree add failed: {stderr}");
        }

        debug!(worktree = %wt_dir.display(), branch = %branch, "created worktree");
        Ok(wt_dir)
    }

    /// Create a worktree from an existing branch.
    pub async fn create_from_branch(
        project_dir: &Path,
        session_id: &str,
        branch: &str,
    ) -> Result<PathBuf> {
        Self::ensure_git_repo(project_dir).await?;

        let wt_dir = project_dir.join(WORKTREE_DIR).join(session_id);
        if wt_dir.exists() {
            return Ok(wt_dir);
        }

        let output = Command::new("git")
            .args(["worktree", "add"])
            .arg(&wt_dir)
            .arg(branch)
            .current_dir(project_dir)
            .output()
            .await
            .context("failed to run git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git worktree add from branch failed: {stderr}");
        }

        debug!(worktree = %wt_dir.display(), branch = %branch, "created worktree from branch");
        Ok(wt_dir)
    }

    /// Remove a worktree and clean up the branch.
    /// If the branch was pushed to a remote, only the worktree is removed.
    /// If not pushed, the branch is also deleted.
    pub async fn cleanup(project_dir: &Path, session_id: &str) -> Result<()> {
        let wt_dir = project_dir.join(WORKTREE_DIR).join(session_id);
        let branch = format!("moltis/{session_id}");

        if wt_dir.exists() {
            let output = Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(&wt_dir)
                .current_dir(project_dir)
                .output()
                .await
                .context("failed to run git worktree remove")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(stderr = %stderr, "git worktree remove failed, trying manual cleanup");
                // Force remove directory as fallback
                tokio::fs::remove_dir_all(&wt_dir).await.ok();
                // Prune stale worktree entries
                Command::new("git")
                    .args(["worktree", "prune"])
                    .current_dir(project_dir)
                    .output()
                    .await
                    .ok();
            }
        }

        // Check if branch was pushed
        let pushed = Self::is_branch_pushed(project_dir, &branch).await;
        if pushed {
            debug!(branch = %branch, "branch was pushed, keeping it");
        } else {
            // Delete local branch
            let output = Command::new("git")
                .args(["branch", "-D", &branch])
                .current_dir(project_dir)
                .output()
                .await;
            match output {
                Ok(o) if o.status.success() => {
                    debug!(branch = %branch, "deleted local branch");
                },
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    debug!(branch = %branch, stderr = %stderr, "branch delete skipped");
                },
                Err(e) => {
                    warn!(branch = %branch, error = %e, "failed to delete branch");
                },
            }
        }

        Ok(())
    }

    /// List active worktrees for a project.
    pub async fn list(project_dir: &Path) -> Result<Vec<WorktreeInfo>> {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(project_dir)
            .output()
            .await
            .context("failed to run git worktree list")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut worktrees = Vec::new();
        let mut current_path: Option<PathBuf> = None;
        let mut current_branch: Option<String> = None;

        for line in stdout.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                current_path = Some(PathBuf::from(path));
            } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                current_branch = Some(branch.to_string());
            } else if line.is_empty() {
                if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take())
                    && branch.starts_with("moltis/")
                {
                    worktrees.push(WorktreeInfo { path, branch });
                }
                current_path = None;
                current_branch = None;
            }
        }
        // Handle last entry (no trailing blank line)
        if let (Some(path), Some(branch)) = (current_path, current_branch)
            && branch.starts_with("moltis/")
        {
            worktrees.push(WorktreeInfo { path, branch });
        }

        Ok(worktrees)
    }

    /// Run the project's setup command in a worktree directory.
    ///
    /// Injects `MOLTIS_WORKSPACE_NAME` (session id) and `MOLTIS_ROOT_PATH`
    /// (project root) as environment variables.
    pub async fn run_setup(
        wt_dir: &Path,
        setup_command: &str,
        project_dir: &Path,
        session_id: &str,
    ) -> Result<()> {
        debug!(cmd = %setup_command, dir = %wt_dir.display(), "running setup command");
        let output = Command::new("sh")
            .args(["-c", setup_command])
            .current_dir(wt_dir)
            .env("MOLTIS_WORKSPACE_NAME", session_id)
            .env("MOLTIS_ROOT_PATH", project_dir.as_os_str())
            .output()
            .await
            .context("failed to run setup command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(cmd = %setup_command, stderr = %stderr, "setup command failed");
        }
        Ok(())
    }

    /// Run a teardown command in a worktree directory before cleanup.
    pub async fn run_teardown(
        wt_dir: &Path,
        teardown_command: &str,
        project_dir: &Path,
        session_id: &str,
    ) -> Result<()> {
        debug!(cmd = %teardown_command, dir = %wt_dir.display(), "running teardown command");
        let output = Command::new("sh")
            .args(["-c", teardown_command])
            .current_dir(wt_dir)
            .env("MOLTIS_WORKSPACE_NAME", session_id)
            .env("MOLTIS_ROOT_PATH", project_dir.as_os_str())
            .output()
            .await
            .context("failed to run teardown command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(cmd = %teardown_command, stderr = %stderr, "teardown command failed");
        }
        Ok(())
    }

    /// Check if a worktree has uncommitted changes.
    pub async fn has_uncommitted_changes(wt_dir: &Path) -> Result<bool> {
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(wt_dir)
            .output()
            .await
            .context("failed to run git status")?;
        Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
    }

    /// Check if a worktree branch has unpushed commits.
    pub async fn has_unpushed_commits(wt_dir: &Path, branch: &str) -> Result<bool> {
        let output = Command::new("git")
            .args(["log", &format!("@{{u}}..{branch}"), "--oneline"])
            .current_dir(wt_dir)
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => {
                Ok(!String::from_utf8_lossy(&o.stdout).trim().is_empty())
            },
            // No upstream tracking branch means nothing is pushed.
            _ => Ok(true),
        }
    }

    /// Resolve the default base branch for a project (main, master, develop, trunk).
    pub async fn resolve_base_branch(project_dir: &Path) -> Result<String> {
        for candidate in ["main", "master", "develop", "trunk"] {
            let output = Command::new("git")
                .args(["rev-parse", "--verify", candidate])
                .current_dir(project_dir)
                .output()
                .await;
            if let Ok(o) = output
                && o.status.success()
            {
                return Ok(candidate.to_string());
            }
        }
        bail!("no default branch found (tried main, master, develop, trunk)")
    }

    /// Create a worktree from a specific base branch.
    pub async fn create_from_base(
        project_dir: &Path,
        session_id: &str,
        base_branch: &str,
    ) -> Result<PathBuf> {
        Self::ensure_git_repo(project_dir).await?;

        let branch = format!("moltis/{session_id}");
        let wt_dir = project_dir.join(WORKTREE_DIR).join(session_id);

        if wt_dir.exists() {
            return Ok(wt_dir);
        }

        let output = Command::new("git")
            .args(["worktree", "add", "-b", &branch])
            .arg(&wt_dir)
            .arg(base_branch)
            .current_dir(project_dir)
            .output()
            .await
            .context("failed to run git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git worktree add from base branch failed: {stderr}");
        }

        debug!(worktree = %wt_dir.display(), branch = %branch, base = %base_branch, "created worktree from base");
        Ok(wt_dir)
    }

    async fn ensure_git_repo(dir: &Path) -> Result<()> {
        if !dir.join(".git").exists() {
            bail!("{} is not a git repository", dir.display());
        }
        Ok(())
    }

    async fn is_branch_pushed(project_dir: &Path, branch: &str) -> bool {
        let output = Command::new("git")
            .args(["branch", "-r", "--list", &format!("origin/{branch}")])
            .current_dir(project_dir)
            .output()
            .await;
        match output {
            Ok(o) => !String::from_utf8_lossy(&o.stdout).trim().is_empty(),
            Err(_) => false,
        }
    }
}

/// Copy `.moltis/` config directory from project root to a worktree, if it
/// exists in the project but not in the worktree.
pub fn copy_project_config(project_dir: &Path, wt_dir: &Path) -> Result<()> {
    let src = project_dir.join(".moltis");
    let dst = wt_dir.join(".moltis");

    if !src.exists() || dst.exists() {
        return Ok(());
    }

    copy_dir_recursive(&src, &dst)?;
    debug!(src = %src.display(), dst = %dst.display(), "copied project config to worktree");
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn init_test_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir.path())
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .await
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir.path())
            .output()
            .await
            .unwrap();
        dir
    }

    #[tokio::test]
    async fn test_create_and_list_worktree() {
        let dir = init_test_repo().await;
        let wt = WorktreeManager::create(dir.path(), "test-session")
            .await
            .unwrap();
        assert!(wt.exists());

        let list = WorktreeManager::list(dir.path()).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].branch, "moltis/test-session");
    }

    #[tokio::test]
    async fn test_create_idempotent() {
        let dir = init_test_repo().await;
        let wt1 = WorktreeManager::create(dir.path(), "sess").await.unwrap();
        let wt2 = WorktreeManager::create(dir.path(), "sess").await.unwrap();
        assert_eq!(wt1, wt2);
    }

    #[tokio::test]
    async fn test_cleanup_worktree() {
        let dir = init_test_repo().await;
        WorktreeManager::create(dir.path(), "sess").await.unwrap();
        WorktreeManager::cleanup(dir.path(), "sess").await.unwrap();
        let list = WorktreeManager::list(dir.path()).await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_not_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = WorktreeManager::create(dir.path(), "x").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_setup_with_env_vars() {
        let dir = init_test_repo().await;
        let wt = WorktreeManager::create(dir.path(), "env-test")
            .await
            .unwrap();

        // Write a setup script that dumps env vars to a file.
        let marker = wt.join("env_check.txt");
        let cmd = format!(
            "echo \"$MOLTIS_WORKSPACE_NAME:$MOLTIS_ROOT_PATH\" > {}",
            marker.display()
        );
        WorktreeManager::run_setup(&wt, &cmd, dir.path(), "env-test")
            .await
            .unwrap();

        let content = std::fs::read_to_string(&marker).unwrap();
        assert!(content.contains("env-test"));
        assert!(content.contains(&dir.path().display().to_string()));
    }

    #[tokio::test]
    async fn test_run_teardown() {
        let dir = init_test_repo().await;
        let wt = WorktreeManager::create(dir.path(), "td-test")
            .await
            .unwrap();

        let marker = wt.join("teardown_ran.txt");
        let cmd = format!("touch {}", marker.display());
        WorktreeManager::run_teardown(&wt, &cmd, dir.path(), "td-test")
            .await
            .unwrap();
        assert!(marker.exists());
    }

    #[tokio::test]
    async fn test_copy_project_config() {
        let dir = init_test_repo().await;
        let wt = WorktreeManager::create(dir.path(), "cfg-test")
            .await
            .unwrap();

        // Create .moltis/config.toml in project root.
        let config_dir = dir.path().join(".moltis");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("config.toml"), "key = 'value'").unwrap();

        copy_project_config(dir.path(), &wt).unwrap();

        assert!(wt.join(".moltis/config.toml").exists());
        let content = std::fs::read_to_string(wt.join(".moltis/config.toml")).unwrap();
        assert_eq!(content, "key = 'value'");
    }

    #[tokio::test]
    async fn test_copy_project_config_skips_if_exists() {
        let dir = init_test_repo().await;
        let wt = WorktreeManager::create(dir.path(), "cfg-skip")
            .await
            .unwrap();

        // Create .moltis in both project and worktree.
        std::fs::create_dir_all(dir.path().join(".moltis")).unwrap();
        std::fs::write(dir.path().join(".moltis/config.toml"), "original").unwrap();
        std::fs::create_dir_all(wt.join(".moltis")).unwrap();
        std::fs::write(wt.join(".moltis/config.toml"), "existing").unwrap();

        copy_project_config(dir.path(), &wt).unwrap();

        // Should not overwrite.
        let content = std::fs::read_to_string(wt.join(".moltis/config.toml")).unwrap();
        assert_eq!(content, "existing");
    }

    #[tokio::test]
    async fn test_has_uncommitted_changes() {
        let dir = init_test_repo().await;
        let wt = WorktreeManager::create(dir.path(), "dirty-test")
            .await
            .unwrap();

        // Clean worktree.
        assert!(!WorktreeManager::has_uncommitted_changes(&wt).await.unwrap());

        // Create an untracked file.
        std::fs::write(wt.join("new_file.txt"), "hello").unwrap();
        assert!(WorktreeManager::has_uncommitted_changes(&wt).await.unwrap());
    }

    #[tokio::test]
    async fn test_resolve_base_branch() {
        let dir = init_test_repo().await;
        // The default branch from `git init` is typically "main" or "master".
        let base = WorktreeManager::resolve_base_branch(dir.path()).await;
        assert!(base.is_ok());
        let branch = base.unwrap();
        assert!(
            branch == "main" || branch == "master",
            "unexpected branch: {branch}"
        );
    }

    #[tokio::test]
    async fn test_create_from_base() {
        let dir = init_test_repo().await;
        let base = WorktreeManager::resolve_base_branch(dir.path())
            .await
            .unwrap();
        let wt = WorktreeManager::create_from_base(dir.path(), "base-test", &base)
            .await
            .unwrap();
        assert!(wt.exists());

        let list = WorktreeManager::list(dir.path()).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].branch, "moltis/base-test");
    }
}
