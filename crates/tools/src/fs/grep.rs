//! `Grep` tool — regex content search across files.
//!
//! Walks the target with `ignore::WalkBuilder` (respects `.gitignore`),
//! applies an optional `glob` file filter, and searches each file with
//! ripgrep's `grep-regex`/`grep-searcher` crates. Supports three output
//! modes mirroring Claude Code / rg:
//!
//! - `content` — matching lines with optional line numbers and context.
//! - `files_with_matches` (default) — just the paths of files that matched.
//! - `count` — match count per file.
//!
//! We keep search in-process so the tool returns structured results and
//! honors the same policy layer as every other native tool.

use {
    async_trait::async_trait,
    globset::{Glob as GlobPattern, GlobMatcher},
    grep_matcher::Matcher,
    grep_regex::RegexMatcherBuilder,
    grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch},
    ignore::WalkBuilder,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::{
        io,
        path::{Path, PathBuf},
        sync::Arc,
    },
    tracing::instrument,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, labels, tools as tools_metrics};

use crate::{
    Result,
    error::Error,
    fs::{
        sandbox_bridge::{SandboxGrepMode, SandboxGrepOptions},
        shared::{FsPathPolicy, enforce_path_policy_deny_only, require_absolute, session_key_from},
    },
    sandbox::{SandboxRouter, file_system::sandbox_file_system_for_session},
};

/// Maximum bytes of a single file we will load for content searching.
const DEFAULT_GREP_FILE_CAP: u64 = 5 * 1024 * 1024;

/// Maximum match rows returned by a single `content`-mode call.
const DEFAULT_GREP_MATCH_CAP: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Content,
    FilesWithMatches,
    Count,
}

impl OutputMode {
    fn parse(raw: Option<&str>) -> Result<Self> {
        match raw {
            None | Some("files_with_matches") => Ok(Self::FilesWithMatches),
            Some("content") => Ok(Self::Content),
            Some("count") => Ok(Self::Count),
            Some(other) => Err(Error::message(format!(
                "invalid output_mode '{other}' — expected 'content', 'files_with_matches', or 'count'"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
struct GrepOptions {
    pattern: String,
    path: PathBuf,
    glob: Option<GlobMatcher>,
    file_type: Option<String>,
    output_mode: OutputMode,
    case_insensitive: bool,
    show_line_numbers: bool,
    before_context: usize,
    after_context: usize,
    multiline: bool,
    head_limit: Option<usize>,
    offset: usize,
}

/// Native `Grep` tool implementation.
pub struct GrepTool {
    /// Optional default root used when the LLM call omits `path`.
    workspace_root: Option<PathBuf>,
    /// Optional allow/deny path policy. Rejects the whole call if the
    /// search root is denied; filters individual matching files
    /// otherwise.
    path_policy: Option<FsPathPolicy>,
    /// Whether to respect `.gitignore` while walking. Default `true`.
    respect_gitignore: bool,
    /// When set and the session is sandboxed, dispatch through the
    /// bridge's `sandbox_grep` helper instead of walking the host.
    sandbox_router: Option<Arc<SandboxRouter>>,
}

impl Default for GrepTool {
    fn default() -> Self {
        Self {
            workspace_root: None,
            path_policy: None,
            respect_gitignore: true,
            sandbox_router: None,
        }
    }
}

impl GrepTool {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the default search root for calls that omit `path`.
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

    /// Attach a shared [`SandboxRouter`]. Sandboxed sessions dispatch
    /// through the bridge's `sandbox_grep` helper.
    #[must_use]
    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }

    #[instrument(skip(self, opts), fields(pattern = %opts.pattern, mode = ?opts.output_mode))]
    fn grep_impl(&self, opts: GrepOptions) -> Result<Value> {
        let root_canonical = std::fs::canonicalize(&opts.path).map_err(|e| {
            Error::message(format!(
                "cannot resolve grep path '{}': {e}",
                opts.path.display()
            ))
        })?;

        // Reject the whole call only if the root is explicitly denied.
        // Allow-list filtering happens per-file below.
        if let Some(ref policy) = self.path_policy
            && let Some(payload) = enforce_path_policy_deny_only(policy, &root_canonical)
        {
            return Ok(payload);
        }

        // If the root is a single file, search just that file.
        let is_file = std::fs::metadata(&root_canonical)
            .map(|m| m.is_file())
            .unwrap_or(false);

        let mut files: Vec<PathBuf> = Vec::new();
        if is_file {
            files.push(root_canonical.clone());
        } else {
            let walker = WalkBuilder::new(&root_canonical)
                .hidden(false)
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
                let path = entry.path().to_path_buf();
                if let Some(ref glob) = opts.glob {
                    let relative = path.strip_prefix(&root_canonical).unwrap_or(&path);
                    if !glob.is_match(relative) {
                        continue;
                    }
                }
                if let Some(ref ft) = opts.file_type
                    && !file_matches_type(&path, ft)
                {
                    continue;
                }
                // Per-file policy filter so allow/deny can carve
                // sub-trees beneath the search root.
                if let Some(ref policy) = self.path_policy
                    && policy.check(&path).is_some()
                {
                    continue;
                }
                files.push(path);
            }
        }

        // Sort files for deterministic output.
        files.sort();

        let matcher = build_matcher(&opts)?;

        match opts.output_mode {
            OutputMode::FilesWithMatches => grep_files_with_matches(&matcher, &files, &opts),
            OutputMode::Count => grep_count_matches(&matcher, &files, &opts),
            OutputMode::Content => grep_content_matches(&matcher, &files, &opts),
        }
    }
}

fn build_matcher(opts: &GrepOptions) -> Result<grep_regex::RegexMatcher> {
    let mut builder = RegexMatcherBuilder::new();
    builder
        .case_insensitive(opts.case_insensitive)
        .multi_line(opts.multiline)
        .dot_matches_new_line(opts.multiline)
        .ban_byte(Some(0));

    if !opts.multiline {
        builder.line_terminator(Some(b'\n'));
    }

    builder
        .build(&opts.pattern)
        .map_err(|e| Error::message(format!("invalid regex '{}': {e}", opts.pattern)))
}

fn read_bounded(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > DEFAULT_GREP_FILE_CAP {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    String::from_utf8(bytes).ok()
}

fn build_searcher(multiline: bool, line_numbers: bool, max_matches: Option<u64>) -> Searcher {
    let mut builder = SearcherBuilder::new();
    builder
        .binary_detection(BinaryDetection::quit(0))
        .line_number(line_numbers)
        .max_matches(max_matches);
    if multiline {
        builder.multi_line(true);
    }
    builder.build()
}

fn grep_files_with_matches(
    matcher: &grep_regex::RegexMatcher,
    files: &[PathBuf],
    opts: &GrepOptions,
) -> Result<Value> {
    let mut searcher = build_searcher(opts.multiline, false, Some(1));
    let mut matched: Vec<String> = Vec::new();

    for path in files {
        let mut sink = FirstMatchSink::default();
        if searcher.search_path(matcher, path, &mut sink).is_ok() && sink.found {
            matched.push(path.to_string_lossy().into_owned());
        }
    }

    let (rows, truncated) = apply_head_offset(matched, opts.offset, opts.head_limit);
    Ok(json!({
        "mode": "files_with_matches",
        "files": rows,
        "truncated": truncated,
    }))
}

fn grep_count_matches(
    matcher: &grep_regex::RegexMatcher,
    files: &[PathBuf],
    opts: &GrepOptions,
) -> Result<Value> {
    let mut searcher = build_searcher(opts.multiline, false, None);
    let mut counts: Vec<Value> = Vec::new();

    for path in files {
        let mut sink = CountMatchesSink::new(matcher);
        if searcher.search_path(matcher, path, &mut sink).is_ok() && sink.count > 0 {
            counts.push(json!({
                "path": path.to_string_lossy(),
                "count": sink.count,
            }));
        }
    }

    let (rows, truncated) = apply_head_offset(counts, opts.offset, opts.head_limit);
    Ok(json!({
        "mode": "count",
        "counts": rows,
        "truncated": truncated,
    }))
}

fn grep_content_matches(
    matcher: &grep_regex::RegexMatcher,
    files: &[PathBuf],
    opts: &GrepOptions,
) -> Result<Value> {
    // Keep content mode line-oriented so each row still corresponds to one
    // concrete line with a local context block.
    let mut searcher = build_searcher(false, true, None);
    let mut rows: Vec<Value> = Vec::new();
    let mut cap_hit = false;

    'outer: for path in files {
        let mut sink = MatchingLineNumbersSink::default();
        if searcher.search_path(matcher, path, &mut sink).is_err() || sink.line_numbers.is_empty() {
            continue;
        }
        let Some(text) = read_bounded(path) else {
            continue;
        };
        let file_rows = collect_content_matches(
            path,
            &text,
            &sink.line_numbers,
            opts.show_line_numbers,
            opts.before_context,
            opts.after_context,
        );
        for row in file_rows {
            if rows.len() >= DEFAULT_GREP_MATCH_CAP {
                cap_hit = true;
                break 'outer;
            }
            rows.push(row);
        }
    }

    let (rows, paging_truncated) = apply_head_offset(rows, opts.offset, opts.head_limit);
    Ok(json!({
        "mode": "content",
        "matches": rows,
        "truncated": cap_hit || paging_truncated,
    }))
}

fn collect_content_matches(
    path: &Path,
    text: &str,
    line_numbers: &[usize],
    show_line_numbers: bool,
    before: usize,
    after: usize,
) -> Vec<Value> {
    let lines: Vec<&str> = text.lines().collect();
    let mut rows: Vec<Value> = Vec::new();
    for &line_number in line_numbers {
        let idx = line_number.saturating_sub(1);
        let Some(line) = lines.get(idx) else {
            continue;
        };
        let start = idx.saturating_sub(before);
        let end = (idx + after + 1).min(lines.len());
        let block: Vec<String> = lines[start..end]
            .iter()
            .enumerate()
            .map(|(offset, line)| {
                let ctx_idx = start + offset;
                if show_line_numbers {
                    format!("{}:{line}", ctx_idx + 1)
                } else {
                    (*line).to_string()
                }
            })
            .collect();
        rows.push(json!({
            "path": path.to_string_lossy(),
            "line": idx + 1,
            "match": line,
            "block": block,
        }));
    }
    rows
}

/// Post-filter sandbox Grep results against the path policy.
///
/// The sandbox bridge can't evaluate the policy inside the container,
/// so we filter the structured JSON on the host side after the results
/// come back. Mirrors the per-file filter in the host `grep_impl` and
/// in `Glob`'s sandbox branch.
fn filter_sandbox_grep_by_policy(result: &mut Value, policy: &FsPathPolicy) {
    let denied = |path_value: &Value| -> bool {
        path_value
            .as_str()
            .is_some_and(|p| policy.check(Path::new(p)).is_some())
    };

    // files_with_matches mode
    if let Some(arr) = result.get_mut("files").and_then(Value::as_array_mut) {
        arr.retain(|f| !denied(f));
    }
    // content mode
    if let Some(arr) = result.get_mut("matches").and_then(Value::as_array_mut) {
        arr.retain(|m| !denied(m.get("path").unwrap_or(&Value::Null)));
    }
    // count mode
    if let Some(arr) = result.get_mut("counts").and_then(Value::as_array_mut) {
        arr.retain(|c| !denied(c.get("path").unwrap_or(&Value::Null)));
    }
}

fn apply_head_offset<T: Clone>(
    rows: Vec<T>,
    offset: usize,
    head_limit: Option<usize>,
) -> (Vec<T>, bool) {
    let total = rows.len();
    let start = offset.min(total);
    let slice = &rows[start..];
    let (capped, truncated) = match head_limit {
        Some(limit) if slice.len() > limit => (&slice[..limit], true),
        _ => (slice, false),
    };
    (capped.to_vec(), truncated)
}

#[derive(Default)]
struct FirstMatchSink {
    found: bool,
}

impl Sink for FirstMatchSink {
    type Error = io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        _mat: &SinkMatch<'_>,
    ) -> std::result::Result<bool, Self::Error> {
        self.found = true;
        Ok(false)
    }
}

struct CountMatchesSink<'a, M> {
    matcher: &'a M,
    count: usize,
}

impl<'a, M> CountMatchesSink<'a, M> {
    fn new(matcher: &'a M) -> Self {
        Self { matcher, count: 0 }
    }
}

impl<M> Sink for CountMatchesSink<'_, M>
where
    M: Matcher,
{
    type Error = io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        mat: &SinkMatch<'_>,
    ) -> std::result::Result<bool, Self::Error> {
        self.matcher
            .find_iter(mat.bytes(), |m| {
                let _ = m;
                self.count += 1;
                true
            })
            .map_err(|err| io::Error::other(err.to_string()))?;
        Ok(true)
    }
}

#[derive(Default)]
struct MatchingLineNumbersSink {
    line_numbers: Vec<usize>,
}

impl Sink for MatchingLineNumbersSink {
    type Error = io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        mat: &SinkMatch<'_>,
    ) -> std::result::Result<bool, Self::Error> {
        if let Some(line_number) = mat.line_number() {
            self.line_numbers.push(line_number as usize);
        }
        Ok(true)
    }
}

/// Map a `type` filter to the grep `--include` globs that cover that
/// language family. Unknown types return an empty list, which means the
/// caller falls back to no include filter.
fn type_to_include_globs(ty: &str) -> &'static [&'static str] {
    match ty {
        "rust" => &["*.rs"],
        "py" | "python" => &["*.py"],
        "js" => &["*.js", "*.mjs", "*.cjs"],
        "ts" => &["*.ts", "*.tsx"],
        "tsx" => &["*.tsx"],
        "jsx" => &["*.jsx"],
        "go" => &["*.go"],
        "java" => &["*.java"],
        "c" => &["*.c", "*.h"],
        "cpp" => &["*.cpp", "*.cc", "*.cxx", "*.hpp", "*.hh"],
        "md" => &["*.md", "*.markdown"],
        "toml" => &["*.toml"],
        "yaml" => &["*.yaml", "*.yml"],
        "json" => &["*.json"],
        "html" => &["*.html", "*.htm"],
        "css" => &["*.css"],
        "sh" => &["*.sh", "*.bash"],
        _ => &[],
    }
}

fn file_matches_type(path: &Path, ty: &str) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        (ty, ext),
        ("rust", "rs")
            | ("py", "py")
            | ("python", "py")
            | ("js", "js" | "mjs" | "cjs")
            | ("ts", "ts" | "tsx")
            | ("tsx", "tsx")
            | ("jsx", "jsx")
            | ("go", "go")
            | ("java", "java")
            | ("c", "c" | "h")
            | ("cpp", "cpp" | "cc" | "cxx" | "hpp" | "hh")
            | ("md", "md" | "markdown")
            | ("toml", "toml")
            | ("yaml", "yaml" | "yml")
            | ("json", "json")
            | ("html", "html" | "htm")
            | ("css", "css")
            | ("sh", "sh" | "bash")
    )
}

#[async_trait]
impl AgentTool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "Search file contents with a regular expression. Three output modes: \
         'files_with_matches' (default, lists matching file paths), 'content' \
         (lines that matched, with optional context), 'count' (match count \
         per file). Supports glob and file-type filters, case-insensitive \
         matching, line numbers, and context lines."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression pattern (Rust regex syntax)."
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search. Defaults to the current working directory."
                },
                "glob": {
                    "type": "string",
                    "description": "Glob filter for file paths (e.g. '**/*.rs')."
                },
                "type": {
                    "type": "string",
                    "description": "File type filter (e.g. 'rust', 'py', 'ts')."
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "default": "files_with_matches"
                },
                "-i": {
                    "type": "boolean",
                    "description": "Case-insensitive match."
                },
                "-n": {
                    "type": "boolean",
                    "description": "Show line numbers (content mode)."
                },
                "-A": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Lines of trailing context (content mode)."
                },
                "-B": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Lines of leading context (content mode)."
                },
                "-C": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Lines of context on both sides (content mode)."
                },
                "context": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Alias for -C."
                },
                "multiline": {
                    "type": "boolean",
                    "description": "Enable multi-line mode (`.` matches newlines)."
                },
                "head_limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum rows to return after offset."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Skip this many rows before applying head_limit."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let pattern = params
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'pattern' parameter"))?
            .to_string();

        let path = match params.get("path").and_then(Value::as_str) {
            Some(raw) => {
                require_absolute(raw, "path")?;
                PathBuf::from(raw)
            },
            None => self.workspace_root.clone().ok_or_else(|| {
                Error::message(
                    "Grep requires an absolute 'path' argument (no workspace root is configured)",
                )
            })?,
        };

        let glob = params
            .get("glob")
            .and_then(Value::as_str)
            .map(|raw| -> Result<GlobMatcher> {
                Ok(GlobPattern::new(raw)
                    .map_err(|e| Error::message(format!("invalid glob '{raw}': {e}")))?
                    .compile_matcher())
            })
            .transpose()?;

        let file_type = params
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string);

        let output_mode = OutputMode::parse(params.get("output_mode").and_then(Value::as_str))?;

        let case_insensitive = params.get("-i").and_then(Value::as_bool).unwrap_or(false);
        let show_line_numbers = params.get("-n").and_then(Value::as_bool).unwrap_or(false);
        let context_symmetric = params
            .get("-C")
            .or_else(|| params.get("context"))
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(0);
        let before_context = params
            .get("-B")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(context_symmetric);
        let after_context = params
            .get("-A")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(context_symmetric);
        let multiline = params
            .get("multiline")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let head_limit = params
            .get("head_limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize);
        let offset = params
            .get("offset")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(0);

        // Sandbox dispatch: shell grep into the container, then apply
        // the same paging and truncation rules on the host side so the
        // LLM-facing payload stays consistent across routing modes.
        if let Some(ref router) = self.sandbox_router {
            let session_key = session_key_from(&params).to_string();
            if router.is_sandboxed(&session_key).await {
                // Enforce path policy before dispatching to the sandbox,
                // matching Read/Write/Edit/MultiEdit.
                if let Some(ref policy) = self.path_policy
                    && let Some(payload) = enforce_path_policy_deny_only(policy, &path)
                {
                    return Ok(payload);
                }
                let path_str = path
                    .to_str()
                    .ok_or_else(|| Error::message("Grep 'path' contains invalid UTF-8"))?;
                let mode = match output_mode {
                    OutputMode::Content => SandboxGrepMode::Content,
                    OutputMode::FilesWithMatches => SandboxGrepMode::FilesWithMatches,
                    OutputMode::Count => SandboxGrepMode::Count,
                };
                // Map `type` to grep --include globs. `glob` takes
                // precedence when both are set.
                let include_globs: Vec<String> = match params.get("glob").and_then(Value::as_str) {
                    Some(g) => vec![g.to_string()],
                    None => file_type
                        .as_deref()
                        .map(type_to_include_globs)
                        .unwrap_or_default()
                        .iter()
                        .map(|glob| (*glob).to_string())
                        .collect(),
                };
                let sandbox_fs = sandbox_file_system_for_session(router, &session_key).await?;
                let mut result = sandbox_fs
                    .grep(SandboxGrepOptions {
                        pattern: pattern.clone(),
                        path: path_str.to_string(),
                        mode,
                        case_insensitive,
                        include_globs,
                        offset,
                        head_limit,
                        match_cap: (output_mode == OutputMode::Content)
                            .then_some(DEFAULT_GREP_MATCH_CAP),
                    })
                    .await?;

                // Per-file path policy filter on sandbox results,
                // mirroring what Glob's sandbox branch does and what
                // the host grep_impl does via its walk filter.
                if let Some(ref policy) = self.path_policy {
                    filter_sandbox_grep_by_policy(&mut result, policy);
                }

                return Ok(result);
            }
        }

        let opts = GrepOptions {
            pattern,
            path,
            glob,
            file_type,
            output_mode,
            case_insensitive,
            show_line_numbers,
            before_context,
            after_context,
            multiline,
            head_limit,
            offset,
        };

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
            tool.grep_impl(opts)
        })
        .await
        .map_err(|e| Error::message(format!("grep task failed: {e}")))?;

        match result {
            Ok(value) => {
                #[cfg(feature = "metrics")]
                counter!(
                    tools_metrics::EXECUTIONS_TOTAL,
                    labels::TOOL => "Grep".to_string(),
                    labels::SUCCESS => "true".to_string()
                )
                .increment(1);
                Ok(value)
            },
            Err(e) => {
                #[cfg(feature = "metrics")]
                counter!(
                    tools_metrics::EXECUTION_ERRORS_TOTAL,
                    labels::TOOL => "Grep".to_string()
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
    use super::*;

    async fn setup_tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            dir.path().join("lib.rs"),
            "fn alpha() {}\nfn beta() {}\nfn gamma() {}\n",
        )
        .await
        .unwrap();
        tokio::fs::write(dir.path().join("notes.md"), "alpha notes\nbeta notes\n")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("binary.bin"), [0u8, 1, 2, 3, 4, 5])
            .await
            .unwrap();
        dir
    }

    #[tokio::test]
    async fn grep_files_with_matches_default() {
        let dir = setup_tree().await;
        let tool = GrepTool::new();
        let value = tool
            .execute(json!({
                "pattern": "beta",
                "path": dir.path().to_str().unwrap(),
            }))
            .await
            .unwrap();

        assert_eq!(value["mode"], "files_with_matches");
        let files: Vec<String> = value["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|p| p.ends_with("lib.rs")));
        assert!(files.iter().any(|p| p.ends_with("notes.md")));
    }

    #[tokio::test]
    async fn grep_count_mode() {
        let dir = setup_tree().await;
        let tool = GrepTool::new();
        let value = tool
            .execute(json!({
                "pattern": "fn ",
                "path": dir.path().to_str().unwrap(),
                "output_mode": "count",
            }))
            .await
            .unwrap();

        let counts = value["counts"].as_array().unwrap();
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0]["count"], 3);
    }

    #[tokio::test]
    async fn grep_count_mode_counts_multiple_occurrences_on_one_line() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("dups.txt"), "alpha alpha alpha\n")
            .await
            .unwrap();

        let tool = GrepTool::new();
        let value = tool
            .execute(json!({
                "pattern": "alpha",
                "path": dir.path().to_str().unwrap(),
                "output_mode": "count",
            }))
            .await
            .unwrap();

        let counts = value["counts"].as_array().unwrap();
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0]["count"], 3);
    }

    #[tokio::test]
    async fn grep_content_mode_with_line_numbers() {
        let dir = setup_tree().await;
        let tool = GrepTool::new();
        let value = tool
            .execute(json!({
                "pattern": "beta",
                "path": dir.path().to_str().unwrap(),
                "output_mode": "content",
                "-n": true,
            }))
            .await
            .unwrap();

        let matches = value["matches"].as_array().unwrap();
        assert!(!matches.is_empty());
        let first = &matches[0];
        let block = first["block"].as_array().unwrap();
        assert!(block[0].as_str().unwrap().contains(':'));
    }

    #[tokio::test]
    async fn grep_case_insensitive() {
        let dir = setup_tree().await;
        let tool = GrepTool::new();
        let value = tool
            .execute(json!({
                "pattern": "ALPHA",
                "path": dir.path().to_str().unwrap(),
                "-i": true,
            }))
            .await
            .unwrap();

        let files = value["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
    }

    #[tokio::test]
    async fn grep_glob_filter() {
        let dir = setup_tree().await;
        let tool = GrepTool::new();
        let value = tool
            .execute(json!({
                "pattern": "alpha",
                "path": dir.path().to_str().unwrap(),
                "glob": "*.rs",
            }))
            .await
            .unwrap();

        let files = value["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].as_str().unwrap().ends_with("lib.rs"));
    }

    #[tokio::test]
    async fn grep_type_filter() {
        let dir = setup_tree().await;
        let tool = GrepTool::new();
        let value = tool
            .execute(json!({
                "pattern": "alpha",
                "path": dir.path().to_str().unwrap(),
                "type": "rust",
            }))
            .await
            .unwrap();

        let files = value["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].as_str().unwrap().ends_with("lib.rs"));
    }

    #[tokio::test]
    async fn grep_head_limit_and_offset() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            tokio::fs::write(dir.path().join(format!("f{i}.txt")), "match\n")
                .await
                .unwrap();
        }
        let tool = GrepTool::new();
        let value = tool
            .execute(json!({
                "pattern": "match",
                "path": dir.path().to_str().unwrap(),
                "head_limit": 2,
                "offset": 1,
            }))
            .await
            .unwrap();

        let files = value["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(value["truncated"], true);
    }

    #[tokio::test]
    async fn grep_invalid_regex_errors() {
        let dir = tempfile::tempdir().unwrap();
        let tool = GrepTool::new();
        let err = tool
            .execute(json!({
                "pattern": "[invalid",
                "path": dir.path().to_str().unwrap(),
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid regex"));
    }

    #[tokio::test]
    async fn grep_missing_path_without_workspace_root_errors() {
        let tool = GrepTool::new();
        let err = tool.execute(json!({ "pattern": "foo" })).await.unwrap_err();
        assert!(err.to_string().contains("no workspace root"));
    }

    #[tokio::test]
    async fn grep_rejects_relative_path() {
        let tool = GrepTool::new();
        let err = tool
            .execute(json!({
                "pattern": "foo",
                "path": "rel/dir",
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("must be an absolute path"));
    }

    #[tokio::test]
    async fn grep_falls_back_to_workspace_root() {
        let dir = setup_tree().await;
        let tool = GrepTool::new().with_workspace_root(dir.path().to_path_buf());
        let value = tool.execute(json!({ "pattern": "alpha" })).await.unwrap();
        let files = value["files"].as_array().unwrap();
        assert!(!files.is_empty());
    }

    #[tokio::test]
    async fn grep_single_file_path() {
        let dir = setup_tree().await;
        let tool = GrepTool::new();
        let value = tool
            .execute(json!({
                "pattern": "alpha",
                "path": dir.path().join("lib.rs").to_str().unwrap(),
            }))
            .await
            .unwrap();

        let files = value["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
    }
}
