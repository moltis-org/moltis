//! Shared helpers for the native filesystem tools (Read/Write/Edit/MultiEdit/Glob/Grep).
//!
//! Kept deliberately small: path canonicalization, binary detection, and the
//! `cat -n` style line-number formatter that callers use to render file
//! contents back to the LLM.

use {
    globset::{Glob, GlobSet, GlobSetBuilder},
    serde_json::{Value, json},
    std::{
        collections::HashMap,
        future::Future,
        io,
        path::{Path, PathBuf},
        sync::{Arc, Mutex, OnceLock},
    },
    tokio::fs,
};

use {
    crate::{
        Result,
        approval::{ApprovalDecision, ApprovalManager},
        error::Error,
        exec::ApprovalBroadcaster,
    },
    tracing::{info, warn},
};

/// Number of consecutive identical reads before a `loop_warning` is added
/// to Read's response payload. Ported from hermes's `_read_tracker`.
pub const READ_LOOP_THRESHOLD: usize = 3;

type MutationQueueMap = Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

fn mutation_queue_map() -> &'static MutationQueueMap {
    static QUEUES: OnceLock<MutationQueueMap> = OnceLock::new();
    QUEUES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Serialize filesystem mutations that target the same logical path.
///
/// Host mutations should use a canonical absolute path as the key. Sandbox
/// mutations should include the session key in the namespace so two isolated
/// sandboxes writing the same in-container path do not block each other.
pub async fn with_fs_mutation_lock<T, F>(key: String, op: F) -> T
where
    F: Future<Output = T>,
{
    let gate = {
        let mut queues = mutation_queue_map()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queues
            .entry(key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };

    let guard = gate.lock().await;
    let result = op.await;
    drop(guard);

    let mut queues = mutation_queue_map()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let should_remove = queues
        .get(&key)
        .is_some_and(|current| Arc::ptr_eq(current, &gate) && Arc::strong_count(current) == 2);
    if should_remove {
        queues.remove(&key);
    }

    result
}

/// Key for host-side filesystem mutation serialization.
#[must_use]
pub fn host_mutation_queue_key(path: &Path) -> String {
    format!("host:{}", path.display())
}

/// Key for sandbox-side filesystem mutation serialization.
#[must_use]
pub fn sandbox_mutation_queue_key(session_key: &str, file_path: &str) -> String {
    format!("sandbox:{session_key}:{file_path}")
}

/// Strategy for handling binary files encountered by `Read`.
///
/// Mirrors `config::schema::FsBinaryPolicy`. The tools crate can't
/// depend on the config crate directly (that would be a cycle), so this
/// enum is an internal copy the gateway maps into at registration time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BinaryPolicy {
    /// Return a typed `{kind: "binary", bytes: N}` marker without
    /// content. Default.
    #[default]
    Reject,
    /// Return `{kind: "binary", bytes: N, base64: "..."}` so the LLM
    /// can access the raw bytes. Still gated by `max_read_bytes`.
    Base64,
}

/// Typed error kinds that fs tools surface to the LLM as structured `Ok`
/// payloads rather than plain `Err` strings.
///
/// These are the expected failure modes the issue (moltis-org/moltis#657)
/// asks for: *"structured response for binary / nonexistent / permission-
/// denied"*. Returning them as `Ok(value_with_kind_field)` means the chat
/// loop's `err.to_string()` conversion doesn't strip the structure — the
/// LLM sees the typed JSON directly and can branch on `kind`.
///
/// Internal / unexpected failures (I/O mid-read, spawn_blocking crash,
/// etc.) still propagate as `Err` — this enum is strictly for anticipated
/// user-visible error conditions.
pub enum FsErrorKind {
    NotFound,
    PermissionDenied,
    TooLarge,
    NotRegularFile,
    /// The session is configured with `must_read_before_write` and tried
    /// to mutate a file it had not previously read.
    MustReadBeforeWrite,
}

impl FsErrorKind {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::PermissionDenied => "permission_denied",
            Self::TooLarge => "too_large",
            Self::NotRegularFile => "not_regular_file",
            Self::MustReadBeforeWrite => "must_read_before_write",
        }
    }
}

/// Build a typed error `Value` payload the LLM can branch on.
///
/// Shape: `{ "kind": "<kind>", "file_path": "...", "error": "...", "detail": "..." }`.
/// `detail` carries the raw OS error when we have one, so operators
/// looking at the LLM transcript can still diagnose the underlying cause.
#[must_use]
pub fn fs_error_payload(
    kind: FsErrorKind,
    file_path: &str,
    error: &str,
    detail: Option<&str>,
) -> Value {
    json!({
        "kind": kind.as_str(),
        "file_path": file_path,
        "error": error,
        "detail": detail.unwrap_or(""),
    })
}

/// If `err` corresponds to an anticipated user-visible failure mode,
/// return the typed payload. Otherwise return `None` so the caller can
/// propagate as a real `Err` (unexpected I/O failure).
#[must_use]
pub fn io_error_to_typed_payload(err: &io::Error, file_path: &str) -> Option<Value> {
    let (kind, message) = match err.kind() {
        io::ErrorKind::NotFound => (FsErrorKind::NotFound, "file does not exist"),
        io::ErrorKind::PermissionDenied => (
            FsErrorKind::PermissionDenied,
            "insufficient permissions to access file",
        ),
        _ => return None,
    };
    Some(fs_error_payload(
        kind,
        file_path,
        message,
        Some(&err.to_string()),
    ))
}

/// Default maximum file size the fs tools will read in a single call.
///
/// Overridden per-tool via `ReadTool::with_max_read_bytes`, wired from
/// `[tools.fs].max_read_bytes`.
pub const DEFAULT_MAX_READ_BYTES: u64 = 10 * 1024 * 1024;

/// Maximum number of lines returned by a single `Read` call when no explicit
/// limit is provided.
pub const DEFAULT_READ_LINE_LIMIT: usize = 2000;

/// Maximum characters per line before the line is truncated in `Read` output.
///
/// Mirrors Claude Code's behavior so LLMs trained on it encounter the same
/// shape of response.
pub const MAX_LINE_CHARS: usize = 2000;

/// Maximum total bytes of rendered output returned by a single `Read`
/// when no adaptive context window is configured.
///
/// Caps payload size independently of line count: 2000 lines × 2000
/// chars would otherwise permit ~4 MB per call, which can blow tool-
/// response limits on smaller-context models. When the rendered output
/// exceeds this cap, [`format_numbered_lines`] truncates at the last
/// full line that fits and sets `truncated=true`.
pub const MAX_READ_OUTPUT_BYTES: usize = 256 * 1024;

/// Minimum floor for the adaptive read cap. Even a tiny context window
/// gets at least this many bytes per Read so small reads remain useful.
/// Ported from openclaw's `DEFAULT_READ_PAGE_MAX_BYTES`.
pub const MIN_ADAPTIVE_READ_BYTES: usize = 50 * 1024;

/// Hard ceiling on the adaptive read cap. A huge context window (1M+
/// tokens) would otherwise request hundreds of KB per call and blow
/// tool-response payload limits. Ported from openclaw's
/// `MAX_ADAPTIVE_READ_MAX_BYTES`.
pub const MAX_ADAPTIVE_READ_BYTES: usize = 512 * 1024;

/// Rough tokens-to-characters conversion for adaptive cap computation.
/// English text averages ~4 characters per token for Claude/GPT
/// tokenizers; code is closer to 3 so we err slightly under.
pub const CHARS_PER_TOKEN_ESTIMATE: u64 = 4;

/// Share of the model's context window a single `Read` call is
/// allowed to consume. 20% leaves the LLM plenty of room to think.
pub const ADAPTIVE_READ_CONTEXT_SHARE_PERCENT: u64 = 20;

/// Compute the adaptive per-call read cap in bytes, given the model's
/// context window in tokens. Clamped to
/// `[MIN_ADAPTIVE_READ_BYTES, MAX_ADAPTIVE_READ_BYTES]`.
///
/// ```text
/// cap = clamp(
///     ctx_tokens * CHARS_PER_TOKEN_ESTIMATE * SHARE_PERCENT / 100,
///     MIN_ADAPTIVE_READ_BYTES,
///     MAX_ADAPTIVE_READ_BYTES,
/// )
/// ```
#[must_use]
pub fn compute_adaptive_read_cap(context_window_tokens: u64) -> usize {
    let raw = context_window_tokens
        .saturating_mul(CHARS_PER_TOKEN_ESTIMATE)
        .saturating_mul(ADAPTIVE_READ_CONTEXT_SHARE_PERCENT)
        / 100;
    let clamped = raw
        .max(MIN_ADAPTIVE_READ_BYTES as u64)
        .min(MAX_ADAPTIVE_READ_BYTES as u64);
    clamped as usize
}

/// Number of bytes to inspect when heuristically detecting binary content.
const BINARY_SNIFF_BYTES: usize = 8192;

/// Reject relative paths at the tool boundary.
///
/// Claude Code's fs tools require absolute paths. We enforce the same to
/// avoid silent resolution against the gateway's process cwd, which is
/// almost never what the LLM means. See `moltis-ung` for context.
pub fn require_absolute(path: &str, field: &str) -> Result<()> {
    if path.is_empty() {
        return Err(Error::message(format!("{field} must not be empty")));
    }
    if !Path::new(path).is_absolute() {
        return Err(Error::message(format!(
            "{field} must be an absolute path (got '{path}'); relative paths are not supported \
             to avoid silent resolution against the gateway process working directory"
        )));
    }
    Ok(())
}

/// Extract the optional `_session_key` parameter threaded into every tool
/// call by the chat loop. Returns `"default"` when absent.
#[must_use]
pub fn session_key_from(params: &Value) -> &str {
    params
        .get("_session_key")
        .and_then(Value::as_str)
        .unwrap_or("default")
}

/// Per-session fs tool state.
///
/// Tracks which files the session has read (for must-read-before-write
/// enforcement) and how many consecutive identical reads it has made
/// (for loop detection after context compression).
#[derive(Debug, Default)]
pub struct SessionFsState {
    /// Canonical paths the session has successfully read, with the
    /// mtime at read time. Edit/Write check this to detect files
    /// modified by linters or hooks between Read and mutation.
    pub read_files: HashMap<PathBuf, Option<std::time::SystemTime>>,
    /// Most recent read signature: (path, offset, limit).
    pub last_read_key: Option<(PathBuf, usize, usize)>,
    /// Number of consecutive reads matching `last_read_key` with no
    /// intervening mutation.
    pub consecutive_reads: usize,
}

/// Shared fs state across all fs tool instances in one gateway.
///
/// `None` disables every phase-3 tracker: must-read-before-write does
/// nothing, loop detection does nothing, read history is not recorded.
/// The gateway currently passes `None` and phase 4 config will flip it
/// on via `[tools.fs].track_reads` / `must_read_before_write`.
pub type FsState = Arc<Mutex<FsStateInner>>;

#[derive(Debug, Default)]
pub struct FsStateInner {
    sessions: HashMap<String, SessionFsState>,
    /// When true, Write/Edit/MultiEdit refuse to mutate a file the
    /// session has not read.
    pub must_read_before_write: bool,
}

/// Construct a fresh [`FsState`] handle.
#[must_use]
pub fn new_fs_state(must_read_before_write: bool) -> FsState {
    Arc::new(Mutex::new(FsStateInner {
        sessions: HashMap::new(),
        must_read_before_write,
    }))
}

/// Path allow/deny policy shared across fs tools.
///
/// Built from `[tools.fs].allow_paths` / `[tools.fs].deny_paths` globs at
/// gateway startup and checked at the tool boundary after path
/// canonicalization. Deny always wins over allow. An empty allow list
/// means "no allowlist — all paths are allowed unless explicitly denied."
///
/// Cheap to clone (compiled [`GlobSet`]s are `Arc`-backed internally).
#[derive(Debug, Clone, Default)]
pub struct FsPathPolicy {
    allow: Option<GlobSet>,
    deny: Option<GlobSet>,
}

impl FsPathPolicy {
    /// Build a new [`FsPathPolicy`] from allow/deny glob lists.
    ///
    /// Returns an error if any glob fails to compile.
    pub fn new(
        allow_patterns: &[String],
        deny_patterns: &[String],
    ) -> std::result::Result<Self, String> {
        let allow = if allow_patterns.is_empty() {
            None
        } else {
            Some(build_globset(allow_patterns, "allow_paths")?)
        };
        let deny = if deny_patterns.is_empty() {
            None
        } else {
            Some(build_globset(deny_patterns, "deny_paths")?)
        };
        Ok(Self { allow, deny })
    }

    /// Check whether `path` is permitted under the full allow+deny rules.
    ///
    /// Returns `None` on permit and `Some(reason)` on reject. Used for
    /// individual file accesses (Read, Write, Edit, MultiEdit, single
    /// Glob/Grep entries).
    pub fn check(&self, path: &Path) -> Option<&'static str> {
        if let Some(ref deny) = self.deny
            && deny.is_match(path)
        {
            return Some("denied by tools.fs.deny_paths");
        }
        if let Some(ref allow) = self.allow
            && !allow.is_match(path)
        {
            return Some("not permitted by tools.fs.allow_paths");
        }
        None
    }

    /// Check whether `path` is blocked by the deny list only.
    ///
    /// Used for directory walk roots (Glob/Grep) where the root itself
    /// typically won't match a file-granular allow list but its children
    /// might — so the allow list filters results rather than gating the
    /// whole call. Deny-list matches still reject the entire call.
    pub fn check_deny_only(&self, path: &Path) -> Option<&'static str> {
        if let Some(ref deny) = self.deny
            && deny.is_match(path)
        {
            return Some("denied by tools.fs.deny_paths");
        }
        None
    }

    /// Whether this policy is the permissive default (no rules).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.allow.is_none() && self.deny.is_none()
    }
}

fn build_globset(patterns: &[String], field: &str) -> std::result::Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob =
            Glob::new(pattern).map_err(|e| format!("invalid glob in {field}: '{pattern}': {e}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| format!("failed to compile {field}: {e}"))
}

/// Check path policy and return a typed error payload if the path is
/// denied. Canonicalize before calling so symlinks can't bypass the
/// allowlist.
#[must_use]
pub fn enforce_path_policy(policy: &FsPathPolicy, path: &Path) -> Option<Value> {
    let reason = policy.check(path)?;
    Some(path_denied_payload(path, reason))
}

/// Variant of [`enforce_path_policy`] that only considers the deny list.
///
/// Directory walk roots (Glob/Grep) use this because the root typically
/// won't match a file-granular allow list, but its children can. Per-
/// file filtering still applies the full policy.
#[must_use]
pub fn enforce_path_policy_deny_only(policy: &FsPathPolicy, path: &Path) -> Option<Value> {
    let reason = policy.check_deny_only(path)?;
    Some(path_denied_payload(path, reason))
}

fn path_denied_payload(path: &Path, reason: &str) -> Value {
    json!({
        "kind": "path_denied",
        "file_path": path.to_string_lossy(),
        "error": "path is not permitted by tools.fs policy",
        "detail": reason,
    })
}

/// Require explicit operator approval before a filesystem mutation.
///
/// The gateway threads the existing approval manager and broadcaster
/// into fs tools only when `[tools.fs].require_approval = true`, so
/// `None` means "approval disabled" rather than "misconfigured".
pub async fn require_fs_mutation_approval(
    approval_manager: Option<&Arc<ApprovalManager>>,
    broadcaster: Option<&Arc<dyn ApprovalBroadcaster>>,
    request: &str,
) -> Result<()> {
    let Some(manager) = approval_manager else {
        return Ok(());
    };

    info!(request, "fs mutation needs approval, waiting...");
    let (request_id, rx) = manager.create_request(request, None).await;

    if let Some(broadcaster) = broadcaster
        && let Err(error) = broadcaster
            .broadcast_request(&request_id, request, None)
            .await
    {
        warn!(%error, request, "failed to broadcast fs approval request");
    }

    match manager.wait_for_decision(rx).await {
        ApprovalDecision::Approved => Ok(()),
        ApprovalDecision::Denied => Err(Error::message(format!(
            "filesystem mutation denied by user: {request}"
        ))),
        ApprovalDecision::Timeout => Err(Error::message(format!(
            "filesystem approval timed out for: {request}"
        ))),
    }
}

/// Check the must-read-before-write invariant against the shared state.
///
/// When `fs_state` is `Some` and [`FsStateInner::must_read_before_write`]
/// is on, returns a typed `must_read_before_write` payload if the session
/// has not read `file_path`. Otherwise returns `None` and mutation
/// proceeds normally.
///
/// Designed to be called *after* path canonicalization and *after* the
/// target file is known to exist. For Write to a new file, callers should
/// skip this check since there's nothing to have read yet.
#[must_use]
pub fn enforce_must_read_before_write(
    fs_state: Option<&FsState>,
    session_key: &str,
    file_path: &str,
) -> Option<Value> {
    let state = fs_state?;
    let guard = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !guard.must_read_before_write {
        return None;
    }
    let path = Path::new(file_path);
    if guard.has_been_read(session_key, path) {
        return None;
    }
    Some(fs_error_payload(
        FsErrorKind::MustReadBeforeWrite,
        file_path,
        "cannot mutate a file this session has not read — call Read first",
        Some("must_read_before_write policy is enabled"),
    ))
}

/// Check whether a file has been modified since the session last read it.
///
/// Returns a typed error payload when the current mtime differs from
/// the recorded mtime. Returns `None` (proceed) when mtimes match,
/// when tracking is off, or when the file was never read (the latter
/// is handled by `enforce_must_read_before_write` separately).
///
/// Ported from Claude Code's errorCode 7: *"File has been modified since
/// read, either by the user or by a linter. Read it again."*
#[must_use]
pub fn check_file_modified_since_read(
    fs_state: Option<&FsState>,
    session_key: &str,
    canonical_path: &Path,
    file_path_display: &str,
) -> Option<Value> {
    let state = fs_state?;
    let guard = state.lock().unwrap_or_else(|p| p.into_inner());
    let recorded_mtime = guard.read_mtime(session_key, canonical_path)?;
    let current_mtime = std::fs::metadata(canonical_path)
        .ok()
        .and_then(|m| m.modified().ok());
    let Some(current) = current_mtime else {
        return None; // can't stat — let the write proceed and fail naturally
    };
    if current != recorded_mtime {
        return Some(json!({
            "kind": "file_modified_since_read",
            "file_path": file_path_display,
            "error": "File has been modified since read, either by the user or by a linter. Read it again before attempting to edit.",
            "detail": "",
        }));
    }
    None
}

/// Note a successful mutation so the per-session loop counter is reset
/// for subsequent reads of the same path.
pub fn note_fs_mutation(fs_state: Option<&FsState>, session_key: &str, file_path: &str) {
    if let Some(state) = fs_state {
        let mut guard = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.note_mutation(session_key, Path::new(file_path));
    }
}

impl FsStateInner {
    /// Record that `session_key` successfully read `(path, offset, limit)`.
    ///
    /// Returns the new consecutive-read count for this `(path, offset,
    /// limit)` signature. A count of >= [`READ_LOOP_THRESHOLD`] signals a
    /// re-read loop and Read should append a warning to its response.
    pub fn record_read(
        &mut self,
        session_key: &str,
        path: PathBuf,
        offset: usize,
        limit: usize,
        mtime: Option<std::time::SystemTime>,
    ) -> usize {
        let entry = self.sessions.entry(session_key.to_string()).or_default();
        let key = (path.clone(), offset, limit);
        if entry.last_read_key.as_ref() == Some(&key) {
            entry.consecutive_reads = entry.consecutive_reads.saturating_add(1);
        } else {
            entry.last_read_key = Some(key);
            entry.consecutive_reads = 1;
        }
        entry.read_files.insert(path, mtime);
        entry.consecutive_reads
    }

    /// Whether `session_key` has successfully read `path`.
    #[must_use]
    pub fn has_been_read(&self, session_key: &str, path: &Path) -> bool {
        self.sessions
            .get(session_key)
            .is_some_and(|state| state.read_files.contains_key(path))
    }

    /// Return the mtime that was recorded when `path` was last read
    /// by `session_key`. Returns `None` if the file was never read or
    /// if the mtime was unavailable at read time.
    #[must_use]
    pub fn read_mtime(&self, session_key: &str, path: &Path) -> Option<std::time::SystemTime> {
        self.sessions
            .get(session_key)
            .and_then(|state| state.read_files.get(path).copied().flatten())
    }

    /// Current consecutive-read count for `session_key`. Returns 0
    /// when the session has no tracked reads.
    #[must_use]
    pub fn consecutive_reads(&self, session_key: &str) -> usize {
        self.sessions
            .get(session_key)
            .map(|s| s.consecutive_reads)
            .unwrap_or(0)
    }

    /// Remove all tracking state for a completed session.
    pub fn remove_session(&mut self, session_key: &str) {
        self.sessions.remove(session_key);
    }

    /// Note a mutation to `path`: reset the loop counter if the most
    /// recent read targeted the same file, and leave `read_files`
    /// untouched so the LLM doesn't need to re-read after its own edit.
    pub fn note_mutation(&mut self, session_key: &str, path: &Path) {
        if let Some(entry) = self.sessions.get_mut(session_key)
            && let Some((last_path, ..)) = entry.last_read_key.as_ref()
            && last_path.as_path() == path
        {
            entry.consecutive_reads = 0;
        }
    }
}

/// Canonicalize a user-supplied path. Requires the path to exist and be absolute.
///
/// Returns an absolute, symlink-resolved `PathBuf`. All fs tools canonicalize
/// at the boundary so symlink escapes can't bypass future path allowlists.
pub async fn canonicalize_existing(path: &str) -> Result<PathBuf> {
    require_absolute(path, "file_path")?;
    fs::canonicalize(path)
        .await
        .map_err(|e| Error::message(format!("cannot resolve path '{path}': {e}")))
}

/// Canonicalize a path that may not exist yet (e.g. for `Write` to a new file).
///
/// Requires the path to be absolute. Canonicalizes the parent directory and
/// appends the final component. Returns an error if the parent does not
/// exist or is not a directory.
pub async fn canonicalize_for_create(path: &str) -> Result<PathBuf> {
    require_absolute(path, "file_path")?;
    let pb = PathBuf::from(path);
    let parent = pb
        .parent()
        .ok_or_else(|| Error::message(format!("path '{path}' has no parent directory")))?;
    let file_name = pb
        .file_name()
        .ok_or_else(|| Error::message(format!("path '{path}' has no file name")))?;

    let parent_canonical = fs::canonicalize(parent)
        .await
        .map_err(|e| Error::message(format!("cannot resolve parent of '{path}': {e}")))?;

    Ok(parent_canonical.join(file_name))
}

/// Reject paths that resolve through a symlink.
///
/// Called after `canonicalize_existing` when policy requires the final path to
/// be a real file. The canonicalized path itself is not a symlink (canonicalize
/// resolves them), so this checks the *original* path's metadata separately.
pub async fn reject_if_symlink(original: &str) -> Result<()> {
    let meta = fs::symlink_metadata(original)
        .await
        .map_err(|e| Error::message(format!("cannot stat '{original}': {e}")))?;
    if meta.file_type().is_symlink() {
        return Err(Error::message(format!(
            "'{original}' is a symbolic link; refusing to follow"
        )));
    }
    Ok(())
}

/// Ensure the path is a regular file (not a directory, fifo, socket, etc).
pub async fn ensure_regular_file(path: &Path) -> Result<u64> {
    let meta = fs::metadata(path)
        .await
        .map_err(|e| Error::message(format!("cannot stat '{}': {e}", path.display())))?;
    if !meta.is_file() {
        return Err(Error::message(format!(
            "'{}' is not a regular file",
            path.display()
        )));
    }
    Ok(meta.len())
}

/// Known binary file extensions (ported from Claude Code's `KHK` set).
///
/// Checked by [`is_binary_extension`] before the content-based
/// null-byte heuristic so files like `.woff2` or `.sqlite` are
/// rejected even when they lack null bytes in the first 8 KB.
const BINARY_EXTENSIONS: &[&str] = &[
    // Images
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "tiff", "tif", // Video
    "mp4", "mov", "avi", "mkv", "webm", "wmv", "flv", "m4v", "mpeg", "mpg", // Audio
    "mp3", "wav", "ogg", "flac", "aac", "m4a", "wma", "aiff", "opus", // Archives
    "zip", "tar", "gz", "bz2", "7z", "rar", "xz", "z", "tgz", "iso",
    // Executables / native
    "exe", "dll", "so", "dylib", "bin", "o", "a", "obj", "lib", "app", "msi", "deb", "rpm",
    // Documents (binary formats)
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", // Fonts
    "ttf", "otf", "woff", "woff2", "eot", // Compiled / bytecode
    "pyc", "pyo", "class", "jar", "war", "ear", // Runtime artifacts
    "node", "wasm", "rlib", // Databases
    "sqlite", "sqlite3", "db", "mdb", "idx", // Design tools
    "psd", "ai", "eps", "sketch", "fig", "xd", // 3D / legacy
    "blend", "3ds", "max", "swf", "fla", // Misc binary
    "lockb", "dat", "data",
];

/// Check file extension against the known binary set.
#[must_use]
pub fn is_binary_extension(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    BINARY_EXTENSIONS.iter().any(|&b| b == ext)
}

/// Heuristic binary detection: extension-based check first (fast, covers
/// known binary formats), then null-byte probe in the first
/// `BINARY_SNIFF_BYTES` (catches everything else).
#[must_use]
pub fn looks_binary(bytes: &[u8]) -> bool {
    let limit = bytes.len().min(BINARY_SNIFF_BYTES);
    bytes[..limit].contains(&0)
}

/// Render file contents with `cat -n` style 1-indexed line numbers,
/// using the default byte cap.
#[must_use]
pub fn format_numbered_lines(content: &str, start_line: usize, max_lines: usize) -> NumberedLines {
    format_numbered_lines_with_cap(content, start_line, max_lines, MAX_READ_OUTPUT_BYTES)
}

/// Render file contents with `cat -n` style 1-indexed line numbers
/// and a configurable byte cap.
///
/// The line-number column is right-padded to the width of the last
/// visible line number so columns align. Lines longer than
/// [`MAX_LINE_CHARS`] are truncated with a `…` marker appended.
/// When `max_output_bytes` is hit, truncation happens at the last
/// full line that fits and `truncated=true` is set.
#[must_use]
pub fn format_numbered_lines_with_cap(
    content: &str,
    start_line: usize,
    max_lines: usize,
    max_output_bytes: usize,
) -> NumberedLines {
    // Strip UTF-8 BOM at byte 0 (matches Claude Code's BOM handling).
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);

    // Split on '\n' so that a trailing newline does not create an extra empty
    // line in the output (common cat -n behavior).
    let mut lines: Vec<&str> = content.split('\n').collect();
    let had_trailing_newline = content.ends_with('\n');
    if had_trailing_newline {
        lines.pop();
    }

    let total_lines = lines.len();
    let start = start_line.max(1);
    if start > total_lines {
        return NumberedLines {
            text: String::new(),
            total_lines,
            start_line: start,
            rendered_lines: 0,
            truncated: false,
        };
    }

    let end_exclusive = total_lines.min(start.saturating_sub(1).saturating_add(max_lines));
    let visible = &lines[start.saturating_sub(1)..end_exclusive];
    let last_line_number = start.saturating_add(visible.len().saturating_sub(1));
    let width = decimal_width(last_line_number);

    let mut out = String::new();
    let mut rendered_lines: usize = 0;
    let mut byte_capped = false;
    for (offset, raw) in visible.iter().enumerate() {
        let line_number = start.saturating_add(offset);
        // Strip a trailing '\r' so CRLF-authored files render cleanly.
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let (body, line_trunc) = truncate_line(raw);
        // 6-space padding minimum to match Claude Code's visual style.
        let pad_width = width.max(6);
        let mut next = format!("{line_number:>pad_width$}→{body}");
        if line_trunc {
            next.push('…');
        }
        next.push('\n');
        if out.len().saturating_add(next.len()) > max_output_bytes {
            byte_capped = true;
            break;
        }
        out.push_str(&next);
        rendered_lines = rendered_lines.saturating_add(1);
    }

    let line_capped = rendered_lines < visible.len() || end_exclusive < total_lines;
    NumberedLines {
        text: out,
        total_lines,
        start_line: start,
        rendered_lines,
        truncated: line_capped || byte_capped,
    }
}

/// Result of [`format_numbered_lines`], including enough metadata for the
/// tool response payload to describe what was rendered.
#[derive(Debug, Clone)]
pub struct NumberedLines {
    pub text: String,
    pub total_lines: usize,
    pub start_line: usize,
    pub rendered_lines: usize,
    pub truncated: bool,
}

fn truncate_line(line: &str) -> (&str, bool) {
    if line.chars().count() <= MAX_LINE_CHARS {
        return (line, false);
    }
    // Find the byte index that covers MAX_LINE_CHARS chars.
    let mut iter = line.char_indices();
    let cutoff_byte = iter.nth(MAX_LINE_CHARS).map_or(line.len(), |(idx, _)| idx);
    (&line[..cutoff_byte], true)
}

fn decimal_width(n: usize) -> usize {
    let mut n = n.max(1);
    let mut width = 0usize;
    while n > 0 {
        width = width.saturating_add(1);
        n /= 10;
    }
    width
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        std::{
            io::Write,
            sync::atomic::{AtomicBool, Ordering},
            time::Duration,
        },
    };

    #[test]
    fn adaptive_read_cap_small_context_clamps_to_min() {
        // 8K tokens × 4 chars × 0.2 = 6400 bytes; below the 50 KB min.
        let cap = compute_adaptive_read_cap(8_000);
        assert_eq!(cap, MIN_ADAPTIVE_READ_BYTES);
    }

    #[test]
    fn adaptive_read_cap_medium_context_scales_linearly() {
        // 200K tokens × 4 chars × 0.2 = 160_000 bytes. Between min (50 KB)
        // and max (512 KB), so unclamped.
        let cap = compute_adaptive_read_cap(200_000);
        assert_eq!(cap, 160_000);
    }

    #[test]
    fn adaptive_read_cap_huge_context_clamps_to_max() {
        // 1M tokens × 4 chars × 0.2 = 800_000 bytes; above the 512 KB max.
        let cap = compute_adaptive_read_cap(1_000_000);
        assert_eq!(cap, MAX_ADAPTIVE_READ_BYTES);
    }

    #[test]
    fn adaptive_read_cap_zero_context_returns_min() {
        let cap = compute_adaptive_read_cap(0);
        assert_eq!(cap, MIN_ADAPTIVE_READ_BYTES);
    }

    #[test]
    fn format_numbered_lines_with_cap_respects_custom_ceiling() {
        // Render enough lines to blow a 5 KB cap but fit the default.
        let line = "x".repeat(100);
        let content: String = std::iter::repeat_n(line.as_str(), 200)
            .collect::<Vec<&str>>()
            .join("\n");

        let tight = format_numbered_lines_with_cap(&content, 1, 10_000, 5 * 1024);
        assert!(tight.truncated);
        assert!(tight.text.len() <= 5 * 1024);

        let loose = format_numbered_lines_with_cap(&content, 1, 10_000, 10 * 1024 * 1024);
        assert!(!loose.truncated);
        assert_eq!(loose.rendered_lines, 200);
    }

    #[test]
    fn fs_state_records_read_and_tracks_consecutive_reads() {
        let state = new_fs_state(false);
        let mut inner = state.lock().unwrap();
        let path = PathBuf::from("/tmp/x");

        let c1 = inner.record_read("s1", path.clone(), 1, 100, None);
        let c2 = inner.record_read("s1", path.clone(), 1, 100, None);
        let c3 = inner.record_read("s1", path.clone(), 1, 100, None);
        assert_eq!(c1, 1);
        assert_eq!(c2, 2);
        assert_eq!(c3, 3);
        assert!(c3 >= READ_LOOP_THRESHOLD);
    }

    #[test]
    fn fs_state_different_offsets_reset_counter() {
        let state = new_fs_state(false);
        let mut inner = state.lock().unwrap();
        let path = PathBuf::from("/tmp/x");

        assert_eq!(inner.record_read("s1", path.clone(), 1, 100, None), 1);
        assert_eq!(inner.record_read("s1", path.clone(), 10, 100, None), 1);
    }

    #[test]
    fn fs_state_has_been_read_tracks_across_reads() {
        let state = new_fs_state(false);
        let mut inner = state.lock().unwrap();
        let path = PathBuf::from("/tmp/x");

        assert!(!inner.has_been_read("s1", &path));
        inner.record_read("s1", path.clone(), 1, 100, None);
        assert!(inner.has_been_read("s1", &path));
        // Session isolation: a different session_key sees nothing.
        assert!(!inner.has_been_read("s2", &path));
    }

    #[test]
    fn fs_state_note_mutation_resets_loop_counter() {
        let state = new_fs_state(false);
        let mut inner = state.lock().unwrap();
        let path = PathBuf::from("/tmp/x");

        inner.record_read("s1", path.clone(), 1, 100, None);
        inner.record_read("s1", path.clone(), 1, 100, None);
        inner.note_mutation("s1", &path);
        // After note_mutation, the next read starts a fresh streak.
        assert_eq!(inner.record_read("s1", path.clone(), 1, 100, None), 1);
    }

    #[tokio::test]
    async fn mutation_lock_serializes_same_key() {
        let entered_second = Arc::new(AtomicBool::new(false));
        let entered_second_task = Arc::clone(&entered_second);

        let first = tokio::spawn(async move {
            with_fs_mutation_lock("host:/tmp/demo".to_string(), async {
                tokio::time::sleep(Duration::from_millis(100)).await;
            })
            .await;
        });

        tokio::time::sleep(Duration::from_millis(20)).await;

        let second = tokio::spawn(async move {
            with_fs_mutation_lock("host:/tmp/demo".to_string(), async {
                entered_second_task.store(true, Ordering::SeqCst);
            })
            .await;
        });

        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(
            !entered_second.load(Ordering::SeqCst),
            "second mutation should wait for the first lock holder"
        );

        first.await.unwrap();
        second.await.unwrap();
        assert!(entered_second.load(Ordering::SeqCst));
    }

    #[test]
    fn session_key_default_when_absent() {
        assert_eq!(session_key_from(&json!({})), "default");
    }

    #[test]
    fn session_key_reads_underscore_prefixed_param() {
        let params = json!({ "_session_key": "session:abc", "file_path": "/tmp/x" });
        assert_eq!(session_key_from(&params), "session:abc");
    }

    #[test]
    fn decimal_width_examples() {
        assert_eq!(decimal_width(0), 1);
        assert_eq!(decimal_width(1), 1);
        assert_eq!(decimal_width(9), 1);
        assert_eq!(decimal_width(10), 2);
        assert_eq!(decimal_width(999), 3);
    }

    #[test]
    fn format_numbered_lines_basic() {
        let content = "alpha\nbeta\ngamma\n";
        let rendered = format_numbered_lines(content, 1, 100);
        assert_eq!(rendered.total_lines, 3);
        assert_eq!(rendered.rendered_lines, 3);
        assert!(!rendered.truncated);
        assert_eq!(rendered.text, "     1→alpha\n     2→beta\n     3→gamma\n");
    }

    #[test]
    fn format_numbered_lines_respects_offset_and_limit() {
        let content = "a\nb\nc\nd\ne\n";
        let rendered = format_numbered_lines(content, 2, 2);
        assert_eq!(rendered.rendered_lines, 2);
        assert!(rendered.truncated);
        assert_eq!(rendered.text, "     2→b\n     3→c\n");
    }

    #[test]
    fn format_numbered_lines_truncates_long_lines() {
        let long = "x".repeat(MAX_LINE_CHARS + 50);
        let content = format!("{long}\n");
        let rendered = format_numbered_lines(&content, 1, 100);
        // Should end with the ellipsis marker before the newline.
        assert!(rendered.text.trim_end().ends_with('…'));
    }

    #[test]
    fn format_numbered_lines_strips_crlf() {
        let content = "alpha\r\nbeta\r\n";
        let rendered = format_numbered_lines(content, 1, 100);
        assert_eq!(rendered.text, "     1→alpha\n     2→beta\n");
    }

    #[test]
    fn format_numbered_lines_handles_no_trailing_newline() {
        let content = "alpha\nbeta";
        let rendered = format_numbered_lines(content, 1, 100);
        assert_eq!(rendered.total_lines, 2);
        assert_eq!(rendered.rendered_lines, 2);
        assert_eq!(rendered.text, "     1→alpha\n     2→beta\n");
    }

    #[test]
    fn format_numbered_lines_byte_caps_large_payloads() {
        // Build a file that would exceed MAX_READ_OUTPUT_BYTES if fully rendered.
        // Each line is ~100 chars; 3000 lines × 100 = 300 KB > 256 KB cap.
        let line = "x".repeat(100);
        let content: String = std::iter::repeat_n(line.as_str(), 3000)
            .collect::<Vec<&str>>()
            .join("\n");
        let rendered = format_numbered_lines(&content, 1, 10_000);

        assert!(rendered.truncated, "should be byte-capped");
        assert!(
            rendered.text.len() <= MAX_READ_OUTPUT_BYTES,
            "output {} > cap {}",
            rendered.text.len(),
            MAX_READ_OUTPUT_BYTES
        );
        assert!(rendered.rendered_lines < 3000);
        assert!(rendered.rendered_lines > 0);
    }

    #[test]
    fn format_numbered_lines_start_beyond_end() {
        let content = "a\nb\n";
        let rendered = format_numbered_lines(content, 10, 5);
        assert_eq!(rendered.total_lines, 2);
        assert_eq!(rendered.rendered_lines, 0);
        assert!(rendered.text.is_empty());
    }

    #[test]
    fn looks_binary_detects_null_bytes() {
        assert!(!looks_binary(b"hello world"));
        assert!(looks_binary(b"hello\0world"));
    }

    #[test]
    fn looks_binary_empty() {
        assert!(!looks_binary(b""));
    }

    #[tokio::test]
    async fn canonicalize_existing_resolves_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"hi").unwrap();
        let resolved = canonicalize_existing(tmp.path().to_str().unwrap())
            .await
            .unwrap();
        assert!(resolved.is_absolute());
    }

    #[tokio::test]
    async fn canonicalize_existing_missing_errors() {
        let err = canonicalize_existing("/tmp/definitely-does-not-exist-5b7c1")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot resolve path"));
    }

    #[tokio::test]
    async fn canonicalize_existing_rejects_relative() {
        let err = canonicalize_existing("relative/path.txt")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("must be an absolute path"));
    }

    #[tokio::test]
    async fn canonicalize_existing_rejects_empty() {
        let err = canonicalize_existing("").await.unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[tokio::test]
    async fn canonicalize_for_create_resolves_parent() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("new-file.txt");
        let resolved = canonicalize_for_create(target.to_str().unwrap())
            .await
            .unwrap();
        assert!(resolved.is_absolute());
        assert_eq!(resolved.file_name().unwrap(), "new-file.txt");
    }

    #[tokio::test]
    async fn canonicalize_for_create_missing_parent_errors() {
        let err = canonicalize_for_create("/tmp/does-not-exist-parent-99/out.txt")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot resolve parent"));
    }

    #[tokio::test]
    async fn canonicalize_for_create_rejects_relative() {
        let err = canonicalize_for_create("out.txt").await.unwrap_err();
        assert!(err.to_string().contains("must be an absolute path"));
    }

    #[test]
    fn fs_state_remove_session_drops_read_history() {
        let state = new_fs_state(false);
        let mut guard = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = guard.record_read("session-a", PathBuf::from("/tmp/demo.txt"), 0, 10, None);
        assert!(guard.has_been_read("session-a", Path::new("/tmp/demo.txt")));

        guard.remove_session("session-a");
        assert!(!guard.has_been_read("session-a", Path::new("/tmp/demo.txt")));
    }

    #[tokio::test]
    async fn ensure_regular_file_rejects_dir() {
        let dir = tempfile::tempdir().unwrap();
        let err = ensure_regular_file(dir.path()).await.unwrap_err();
        assert!(err.to_string().contains("not a regular file"));
    }
}
