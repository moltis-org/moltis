//! `Glob` tool — fast pattern-based file matching.
//!
//! Walks the given directory tree using `ignore::WalkBuilder` (respects
//! `.gitignore` by default) and matches entries against a `globset::Glob`
//! pattern. Results are sorted by modification time descending so the most
//! recently changed files come first — a small detail that matters when an
//! agent is navigating an unfamiliar codebase.

use {
    async_trait::async_trait,
    globset::{Glob as GlobPattern, GlobMatcher},
    ignore::WalkBuilder,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::{
        path::{Path, PathBuf},
        sync::Arc,
        time::SystemTime,
    },
    tracing::instrument,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, labels, tools as tools_metrics};

use crate::{
    Result,
    error::Error,
    fs::shared::{FsPathPolicy, enforce_path_policy_deny_only, require_absolute, session_key_from},
    sandbox::{SandboxRouter, file_system::sandbox_file_system_for_session},
};

/// Maximum number of entries returned by a single `Glob` call.
const DEFAULT_GLOB_LIMIT: usize = 1000;

/// Native `Glob` tool implementation.
pub struct GlobTool {
    /// Optional default root used when the LLM call omits `path`.
    workspace_root: Option<PathBuf>,
    /// Optional allow/deny path policy. Applied to the walk root (call
    /// is rejected entirely if the root is denied) and to each matching
    /// file (denied files are filtered out of results).
    path_policy: Option<FsPathPolicy>,
    /// Whether to respect `.gitignore` / `.ignore` / `.git/info/exclude`
    /// while walking. Default `true`.
    respect_gitignore: bool,
    /// When set and the session is sandboxed, `find` the sandbox's
    /// filesystem instead of the host.
    sandbox_router: Option<Arc<SandboxRouter>>,
}

impl Default for GlobTool {
    fn default() -> Self {
        Self {
            workspace_root: None,
            path_policy: None,
            respect_gitignore: true,
            sandbox_router: None,
        }
    }
}

impl GlobTool {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the default search root for calls that omit `path`.
    ///
    /// Must be an absolute path. Relative roots are rejected so the tool
    /// can't silently walk the gateway process cwd.
    #[must_use]
    pub fn with_workspace_root(mut self, root: PathBuf) -> Self {
        self.workspace_root = Some(root);
        self
    }

    /// Attach an allow/deny path policy.
    #[must_use]
    pub fn with_path_policy(mut self, policy: FsPathPolicy) -> Self {
        self.path_policy = Some(policy);
        self
    }

    /// Override gitignore respect. Default `true`.
    #[must_use]
    pub fn with_respect_gitignore(mut self, respect: bool) -> Self {
        self.respect_gitignore = respect;
        self
    }

    /// Attach a shared [`SandboxRouter`]. When the session is sandboxed
    /// Glob lists files via the bridge's `find` helper and applies the
    /// glob matcher on the host side; `.gitignore` semantics are lost
    /// in the sandbox walk (the container `find` doesn't parse them).
    #[must_use]
    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }

    #[instrument(skip(self), fields(pattern = %pattern, path = ?path))]
    fn glob_impl(&self, pattern: &str, path: Option<&Path>) -> Result<Value> {
        let matcher = GlobPattern::new(pattern)
            .map_err(|e| Error::message(format!("invalid glob pattern '{pattern}': {e}")))?
            .compile_matcher();

        let root = match path {
            Some(p) => p.to_path_buf(),
            None => self.workspace_root.clone().ok_or_else(|| {
                Error::message(
                    "Glob requires an absolute 'path' argument (no workspace root is configured)",
                )
            })?,
        };

        if !root.is_absolute() {
            return Err(Error::message(format!(
                "Glob 'path' must be absolute (got '{}')",
                root.display()
            )));
        }

        let root_canonical = std::fs::canonicalize(&root).map_err(|e| {
            Error::message(format!(
                "cannot resolve glob root '{}': {e}",
                root.display()
            ))
        })?;

        // Reject the entire call only if the walk root is explicitly
        // denied. Allow-list filtering happens per-file below — a
        // directory root typically won't match a file-granular allow
        // glob even if its children do.
        if let Some(ref policy) = self.path_policy
            && let Some(payload) = enforce_path_policy_deny_only(policy, &root_canonical)
        {
            return Ok(payload);
        }

        let mut results: Vec<(PathBuf, SystemTime)> = Vec::new();

        let walker = WalkBuilder::new(&root_canonical)
            .hidden(false) // allow dotfiles; gitignore still filters .git/
            .git_ignore(self.respect_gitignore)
            .git_exclude(self.respect_gitignore)
            .git_global(self.respect_gitignore)
            .ignore(self.respect_gitignore)
            .build();

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let abs_path = entry.path();
            // Match against the path relative to the walk root, so patterns
            // like `**/*.rs` work naturally.
            let relative = abs_path.strip_prefix(&root_canonical).unwrap_or(abs_path);
            if !matcher.is_match(relative) {
                continue;
            }
            // Per-file path policy filter so allow/deny can carve out
            // sub-trees underneath the walk root.
            if let Some(ref policy) = self.path_policy
                && policy.check(abs_path).is_some()
            {
                continue;
            }
            let mtime = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            results.push((abs_path.to_path_buf(), mtime));
        }

        // Sort by mtime descending.
        results.sort_by(|a, b| b.1.cmp(&a.1));

        let truncated = results.len() > DEFAULT_GLOB_LIMIT;
        if truncated {
            results.truncate(DEFAULT_GLOB_LIMIT);
        }

        let paths: Vec<String> = results
            .into_iter()
            .map(|(p, _)| p.to_string_lossy().into_owned())
            .collect();

        #[cfg(feature = "metrics")]
        counter!(
            tools_metrics::EXECUTIONS_TOTAL,
            labels::TOOL => "Glob".to_string(),
            labels::SUCCESS => "true".to_string()
        )
        .increment(1);

        Ok(json!({
            "paths": paths,
            "truncated": truncated,
            "root": root_canonical.to_string_lossy(),
        }))
    }
}

#[async_trait]
impl AgentTool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Results are sorted by modification \
         time, most recent first. Respects .gitignore by default. Supports \
         standard glob syntax (`*.rs`, `**/*.ts`, `src/**/*.{js,jsx}`)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match (e.g. '**/*.rs', 'src/**/*.ts')."
                },
                "path": {
                    "type": "string",
                    "description": "Absolute path of the directory to search in. Required unless a workspace root is configured for this tool instance."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let pattern = params
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'pattern' parameter"))?;
        if let Some(raw) = params.get("path").and_then(Value::as_str) {
            require_absolute(raw, "path")?;
        }
        let pattern = pattern.to_string();
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let session_key = session_key_from(&params).to_string();

        // Sandbox dispatch: shell `find ROOT -type f` into the container
        // and apply the glob matcher + path policy on the host side.
        // Gitignore semantics are not honored in the sandbox walk; that
        // can be a follow-up.
        if let Some(ref router) = self.sandbox_router
            && router.is_sandboxed(&session_key).await
        {
            let root = match path.as_ref() {
                Some(p) => p.clone(),
                None => self.workspace_root.clone().ok_or_else(|| {
                    Error::message(
                        "Glob requires an absolute 'path' argument (no workspace root is configured)",
                    )
                })?,
            };
            // Root deny-only check, matching the host path.
            if let Some(ref policy) = self.path_policy
                && let Some(payload) = enforce_path_policy_deny_only(policy, &root)
            {
                return Ok(payload);
            }
            let root_str = root
                .to_str()
                .ok_or_else(|| Error::message("Glob 'path' contains invalid UTF-8"))?;
            let sandbox_fs = sandbox_file_system_for_session(router, &session_key).await?;
            let listed = sandbox_fs.list_files(root_str).await?;
            let matcher: GlobMatcher = GlobPattern::new(&pattern)
                .map_err(|e| Error::message(format!("invalid glob pattern '{pattern}': {e}")))?
                .compile_matcher();
            let mut matched: Vec<String> = listed
                .files
                .into_iter()
                .filter(|f| {
                    let relative = PathBuf::from(f)
                        .strip_prefix(&root)
                        .map(PathBuf::from)
                        .unwrap_or_else(|_| PathBuf::from(f));
                    if !matcher.is_match(&relative) {
                        return false;
                    }
                    if let Some(ref policy) = self.path_policy
                        && policy.check(&PathBuf::from(f)).is_some()
                    {
                        return false;
                    }
                    true
                })
                .collect();
            let result_truncated = matched.len() > DEFAULT_GLOB_LIMIT;
            if result_truncated {
                matched.truncate(DEFAULT_GLOB_LIMIT);
            }
            let truncated = listed.truncated || result_truncated;
            #[cfg(feature = "metrics")]
            counter!(
                tools_metrics::EXECUTIONS_TOTAL,
                labels::TOOL => "Glob".to_string(),
                labels::SUCCESS => "true".to_string()
            )
            .increment(1);
            let mut payload = json!({
                "paths": matched,
                "truncated": truncated,
                "root": root.to_string_lossy(),
            });
            if listed.truncated {
                let limit = listed.limit.unwrap_or(0);
                payload["scan_truncated"] = json!(true);
                payload["scan_limit"] = json!(limit);
                payload["continuation_hint"] = json!(format!(
                    "Sandbox file scan was capped at {limit} files. Narrow the search root or use a more specific glob pattern."
                ));
            }
            return Ok(payload);
        }

        let workspace_root = self.workspace_root.clone();
        let path_policy = self.path_policy.clone();
        let respect_gitignore = self.respect_gitignore;
        let sandbox_router = self.sandbox_router.clone();
        let result = tokio::task::spawn_blocking(move || {
            let tool = Self {
                workspace_root,
                path_policy,
                respect_gitignore,
                sandbox_router,
            };
            tool.glob_impl(&pattern, path.as_deref())
        })
        .await
        .map_err(|e| Error::message(format!("glob task failed: {e}")))?;

        match result {
            Ok(value) => Ok(value),
            Err(e) => {
                #[cfg(feature = "metrics")]
                counter!(
                    tools_metrics::EXECUTION_ERRORS_TOTAL,
                    labels::TOOL => "Glob".to_string()
                )
                .increment(1);
                Err(e.into())
            },
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            exec::ExecResult,
            sandbox::{
                SandboxConfig, SandboxMode, SandboxRouter,
                file_system::{MAX_SANDBOX_LIST_FILES, test_helpers::MockSandbox},
            },
        },
        std::sync::Arc,
    };

    #[tokio::test]
    async fn glob_finds_matching_files() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "x")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.rs"), "x")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("c.txt"), "x")
            .await
            .unwrap();

        let tool = GlobTool::new();
        let value = tool
            .execute(json!({
                "pattern": "*.rs",
                "path": dir.path().to_str().unwrap(),
            }))
            .await
            .unwrap();

        let paths = value["paths"].as_array().unwrap();
        assert_eq!(paths.len(), 2);
        for p in paths {
            assert!(p.as_str().unwrap().ends_with(".rs"));
        }
    }

    #[tokio::test]
    async fn glob_recursive_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("src");
        tokio::fs::create_dir(&sub).await.unwrap();
        tokio::fs::write(sub.join("lib.rs"), "x").await.unwrap();
        tokio::fs::write(dir.path().join("top.rs"), "x")
            .await
            .unwrap();

        let tool = GlobTool::new();
        let value = tool
            .execute(json!({
                "pattern": "**/*.rs",
                "path": dir.path().to_str().unwrap(),
            }))
            .await
            .unwrap();

        let paths: Vec<String> = value["paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().any(|p| p.ends_with("lib.rs")));
        assert!(paths.iter().any(|p| p.ends_with("top.rs")));
    }

    #[tokio::test]
    async fn glob_sorted_by_mtime_desc() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.txt");
        let new = dir.path().join("new.txt");
        tokio::fs::write(&old, "x").await.unwrap();
        // Ensure distinct mtimes.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        tokio::fs::write(&new, "x").await.unwrap();

        let tool = GlobTool::new();
        let value = tool
            .execute(json!({
                "pattern": "*.txt",
                "path": dir.path().to_str().unwrap(),
            }))
            .await
            .unwrap();

        let paths = value["paths"].as_array().unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].as_str().unwrap().ends_with("new.txt"));
        assert!(paths[1].as_str().unwrap().ends_with("old.txt"));
    }

    #[tokio::test]
    async fn glob_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        // Must be a git repo root for gitignore to apply.
        tokio::fs::create_dir(dir.path().join(".git"))
            .await
            .unwrap();
        tokio::fs::write(dir.path().join(".gitignore"), "ignored.rs\n")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("kept.rs"), "x")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("ignored.rs"), "x")
            .await
            .unwrap();

        let tool = GlobTool::new();
        let value = tool
            .execute(json!({
                "pattern": "*.rs",
                "path": dir.path().to_str().unwrap(),
            }))
            .await
            .unwrap();

        let paths: Vec<String> = value["paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("kept.rs"));
    }

    #[tokio::test]
    async fn glob_respect_gitignore_false_includes_ignored() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir(dir.path().join(".git"))
            .await
            .unwrap();
        tokio::fs::write(dir.path().join(".gitignore"), "ignored.rs\n")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("kept.rs"), "x")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("ignored.rs"), "x")
            .await
            .unwrap();

        let tool = GlobTool::new().with_respect_gitignore(false);
        let value = tool
            .execute(json!({
                "pattern": "*.rs",
                "path": dir.path().to_str().unwrap(),
            }))
            .await
            .unwrap();

        let paths: Vec<String> = value["paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().any(|p| p.ends_with("kept.rs")));
        assert!(paths.iter().any(|p| p.ends_with("ignored.rs")));
    }

    #[tokio::test]
    async fn glob_invalid_pattern_errors() {
        let tool = GlobTool::new();
        let err = tool
            .execute(json!({ "pattern": "[invalid" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid glob pattern"));
    }

    #[tokio::test]
    async fn glob_missing_pattern_errors() {
        let tool = GlobTool::new();
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("missing 'pattern'"));
    }

    #[tokio::test]
    async fn glob_missing_path_without_workspace_root_errors() {
        let tool = GlobTool::new();
        let err = tool
            .execute(json!({ "pattern": "*.rs" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no workspace root"));
    }

    #[tokio::test]
    async fn glob_rejects_relative_path() {
        let tool = GlobTool::new();
        let err = tool
            .execute(json!({
                "pattern": "*.rs",
                "path": "relative/dir",
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("must be an absolute path"));
    }

    #[tokio::test]
    async fn glob_falls_back_to_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "x")
            .await
            .unwrap();

        let tool = GlobTool::new().with_workspace_root(dir.path().to_path_buf());
        let value = tool.execute(json!({ "pattern": "*.rs" })).await.unwrap();

        let paths = value["paths"].as_array().unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].as_str().unwrap().ends_with("a.rs"));
    }

    #[tokio::test]
    async fn glob_sandbox_scan_truncation_returns_friendly_metadata() {
        let stdout = (0..=MAX_SANDBOX_LIST_FILES)
            .map(|index| format!("/workspace/file-{index}.rs"))
            .collect::<Vec<_>>()
            .join("\n");
        let mock = MockSandbox::new(vec![ExecResult {
            stdout,
            stderr: String::new(),
            exit_code: 0,
        }]);
        let router = Arc::new(SandboxRouter::with_backend(
            SandboxConfig {
                mode: SandboxMode::All,
                ..Default::default()
            },
            mock,
        ));
        let tool = GlobTool::new().with_sandbox_router(router);

        let value = tool
            .execute(json!({
                "pattern": "*.rs",
                "path": "/workspace",
                "_session_key": "sandboxed",
            }))
            .await
            .unwrap();

        assert_eq!(value["scan_truncated"], true);
        assert_eq!(value["scan_limit"], MAX_SANDBOX_LIST_FILES);
        assert_eq!(value["truncated"], true);
        assert!(
            value["continuation_hint"]
                .as_str()
                .unwrap()
                .contains("Sandbox file scan was capped")
        );
    }
}
