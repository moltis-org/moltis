//! Native filesystem tools: `Read`, `Write`, `Edit`, `MultiEdit`, `Glob`, `Grep`.
//!
//! These are the structured, typed alternative to shell-based file I/O via
//! `exec`. They match Claude Code's tool schemas exactly so LLMs trained on
//! those tools encounter the same shape of parameters and responses.
//!
//! See GH moltis-org/moltis#657 for context.
//!
//! Phase 1 (this module) covers host-path execution only. Sandbox routing
//! arrives in phase 2, UX polish (adaptive paging, edit recovery, re-read
//! detection) in phase 3, and operator-facing `[tools.fs]` config in phase 4.

pub mod edit;
pub mod glob;
pub mod grep;
pub mod multi_edit;
pub mod read;
pub mod sandbox_bridge;
pub mod shared;
pub mod write;

pub use {
    edit::EditTool,
    glob::GlobTool,
    grep::GrepTool,
    multi_edit::MultiEditTool,
    read::ReadTool,
    shared::{BinaryPolicy, FsPathPolicy, FsState, new_fs_state},
    write::WriteTool,
};

use {
    crate::{
        approval::ApprovalManager, checkpoints::CheckpointManager, exec::ApprovalBroadcaster,
        sandbox::SandboxRouter,
    },
    moltis_agents::tool_registry::ToolRegistry,
    std::{path::PathBuf, sync::Arc},
};

/// Aggregated configuration for fs tool registration.
///
/// Phase 1 shipped with three bare positional parameters; phase 4 keeps
/// adding knobs so the registration signature is migrated to a single
/// context struct.
#[derive(Clone)]
pub struct FsToolsContext {
    /// Default search root for `Glob`/`Grep` when the LLM omits `path`.
    /// Must be absolute. When `None`, calls without explicit `path` error.
    pub workspace_root: Option<PathBuf>,
    /// Shared per-session state for read tracking, loop detection, and
    /// must-read-before-write enforcement. `None` disables all trackers.
    pub fs_state: Option<FsState>,
    /// Allow/deny path policy. Empty policy (`None`) permits everything.
    pub path_policy: Option<FsPathPolicy>,
    /// Binary-file handling strategy for `Read`. Default is `Reject`.
    pub binary_policy: BinaryPolicy,
    /// Whether `Glob`/`Grep` honor `.gitignore` while walking. Default
    /// `true`.
    pub respect_gitignore: bool,
    /// When set, `Write`/`Edit`/`MultiEdit` call `checkpoint_path` on
    /// this manager before mutating. `None` disables checkpoints.
    pub checkpoint_manager: Option<Arc<CheckpointManager>>,
    /// Shared [`SandboxRouter`]. When set, fs tools dispatch through
    /// the [`sandbox_bridge`] for sessions the router marks as
    /// sandboxed; unsandboxed sessions still run on the host.
    pub sandbox_router: Option<Arc<SandboxRouter>>,
    /// Optional approval gate for mutating fs tools. When set,
    /// Write/Edit/MultiEdit pause for explicit approval before
    /// persisting changes.
    pub approval_manager: Option<Arc<ApprovalManager>>,
    /// Optional broadcaster paired with [`approval_manager`] so pending
    /// fs mutation approvals show up in the gateway UI.
    pub broadcaster: Option<Arc<dyn ApprovalBroadcaster>>,
    /// Override for the maximum bytes a single `Read` can return
    /// before a `too_large` typed error. Wired from
    /// `[tools.fs].max_read_bytes`. `None` → `DEFAULT_MAX_READ_BYTES`.
    pub max_read_bytes: Option<u64>,
    /// Model context window in tokens. When set, enables `Read`'s
    /// adaptive byte cap so per-call output scales with the working
    /// set instead of using a fixed 256 KB ceiling.
    pub context_window_tokens: Option<u64>,
}

impl Default for FsToolsContext {
    fn default() -> Self {
        Self {
            workspace_root: None,
            fs_state: None,
            path_policy: None,
            binary_policy: BinaryPolicy::default(),
            // Follow the upstream default: WalkBuilder respects .gitignore
            // unless explicitly disabled.
            respect_gitignore: true,
            checkpoint_manager: None,
            sandbox_router: None,
            approval_manager: None,
            broadcaster: None,
            max_read_bytes: None,
            context_window_tokens: None,
        }
    }
}

impl FsToolsContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Register every native filesystem tool on a [`ToolRegistry`].
///
/// See [`FsToolsContext`] for the individual knobs. The `tools.policy`
/// allow/deny layer (per-tool names, not paths) still gates access
/// per-agent; registration is independent of authorization.
pub fn register_fs_tools(registry: &mut ToolRegistry, context: FsToolsContext) {
    let FsToolsContext {
        workspace_root,
        fs_state,
        path_policy,
        binary_policy,
        respect_gitignore,
        checkpoint_manager,
        sandbox_router,
        approval_manager,
        broadcaster,
        max_read_bytes,
        context_window_tokens,
    } = context;

    let mut read = ReadTool::new().with_binary_policy(binary_policy);
    if let Some(max) = max_read_bytes {
        read = read.with_max_read_bytes(max);
    }
    if let Some(tokens) = context_window_tokens {
        read = read.with_context_window_tokens(tokens);
    }
    if let Some(ref s) = fs_state {
        read = read.with_fs_state(s.clone());
    }
    if let Some(ref p) = path_policy {
        read = read.with_path_policy(p.clone());
    }
    if let Some(ref r) = sandbox_router {
        read = read.with_sandbox_router(r.clone());
    }
    registry.register(Box::new(read));

    let mut write = WriteTool::new();
    if let Some(ref s) = fs_state {
        write = write.with_fs_state(s.clone());
    }
    if let Some(ref p) = path_policy {
        write = write.with_path_policy(p.clone());
    }
    if let Some(ref m) = checkpoint_manager {
        write = write.with_checkpoint_manager(m.clone());
    }
    if let Some(ref r) = sandbox_router {
        write = write.with_sandbox_router(r.clone());
    }
    if let (Some(manager), Some(broadcaster)) = (&approval_manager, &broadcaster) {
        write = write.with_approval(manager.clone(), broadcaster.clone());
    }
    registry.register(Box::new(write));

    let mut edit = EditTool::new();
    if let Some(ref s) = fs_state {
        edit = edit.with_fs_state(s.clone());
    }
    if let Some(ref p) = path_policy {
        edit = edit.with_path_policy(p.clone());
    }
    if let Some(ref m) = checkpoint_manager {
        edit = edit.with_checkpoint_manager(m.clone());
    }
    if let Some(ref r) = sandbox_router {
        edit = edit.with_sandbox_router(r.clone());
    }
    if let (Some(manager), Some(broadcaster)) = (&approval_manager, &broadcaster) {
        edit = edit.with_approval(manager.clone(), broadcaster.clone());
    }
    registry.register(Box::new(edit));

    let mut multi_edit = MultiEditTool::new();
    if let Some(ref s) = fs_state {
        multi_edit = multi_edit.with_fs_state(s.clone());
    }
    if let Some(ref p) = path_policy {
        multi_edit = multi_edit.with_path_policy(p.clone());
    }
    if let Some(ref m) = checkpoint_manager {
        multi_edit = multi_edit.with_checkpoint_manager(m.clone());
    }
    if let Some(ref r) = sandbox_router {
        multi_edit = multi_edit.with_sandbox_router(r.clone());
    }
    if let (Some(manager), Some(broadcaster)) = (&approval_manager, &broadcaster) {
        multi_edit = multi_edit.with_approval(manager.clone(), broadcaster.clone());
    }
    registry.register(Box::new(multi_edit));

    let mut glob = GlobTool::new().with_respect_gitignore(respect_gitignore);
    if let Some(ref root) = workspace_root {
        glob = glob.with_workspace_root(root.clone());
    }
    if let Some(ref p) = path_policy {
        glob = glob.with_path_policy(p.clone());
    }
    if let Some(ref r) = sandbox_router {
        glob = glob.with_sandbox_router(r.clone());
    }
    registry.register(Box::new(glob));

    let mut grep = GrepTool::new().with_respect_gitignore(respect_gitignore);
    if let Some(root) = workspace_root {
        grep = grep.with_workspace_root(root);
    }
    if let Some(p) = path_policy {
        grep = grep.with_path_policy(p);
    }
    if let Some(r) = sandbox_router {
        grep = grep.with_sandbox_router(r);
    }
    registry.register(Box::new(grep));
}

/// Canonical list of tool names registered by [`register_fs_tools`].
pub const FS_TOOL_NAMES: &[&str] = &["Read", "Write", "Edit", "MultiEdit", "Glob", "Grep"];

#[cfg(test)]
mod contract_tests;
