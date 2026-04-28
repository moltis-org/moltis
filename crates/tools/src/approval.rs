use std::{collections::HashSet, sync::Arc, time::Duration};

use {
    crate::error::Error,
    regex::RegexSet,
    serde::{Deserialize, Serialize},
    tokio::sync::{RwLock, oneshot},
    tracing::{debug, warn},
};

use crate::Result;

/// Outcome of an approval request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalDecision {
    Approved,
    Denied,
    Timeout,
}

/// Approval mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum ApprovalMode {
    Off,
    #[default]
    OnMiss,
    Always,
}

impl ApprovalMode {
    /// Parse approval mode from config value.
    ///
    /// Accepts canonical values plus legacy aliases:
    /// - `on-miss` / `smart` -> `OnMiss`
    /// - `off` / `never` -> `Off`
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "never" => Some(Self::Off),
            "on-miss" | "on_miss" | "smart" => Some(Self::OnMiss),
            "always" => Some(Self::Always),
            _ => None,
        }
    }
}

/// Security level for exec commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SecurityLevel {
    Deny,
    #[default]
    Allowlist,
    Full,
}

impl SecurityLevel {
    /// Parse security level from config value.
    ///
    /// Accepts canonical values plus schema aliases:
    /// - `allowlist` -> `Allowlist`
    /// - `permissive` / `full` -> `Full`
    /// - `strict` / `deny` -> `Deny`
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "allowlist" => Some(Self::Allowlist),
            "permissive" | "full" => Some(Self::Full),
            "strict" | "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// Well-known safe binaries that don't need approval.
pub const SAFE_BINS: &[&str] = &[
    "cat",
    "echo",
    "printf",
    "head",
    "tail",
    "wc",
    "sort",
    "uniq",
    "cut",
    "tr",
    "grep",
    "egrep",
    "fgrep",
    "awk",
    "sed",
    "jq",
    "yq",
    "date",
    "cal",
    "ls",
    "pwd",
    "whoami",
    "hostname",
    "uname",
    "env",
    "printenv",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "diff",
    "comm",
    "paste",
    "tee",
    "xargs",
    "true",
    "false",
    "test",
    "[",
    "seq",
    "yes",
    "rev",
    "fold",
    "expand",
    "unexpand",
    "md5sum",
    "sha256sum",
    "sha1sum",
    "b2sum",
    "file",
    "stat",
    "du",
    "df",
    "free",
    "which",
    "type",
    "command",
];

/// Environment variable names that can hijack process execution.
///
/// Used by both `DANGEROUS_PATTERN_DEFS` (regex layer) and `extract_first_bin`
/// (semantic layer) for defense-in-depth against env-var prefix injection
/// (moltis-org/moltis#814).
const DANGEROUS_ENV_VARS: &[&str] = &[
    // Linux dynamic linker (LD_DEBUG excluded — diagnostic only, no code injection)
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "LD_CONFIG",
    // macOS dynamic linker
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
    // PATH override
    "PATH",
    // Language runtimes
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "NODE_OPTIONS",
    "NODE_PATH",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "PERL5OPT",
    "PERL5LIB",
    "RUBYOPT",
    "RUBYLIB",
    "CLASSPATH",
    // Shell startup (bare ENV excluded — only dangerous for sh/bash interactive
    // startup, too noisy for general use: ENV=test, ENV=production, etc.)
    "BASH_ENV",
    "ZDOTDIR",
];

/// Dangerous command patterns that force approval even when `approval_mode` is
/// off or `security_level` is full.  Each entry: `(regex_pattern, description)`.
static DANGEROUS_PATTERN_DEFS: &[(&str, &str)] = &[
    // Filesystem destruction
    (
        r"rm\s+(-\S*[rR]\S*\s+)*/(\s|$|\*)",
        "rm -r on filesystem root",
    ),
    (
        r"rm\s+(-\S*[rR]\S*\s+)+(~|\$HOME)",
        "rm -r on home directory",
    ),
    (r"\bmkfs\b", "make filesystem"),
    (
        r"\bdd\b.*\bif=/dev/(zero|urandom)\b",
        "disk overwrite with dd",
    ),
    (r":\(\)\s*\{.*\|.*&\s*\}\s*;", "fork bomb"),
    // Git destructive operations
    (r"git\s+reset\s+--hard", "git reset --hard"),
    (
        r"git\s+push\s+.*(-\S*f\S*|--force\b|--force-with-lease\b)",
        "git force push",
    ),
    (r"git\s+clean\s+(-\S*f)", "git clean with force"),
    (r"git\s+stash\s+(drop|clear)\b", "git stash drop/clear"),
    // Database destruction
    (
        r"(?i)\bDROP\s+(TABLE|DATABASE|SCHEMA)\b",
        "DROP TABLE/DATABASE",
    ),
    (r"(?i)\bTRUNCATE\b", "TRUNCATE"),
    // Container / infrastructure destruction
    (r"docker\s+system\s+prune", "docker system prune"),
    (r"kubectl\s+delete\s+namespace", "kubectl delete namespace"),
    (r"terraform\s+destroy", "terraform destroy"),
    // System-level danger
    (
        r"chmod\s+(-\S*R\S*\s+)*777\s+/",
        "recursive chmod 777 on root",
    ),
    // Inline environment variable injection (moltis-org/moltis#814).
    //
    // Anchored with `(?:^|[;&|]\s*)` so patterns only fire at command-start
    // positions (start of string, after `;`, `&`, `|`), not inside grep/sed
    // arguments like `grep PATH=/usr/bin .env`. Requires `=\S` (non-empty
    // value) to further reduce false positives.
    //
    // Chained assignments (`FOO=bar PATH=/evil cat`) are NOT caught here
    // because there is no separator between them — Layer 2 in
    // `extract_first_bin` handles that case instead. Subshell injection
    // (`sh -c "LD_PRELOAD=..."`) is covered by `sh`/`bash` not being safe
    // bins and requiring approval separately.
    (
        r"(?i)(?:^|[;&|]\s*)(LD_PRELOAD|LD_LIBRARY_PATH|LD_AUDIT|LD_CONFIG)=\S",
        "dangerous dynamic linker env var",
    ),
    (
        r"(?i)(?:^|[;&|]\s*)(DYLD_INSERT_LIBRARIES|DYLD_LIBRARY_PATH|DYLD_FRAMEWORK_PATH)=\S",
        "dangerous macOS dynamic linker env var",
    ),
    (r"(?i)(?:^|[;&|]\s*)PATH=\S", "PATH override"),
    (
        r"(?i)(?:^|[;&|]\s*)(PYTHONPATH|PYTHONSTARTUP|NODE_OPTIONS|NODE_PATH|JAVA_TOOL_OPTIONS|_JAVA_OPTIONS|JDK_JAVA_OPTIONS)=\S",
        "dangerous language runtime env var",
    ),
    (
        r"(?i)(?:^|[;&|]\s*)(PERL5OPT|PERL5LIB|RUBYOPT|RUBYLIB|CLASSPATH)=\S",
        "dangerous language runtime env var",
    ),
    (
        r"(?i)(?:^|[;&|]\s*)(BASH_ENV|ZDOTDIR)=\S",
        "dangerous shell startup env var",
    ),
];

static DANGEROUS_SET: std::sync::LazyLock<RegexSet> = std::sync::LazyLock::new(|| {
    RegexSet::new(DANGEROUS_PATTERN_DEFS.iter().map(|(p, _)| *p))
        .unwrap_or_else(|e| panic!("built-in dangerous patterns must be valid regex: {e}"))
});

/// Check if a command matches any dangerous pattern.
/// Returns the description of the first matching pattern.
pub fn check_dangerous(command: &str) -> Option<&'static str> {
    DANGEROUS_SET
        .matches(command)
        .iter()
        .next()
        .map(|i| DANGEROUS_PATTERN_DEFS[i].1)
}

/// Extract the first command/binary from a shell command string.
///
/// Returns `None` when the command is empty **or** when a leading env-var
/// assignment uses a dangerous variable name (see [`DANGEROUS_ENV_VARS`]).
/// This prevents attackers from smuggling `LD_PRELOAD=… cat /file` through the
/// safe-bin / allowlist path (moltis-org/moltis#814).
///
/// **Limitation:** Quoted tokens like `"LD_PRELOAD=/evil.so" cat /file` will
/// not be caught here because `split_once('=')` sees key `"LD_PRELOAD` (with
/// a leading `"`). The anchored regex layer (Layer 1) also does not match
/// this case. In practice this is not exploitable: shells treat the quoted
/// string as a command name, not an assignment. Subshell wrappers like
/// `sh -c "LD_PRELOAD=..."` are covered by `sh` not being a safe bin.
fn extract_first_bin(command: &str) -> Option<&str> {
    let trimmed = command.trim();
    // Skip env var assignments at the start (e.g. `FOO=bar cmd`).
    let mut parts = trimmed.split_whitespace();
    for part in parts.by_ref() {
        if let Some((key, _)) = part.split_once('=') {
            // Dangerous env-var prefix — refuse to extract a binary so the
            // caller falls through to the approval / denial path.
            if DANGEROUS_ENV_VARS
                .iter()
                .any(|d| d.eq_ignore_ascii_case(key))
            {
                return None;
            }
        } else {
            // Strip path prefix (e.g. `/usr/bin/jq` → `jq`).
            return Some(part.rsplit('/').next().unwrap_or(part));
        }
    }
    None
}

/// Check if a command is on the safe bins list.
pub fn is_safe_command(command: &str) -> bool {
    if let Some(bin) = extract_first_bin(command) {
        SAFE_BINS.contains(&bin)
    } else {
        false
    }
}

/// Check if a command matches any pattern in an allowlist.
pub fn matches_allowlist(command: &str, allowlist: &[String]) -> bool {
    let bin = extract_first_bin(command);
    let bin_name = bin.unwrap_or("");
    for pattern in allowlist {
        if pattern == "*" {
            return true;
        }
        if pattern == bin_name {
            return true;
        }
        // Prefix match with wildcard.
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            // Only match against the raw command string when we extracted a
            // valid binary. When extract_first_bin returned None (dangerous
            // env-var prefix), raw-string matching would let chained
            // assignments like `MY_APP=1 LD_PRELOAD=/evil.so cat` bypass
            // the env-var protection (moltis-org/moltis#814).
            if bin_name.starts_with(prefix) || (bin.is_some() && command.starts_with(prefix)) {
                return true;
            }
        }
    }
    false
}

/// Pending approval request waiting for gateway resolution.
struct PendingApproval {
    command: String,
    session_key: Option<String>,
    tx: oneshot::Sender<ApprovalDecision>,
}

/// Serializable summary of a pending approval request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingApprovalView {
    pub id: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
}

/// The approval manager handles approval flow for exec commands.
pub struct ApprovalManager {
    pub mode: ApprovalMode,
    pub security_level: SecurityLevel,
    pub allowlist: Vec<String>,
    pub timeout: Duration,
    pending: Arc<RwLock<std::collections::HashMap<String, PendingApproval>>>,
    approved_commands: Arc<RwLock<HashSet<String>>>,
}

impl Default for ApprovalManager {
    fn default() -> Self {
        Self {
            mode: ApprovalMode::OnMiss,
            security_level: SecurityLevel::Allowlist,
            allowlist: Vec::new(),
            timeout: Duration::from_secs(120),
            pending: Arc::new(RwLock::new(std::collections::HashMap::new())),
            approved_commands: Arc::new(RwLock::new(HashSet::new())),
        }
    }
}

impl ApprovalManager {
    /// Decide whether a command needs approval.
    /// Returns Ok(()) if the command can proceed, Err if denied.
    pub async fn check_command(&self, command: &str) -> Result<ApprovalAction> {
        // Safety floor: dangerous patterns are blocked unless explicitly
        // allowlisted. In OnMiss/Always mode we escalate to NeedsApproval so a
        // human can gate. In Off mode there is no human approver to wait on,
        // so the only safe outcome is to deny — otherwise the agent would hang
        // on `NeedsApproval` forever in headless deployments (moltis-org/moltis#654).
        if let Some(desc) = check_dangerous(command) {
            if !matches_allowlist(command, &self.allowlist) {
                if self.mode == ApprovalMode::Off {
                    warn!(
                        command,
                        pattern = %desc,
                        "dangerous command denied in approval_mode=off",
                    );
                    return Err(Error::message(format!(
                        "exec denied: dangerous command pattern '{desc}' (approval_mode=off): \
                         {command}"
                    )));
                }
                warn!(command, pattern = %desc, "dangerous command detected, forcing approval");
                return Ok(ApprovalAction::NeedsApproval);
            }
            debug!(command, pattern = %desc, "dangerous command allowed by explicit allowlist");
        }

        // Safety floor layer 2: catch dangerous env-var prefixes that the
        // anchored regex missed (e.g. chained assignments like
        // `FOO=bar LD_PRELOAD=/evil.so cat`). Same deny/escalate logic as
        // the regex layer above (moltis-org/moltis#814).
        if !command.trim().is_empty() && extract_first_bin(command).is_none() {
            if !matches_allowlist(command, &self.allowlist) {
                if self.mode == ApprovalMode::Off {
                    warn!(
                        command,
                        "dangerous env-var prefix denied in approval_mode=off",
                    );
                    return Err(Error::message(format!(
                        "exec denied: dangerous env-var prefix (approval_mode=off): {command}"
                    )));
                }
                warn!(
                    command,
                    "dangerous env-var prefix detected, forcing approval"
                );
                return Ok(ApprovalAction::NeedsApproval);
            }
            debug!(
                command,
                "dangerous env-var prefix allowed by explicit allowlist"
            );
        }

        match self.security_level {
            SecurityLevel::Deny => {
                return Err(Error::message("exec denied: security level is 'deny'"));
            },
            SecurityLevel::Full => return Ok(ApprovalAction::Proceed),
            SecurityLevel::Allowlist => {},
        }

        match self.mode {
            ApprovalMode::Off => {
                // With an empty allowlist, Off mode is unrestricted (preserves
                // historical behavior for deployments that never configured a list).
                // With a non-empty allowlist, the list is authoritative: the user
                // explicitly asked for enforcement, and there is no human to prompt
                // in headless deployments — non-matches must be denied, not silently
                // proceeded (moltis-org/moltis#654).
                if self.allowlist.is_empty() {
                    return Ok(ApprovalAction::Proceed);
                }
                if matches_allowlist(command, &self.allowlist) {
                    return Ok(ApprovalAction::Proceed);
                }
                if is_safe_command(command) {
                    // Safe bins bypass the explicit allowlist so operators don't
                    // have to enumerate common read-only utilities. Emit a warn
                    // so strict-posture operators can detect the gap at runtime
                    // (they can `grep safe-bin` their logs to audit, or file a
                    // follow-up for an opt-in strict mode that gates safe bins).
                    warn!(
                        command,
                        "exec safe-bin bypassed non-empty allowlist in approval_mode=off",
                    );
                    return Ok(ApprovalAction::Proceed);
                }
                Err(Error::message(format!(
                    "exec denied: command not in allowlist (approval_mode=off): {command}"
                )))
            },
            ApprovalMode::Always => Ok(ApprovalAction::NeedsApproval),
            ApprovalMode::OnMiss => {
                // Check safe bins.
                if is_safe_command(command) {
                    return Ok(ApprovalAction::Proceed);
                }
                // Check custom allowlist.
                if matches_allowlist(command, &self.allowlist) {
                    return Ok(ApprovalAction::Proceed);
                }
                // Check previously approved.
                if self.approved_commands.read().await.contains(command) {
                    return Ok(ApprovalAction::Proceed);
                }
                Ok(ApprovalAction::NeedsApproval)
            },
        }
    }

    /// Register a pending approval request. Returns an ID and a receiver for the decision.
    pub async fn create_request(
        &self,
        command: &str,
        session_key: Option<&str>,
    ) -> (String, oneshot::Receiver<ApprovalDecision>) {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending
            .write()
            .await
            .insert(id.clone(), PendingApproval {
                command: command.to_string(),
                session_key: session_key.map(str::to_string),
                tx,
            });
        debug!(id = %id, command, session_key, "approval request created");
        (id, rx)
    }

    /// Resolve a pending approval request.
    pub async fn resolve(&self, id: &str, decision: ApprovalDecision, command: Option<&str>) {
        if let Some(pending) = self.pending.write().await.remove(id) {
            if decision == ApprovalDecision::Approved
                && let Some(cmd) = command
            {
                self.approved_commands.write().await.insert(cmd.to_string());
            }
            let _ = pending.tx.send(decision);
            debug!(id, "approval resolved");
        } else {
            warn!(id, "approval resolve: no pending request");
        }
    }

    /// Return the IDs of all pending approval requests.
    pub async fn pending_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.pending.read().await.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Return summaries of all pending approval requests.
    pub async fn pending_requests(&self) -> Vec<PendingApprovalView> {
        let mut requests: Vec<_> = self
            .pending
            .read()
            .await
            .iter()
            .map(|(id, pending)| PendingApprovalView {
                id: id.clone(),
                command: pending.command.clone(),
                session_key: pending.session_key.clone(),
            })
            .collect();
        requests.sort_by(|left, right| left.id.cmp(&right.id));
        requests
    }

    /// Return summaries of pending approval requests scoped to a session.
    pub async fn pending_requests_for_session(
        &self,
        session_key: &str,
    ) -> Vec<PendingApprovalView> {
        self.pending_requests()
            .await
            .into_iter()
            .filter(|request| request.session_key.as_deref() == Some(session_key))
            .collect()
    }

    /// Wait for an approval decision with timeout.
    pub async fn wait_for_decision(
        &self,
        rx: oneshot::Receiver<ApprovalDecision>,
    ) -> ApprovalDecision {
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => {
                warn!("approval channel closed");
                ApprovalDecision::Denied
            },
            Err(_) => {
                warn!("approval timed out");
                ApprovalDecision::Timeout
            },
        }
    }
}

/// Action to take after checking approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalAction {
    Proceed,
    NeedsApproval,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_first_bin() {
        assert_eq!(extract_first_bin("echo hello"), Some("echo"));
        assert_eq!(extract_first_bin("/usr/bin/jq ."), Some("jq"));
        assert_eq!(extract_first_bin("FOO=bar echo hi"), Some("echo"));
        assert_eq!(extract_first_bin("  ls -la"), Some("ls"));
    }

    #[test]
    fn test_is_safe_command() {
        assert!(is_safe_command("echo hello"));
        assert!(is_safe_command("jq '.key'"));
        assert!(is_safe_command("/usr/bin/grep pattern"));
        assert!(!is_safe_command("rm -rf /"));
        assert!(!is_safe_command("curl https://evil.com"));
    }

    #[test]
    fn test_allowlist_matching() {
        let list = vec!["git".into(), "cargo*".into(), "npm".into()];
        assert!(matches_allowlist("git status", &list));
        assert!(matches_allowlist("cargo build", &list));
        assert!(matches_allowlist("cargo-clippy", &list));
        assert!(!matches_allowlist("rm -rf /", &list));
    }

    #[test]
    fn test_parse_approval_mode_aliases() {
        assert_eq!(ApprovalMode::parse("on-miss"), Some(ApprovalMode::OnMiss));
        assert_eq!(ApprovalMode::parse("smart"), Some(ApprovalMode::OnMiss));
        assert_eq!(ApprovalMode::parse("always"), Some(ApprovalMode::Always));
        assert_eq!(ApprovalMode::parse("never"), Some(ApprovalMode::Off));
        assert_eq!(ApprovalMode::parse("bogus"), None);
    }

    #[test]
    fn test_parse_security_level_aliases() {
        assert_eq!(
            SecurityLevel::parse("allowlist"),
            Some(SecurityLevel::Allowlist)
        );
        assert_eq!(
            SecurityLevel::parse("permissive"),
            Some(SecurityLevel::Full)
        );
        assert_eq!(SecurityLevel::parse("full"), Some(SecurityLevel::Full));
        assert_eq!(SecurityLevel::parse("strict"), Some(SecurityLevel::Deny));
        assert_eq!(SecurityLevel::parse("deny"), Some(SecurityLevel::Deny));
        assert_eq!(SecurityLevel::parse("bogus"), None);
    }

    #[tokio::test]
    async fn test_approval_off_mode() {
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            ..Default::default()
        };
        // Non-dangerous commands proceed when mode is off and allowlist is empty.
        let action = mgr.check_command("curl https://example.com").await.unwrap();
        assert_eq!(action, ApprovalAction::Proceed);
    }

    #[tokio::test]
    async fn test_approval_off_with_allowlist_match() {
        // Regression test for moltis-org/moltis#654: non-empty allowlist must be
        // enforced even when approval_mode is off (headless deployments).
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            allowlist: vec!["git *".into()],
            ..Default::default()
        };
        let action = mgr.check_command("git status").await.unwrap();
        assert_eq!(action, ApprovalAction::Proceed);
    }

    #[tokio::test]
    async fn test_approval_off_with_allowlist_miss_denies() {
        // Regression test for moltis-org/moltis#654: commands outside the
        // configured allowlist must be denied in Off mode, not silently proceeded.
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            allowlist: vec!["git *".into()],
            ..Default::default()
        };
        let err = mgr
            .check_command("curl https://evil.example.com")
            .await
            .expect_err("expected denial for non-allowlisted command in off mode");
        assert!(
            err.to_string().contains("not in allowlist"),
            "unexpected error message: {err}"
        );
    }

    #[tokio::test]
    async fn test_approval_off_with_allowlist_safe_bin() {
        // Safe bins are still allowed in Off mode so operators don't have to
        // enumerate them in every allowlist.
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            allowlist: vec!["git *".into()],
            ..Default::default()
        };
        let action = mgr.check_command("echo hi").await.unwrap();
        assert_eq!(action, ApprovalAction::Proceed);
    }

    #[tokio::test]
    async fn test_approval_off_empty_allowlist_unrestricted() {
        // Explicit contract lock: Off mode with an empty allowlist preserves
        // historical unrestricted semantics.
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            allowlist: Vec::new(),
            ..Default::default()
        };
        let action = mgr
            .check_command(r#"python3 -c "print('hi')""#)
            .await
            .unwrap();
        assert_eq!(action, ApprovalAction::Proceed);
    }

    #[tokio::test]
    async fn test_approval_off_full_security_bypasses_allowlist() {
        // SecurityLevel::Full short-circuits before the mode match, so even an
        // explicit allowlist has no effect.
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            security_level: SecurityLevel::Full,
            allowlist: vec!["git *".into()],
            ..Default::default()
        };
        let action = mgr.check_command("curl https://example.com").await.unwrap();
        assert_eq!(action, ApprovalAction::Proceed);
    }

    #[tokio::test]
    async fn test_approval_always_mode() {
        let mgr = ApprovalManager {
            mode: ApprovalMode::Always,
            ..Default::default()
        };
        let action = mgr.check_command("echo hi").await.unwrap();
        assert_eq!(action, ApprovalAction::NeedsApproval);
    }

    #[tokio::test]
    async fn test_approval_on_miss_safe() {
        let mgr = ApprovalManager::default();
        let action = mgr.check_command("echo hi").await.unwrap();
        assert_eq!(action, ApprovalAction::Proceed);
    }

    #[tokio::test]
    async fn test_approval_on_miss_unsafe() {
        let mgr = ApprovalManager::default();
        let action = mgr.check_command("rm -rf /").await.unwrap();
        assert_eq!(action, ApprovalAction::NeedsApproval);
    }

    #[tokio::test]
    async fn test_deny_security_level() {
        let mgr = ApprovalManager {
            security_level: SecurityLevel::Deny,
            ..Default::default()
        };
        assert!(mgr.check_command("echo hi").await.is_err());
    }

    // --- Dangerous pattern detection ---

    #[test]
    fn test_dangerous_rm_rf_root() {
        assert_eq!(
            check_dangerous("rm -rf /"),
            Some("rm -r on filesystem root")
        );
        assert_eq!(
            check_dangerous("rm -rf /*"),
            Some("rm -r on filesystem root")
        );
        assert_eq!(check_dangerous("rm -r /"), Some("rm -r on filesystem root"));
    }

    #[test]
    fn test_dangerous_rm_rf_home() {
        assert_eq!(check_dangerous("rm -rf ~"), Some("rm -r on home directory"));
        assert_eq!(
            check_dangerous("rm -rf $HOME"),
            Some("rm -r on home directory")
        );
    }

    #[test]
    fn test_dangerous_git_reset_hard() {
        assert_eq!(
            check_dangerous("git reset --hard"),
            Some("git reset --hard")
        );
        assert_eq!(
            check_dangerous("git reset --hard HEAD~1"),
            Some("git reset --hard")
        );
    }

    #[test]
    fn test_dangerous_git_force_push() {
        assert_eq!(
            check_dangerous("git push --force origin main"),
            Some("git force push")
        );
        assert_eq!(
            check_dangerous("git push -f origin main"),
            Some("git force push")
        );
        assert_eq!(
            check_dangerous("git push --force-with-lease origin main"),
            Some("git force push")
        );
    }

    #[test]
    fn test_dangerous_drop_table() {
        assert_eq!(
            check_dangerous(r#"psql -c "DROP TABLE users""#),
            Some("DROP TABLE/DATABASE")
        );
        assert_eq!(
            check_dangerous("DROP DATABASE production"),
            Some("DROP TABLE/DATABASE")
        );
    }

    #[test]
    fn test_dangerous_mkfs() {
        assert_eq!(
            check_dangerous("mkfs.ext4 /dev/sda1"),
            Some("make filesystem")
        );
    }

    #[test]
    fn test_dangerous_docker_prune() {
        assert_eq!(
            check_dangerous("docker system prune"),
            Some("docker system prune")
        );
        assert_eq!(
            check_dangerous("docker system prune -a --volumes"),
            Some("docker system prune")
        );
    }

    #[test]
    fn test_dangerous_truncate() {
        assert_eq!(check_dangerous("TRUNCATE TABLE sessions"), Some("TRUNCATE"));
    }

    #[test]
    fn test_dangerous_terraform_destroy() {
        assert_eq!(
            check_dangerous("terraform destroy -auto-approve"),
            Some("terraform destroy")
        );
    }

    #[test]
    fn test_dangerous_git_clean_force() {
        assert_eq!(
            check_dangerous("git clean -fd"),
            Some("git clean with force")
        );
    }

    #[test]
    fn test_dangerous_git_stash_drop() {
        assert_eq!(
            check_dangerous("git stash drop"),
            Some("git stash drop/clear")
        );
        assert_eq!(
            check_dangerous("git stash clear"),
            Some("git stash drop/clear")
        );
    }

    #[test]
    fn test_safe_commands_not_flagged() {
        assert!(check_dangerous("git status").is_none());
        assert!(check_dangerous("ls -la").is_none());
        assert!(check_dangerous("cargo build").is_none());
        assert!(check_dangerous("echo hello").is_none());
        assert!(check_dangerous("git push origin main").is_none());
        assert!(check_dangerous("rm file.txt").is_none());
        assert!(check_dangerous("docker ps").is_none());
    }

    #[tokio::test]
    async fn test_dangerous_overridden_by_allowlist() {
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            allowlist: vec!["rm*".into()],
            ..Default::default()
        };
        let action = mgr.check_command("rm -rf /").await.unwrap();
        assert_eq!(action, ApprovalAction::Proceed);
    }

    #[tokio::test]
    async fn test_dangerous_denied_when_mode_off() {
        // In Off mode dangerous commands must be denied (not NeedsApproval),
        // otherwise headless agents hang waiting for an approver that never
        // arrives (moltis-org/moltis#654).
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            ..Default::default()
        };
        let err = mgr
            .check_command("rm -rf /")
            .await
            .expect_err("expected denial for dangerous command in off mode");
        assert!(
            err.to_string().contains("dangerous command pattern"),
            "unexpected error message: {err}"
        );
    }

    #[tokio::test]
    async fn test_dangerous_denied_when_mode_off_full_security() {
        // Full security level does not change the safety floor: dangerous
        // commands are still denied in Off mode.
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            security_level: SecurityLevel::Full,
            ..Default::default()
        };
        let err = mgr
            .check_command("git reset --hard")
            .await
            .expect_err("expected denial for dangerous command in off+full");
        assert!(
            err.to_string().contains("dangerous command pattern"),
            "unexpected error message: {err}"
        );
    }

    #[tokio::test]
    async fn test_dangerous_forces_approval_when_full() {
        let mgr = ApprovalManager {
            security_level: SecurityLevel::Full,
            ..Default::default()
        };
        let action = mgr.check_command("git reset --hard").await.unwrap();
        assert_eq!(action, ApprovalAction::NeedsApproval);
    }

    #[tokio::test]
    async fn test_pending_requests_for_session_filters_other_sessions() {
        let mgr = ApprovalManager::default();
        let _ = mgr.create_request("echo one", Some("session:a")).await;
        let _ = mgr.create_request("echo two", Some("session:b")).await;
        let _ = mgr.create_request("echo three", Some("session:a")).await;

        let pending = mgr.pending_requests_for_session("session:a").await;
        assert_eq!(pending.len(), 2);
        assert!(
            pending
                .iter()
                .all(|request| request.session_key.as_deref() == Some("session:a"))
        );
    }

    // --- Env-var prefix injection (moltis-org/moltis#814) ---

    // Layer 1: check_dangerous regex hits

    #[test]
    fn test_dangerous_ld_preload() {
        assert_eq!(
            check_dangerous("LD_PRELOAD=/evil.so cat /etc/passwd"),
            Some("dangerous dynamic linker env var"),
        );
    }

    #[test]
    fn test_dangerous_ld_library_path() {
        assert_eq!(
            check_dangerous("LD_LIBRARY_PATH=/tmp cat /file"),
            Some("dangerous dynamic linker env var"),
        );
    }

    #[test]
    fn test_dangerous_ld_audit() {
        assert_eq!(
            check_dangerous("LD_AUDIT=/evil.so ls"),
            Some("dangerous dynamic linker env var"),
        );
    }

    #[test]
    fn test_dangerous_dyld_insert_libraries() {
        assert_eq!(
            check_dangerous("DYLD_INSERT_LIBRARIES=/evil.dylib cat /etc/passwd"),
            Some("dangerous macOS dynamic linker env var"),
        );
    }

    #[test]
    fn test_dangerous_dyld_library_path() {
        assert_eq!(
            check_dangerous("DYLD_LIBRARY_PATH=/tmp ls"),
            Some("dangerous macOS dynamic linker env var"),
        );
    }

    #[test]
    fn test_dangerous_path_override() {
        assert_eq!(
            check_dangerous("PATH=/tmp:$PATH cat /etc/passwd"),
            Some("PATH override"),
        );
    }

    #[test]
    fn test_dangerous_pythonpath() {
        assert_eq!(
            check_dangerous("PYTHONPATH=/evil python3 -c 'import os'"),
            Some("dangerous language runtime env var"),
        );
    }

    #[test]
    fn test_dangerous_node_options() {
        assert_eq!(
            check_dangerous("NODE_OPTIONS='--require=/evil.js' node app.js"),
            Some("dangerous language runtime env var"),
        );
    }

    #[test]
    fn test_dangerous_java_tool_options() {
        assert_eq!(
            check_dangerous("JAVA_TOOL_OPTIONS=-javaagent:/evil.jar java Main"),
            Some("dangerous language runtime env var"),
        );
    }

    #[test]
    fn test_dangerous_java_options_variants() {
        assert_eq!(
            check_dangerous("_JAVA_OPTIONS=-javaagent:/evil.jar java Main"),
            Some("dangerous language runtime env var"),
        );
        assert_eq!(
            check_dangerous("JDK_JAVA_OPTIONS=-javaagent:/evil.jar java Main"),
            Some("dangerous language runtime env var"),
        );
    }

    #[test]
    fn test_dangerous_perl5opt() {
        assert_eq!(
            check_dangerous("PERL5OPT=-M/evil perl -e1"),
            Some("dangerous language runtime env var"),
        );
    }

    #[test]
    fn test_dangerous_rubyopt() {
        assert_eq!(
            check_dangerous("RUBYOPT=-r/evil ruby -e1"),
            Some("dangerous language runtime env var"),
        );
    }

    #[test]
    fn test_dangerous_bash_env() {
        assert_eq!(
            check_dangerous("BASH_ENV=/evil.sh bash -c 'echo hi'"),
            Some("dangerous shell startup env var"),
        );
    }

    #[test]
    fn test_dangerous_env_var_in_subshell_not_caught_by_regex() {
        // Anchored regex patterns intentionally do NOT match env vars inside
        // quoted subshell arguments. This is safe because sh/bash are not safe
        // bins and require approval via the mode/allowlist path.
        assert!(check_dangerous(r#"sh -c "LD_PRELOAD=/evil.so cat /etc/passwd""#).is_none());
    }

    #[test]
    fn test_dangerous_env_var_after_separator() {
        // Patterns still fire after command separators.
        assert_eq!(
            check_dangerous("echo hi; LD_PRELOAD=/evil.so cat /file"),
            Some("dangerous dynamic linker env var"),
        );
        assert_eq!(
            check_dangerous("true && PATH=/evil:$PATH cmd"),
            Some("PATH override"),
        );
    }

    #[test]
    fn test_dangerous_env_var_case_insensitive() {
        assert_eq!(
            check_dangerous("ld_preload=/evil.so cat /file"),
            Some("dangerous dynamic linker env var"),
        );
    }

    #[test]
    fn test_benign_env_var_not_flagged() {
        // Variables whose names are not in the dangerous list.
        assert!(check_dangerous("FOO=bar echo hi").is_none());
        assert!(check_dangerous("RUST_LOG=debug cargo test").is_none());
        assert!(check_dangerous("MY_LD_PRELOAD_FLAG=1 echo hi").is_none());
        // LD_DEBUG is diagnostic only (no code injection) — not flagged.
        assert!(check_dangerous("LD_DEBUG=bindings ./myprogram").is_none());
        // Bare ENV is too noisy (ENV=test, ENV=production) — not flagged.
        assert!(check_dangerous("ENV=test rake test").is_none());
        assert!(check_dangerous("ENV=production ./server").is_none());
    }

    #[test]
    fn test_no_false_positive_on_grep_sed_arguments() {
        // Regression test: env var names inside grep/sed/awk arguments must
        // NOT trigger the regex. The (?:^|[;&|]\s*) anchor prevents this.
        assert!(check_dangerous("grep 'PATH=' ~/.bashrc").is_none());
        assert!(check_dangerous(r#"grep "PATH=" .env"#).is_none());
        assert!(check_dangerous("sed -n '/PATH=/p' .env").is_none());
        assert!(check_dangerous("awk -F'PATH=' '{print $2}' file").is_none());
        assert!(check_dangerous("grep 'LD_PRELOAD=' config.txt").is_none());
        assert!(check_dangerous("grep 'NODE_OPTIONS=' .env").is_none());
        // Unquoted grep with empty value — also benign.
        assert!(check_dangerous("grep PATH= file").is_none());
        // Unquoted grep with value — also benign (the `PATH=` is an argument,
        // not a shell assignment).
        assert!(check_dangerous("grep PATH=/usr/bin .env").is_none());
        assert!(check_dangerous("grep LD_PRELOAD=/path config.txt").is_none());
    }

    // Layer 2: extract_first_bin semantic check

    #[test]
    fn test_extract_first_bin_dangerous_prefix_returns_none() {
        assert_eq!(extract_first_bin("LD_PRELOAD=/evil.so cat /file"), None);
        assert_eq!(extract_first_bin("DYLD_INSERT_LIBRARIES=/e.dylib ls"), None);
        assert_eq!(extract_first_bin("PATH=/tmp:$PATH cat /etc/passwd"), None);
        assert_eq!(extract_first_bin("NODE_OPTIONS=--evil node app.js"), None);
        assert_eq!(extract_first_bin("BASH_ENV=/evil.sh bash -c hi"), None);
    }

    #[test]
    fn test_extract_first_bin_dangerous_case_insensitive() {
        assert_eq!(extract_first_bin("ld_preload=/evil.so cat /file"), None);
        assert_eq!(extract_first_bin("Ld_Preload=/evil.so cat /file"), None);
    }

    #[test]
    fn test_extract_first_bin_benign_prefix_still_works() {
        assert_eq!(extract_first_bin("FOO=bar echo hi"), Some("echo"));
        assert_eq!(
            extract_first_bin("RUST_LOG=debug cargo test"),
            Some("cargo"),
        );
        assert_eq!(extract_first_bin("CC=gcc CXX=g++ cmake .."), Some("cmake"),);
    }

    #[test]
    fn test_extract_first_bin_non_dangerous_ld_prefix() {
        // MY_LD_PRELOAD_FLAG contains "LD_PRELOAD" as substring but the key
        // itself is not a match.
        assert_eq!(
            extract_first_bin("MY_LD_PRELOAD_FLAG=1 echo hi"),
            Some("echo"),
        );
    }

    #[test]
    fn test_extract_first_bin_quoted_token_limitation() {
        // Quoted env-var assignments bypass Layer 2 because split_once('=')
        // sees key `"LD_PRELOAD` (with leading `"`). The anchored regex
        // (Layer 1) also does not match here because `"` is not whitespace.
        // In practice, shells interpret `"LD_PRELOAD=/evil.so"` as a command
        // name, not an env-var assignment, so this is not an exploitable
        // vector. And sh/bash subshell wrappers require approval separately.
        assert_eq!(
            extract_first_bin(r#""LD_PRELOAD=/evil.so" cat /file"#),
            Some("cat"),
        );
        assert!(check_dangerous(r#""LD_PRELOAD=/evil.so" cat /file"#).is_none());
    }

    #[test]
    fn test_extract_first_bin_ld_debug_and_env_are_benign() {
        // LD_DEBUG and ENV were intentionally excluded from DANGEROUS_ENV_VARS.
        assert_eq!(
            extract_first_bin("LD_DEBUG=bindings ./myprogram"),
            Some("myprogram"),
        );
        assert_eq!(extract_first_bin("ENV=test rake test"), Some("rake"));
    }

    // is_safe_command rejects dangerous-prefixed safe commands

    #[test]
    fn test_is_safe_command_rejects_dangerous_prefix() {
        assert!(!is_safe_command("LD_PRELOAD=/evil.so cat /etc/passwd"));
        assert!(!is_safe_command("PATH=/tmp echo hi"));
        assert!(!is_safe_command("DYLD_INSERT_LIBRARIES=/e.dylib ls"));
    }

    #[test]
    fn test_is_safe_command_allows_benign_prefix() {
        assert!(is_safe_command("FOO=bar echo hi"));
        assert!(is_safe_command("RUST_LOG=debug cat file.txt"));
    }

    // matches_allowlist rejects dangerous-prefixed allowlisted commands

    #[test]
    fn test_allowlist_rejects_dangerous_prefix() {
        let list = vec!["cat".into(), "ls".into()];
        assert!(!matches_allowlist("LD_PRELOAD=/evil.so cat /file", &list));
        assert!(!matches_allowlist("PATH=/tmp ls", &list));
    }

    #[test]
    fn test_allowlist_wildcard_still_matches_dangerous_prefix() {
        // Wildcard `*` overrides everything — existing documented behavior.
        let list = vec!["*".into()];
        assert!(matches_allowlist("LD_PRELOAD=/evil.so cat /file", &list));
    }

    #[test]
    fn test_allowlist_prefix_no_bypass_via_chained_assignment() {
        // Regression: `command.starts_with(prefix)` must not match when
        // extract_first_bin returned None due to a dangerous env var.
        let list = vec!["MY_APP*".into()];
        assert!(!matches_allowlist(
            "MY_APP=1 LD_PRELOAD=/evil.so cat /file",
            &list,
        ));
        // But benign chained assignments still match.
        assert!(matches_allowlist("MY_APP run --flag", &list));
    }

    // check_command integration tests

    #[tokio::test]
    async fn test_env_injection_needs_approval_on_miss() {
        let mgr = ApprovalManager::default();
        let action = mgr
            .check_command("LD_PRELOAD=/evil.so cat /etc/passwd")
            .await
            .unwrap();
        assert_eq!(action, ApprovalAction::NeedsApproval);
    }

    #[tokio::test]
    async fn test_env_injection_denied_in_off_mode() {
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            ..Default::default()
        };
        let err = mgr
            .check_command("LD_PRELOAD=/evil.so cat /etc/passwd")
            .await
            .expect_err("expected denial for env injection in off mode");
        assert!(
            err.to_string().contains("dangerous"),
            "unexpected error message: {err}"
        );
    }

    #[tokio::test]
    async fn test_chained_env_injection_denied_in_off_empty_allowlist() {
        // Regression: chained assignments must be caught by Layer 2 safety
        // floor even when Off+empty-allowlist short-circuits before mode check.
        let mgr = ApprovalManager {
            mode: ApprovalMode::Off,
            ..Default::default()
        };
        let err = mgr
            .check_command("FOO=bar LD_PRELOAD=/evil.so cat /etc/passwd")
            .await
            .expect_err("expected denial for chained env injection in off mode");
        assert!(
            err.to_string().contains("dangerous env-var prefix"),
            "unexpected error message: {err}"
        );
    }

    #[tokio::test]
    async fn test_chained_env_injection_needs_approval_on_miss() {
        let mgr = ApprovalManager::default();
        let action = mgr
            .check_command("FOO=bar LD_PRELOAD=/evil.so cat /etc/passwd")
            .await
            .unwrap();
        assert_eq!(action, ApprovalAction::NeedsApproval);
    }

    #[tokio::test]
    async fn test_env_injection_needs_approval_always_mode() {
        let mgr = ApprovalManager {
            mode: ApprovalMode::Always,
            ..Default::default()
        };
        let action = mgr
            .check_command("DYLD_INSERT_LIBRARIES=/evil.dylib ls")
            .await
            .unwrap();
        assert_eq!(action, ApprovalAction::NeedsApproval);
    }

    #[tokio::test]
    async fn test_benign_prefix_proceeds_on_miss() {
        let mgr = ApprovalManager::default();
        let action = mgr.check_command("RUST_LOG=debug echo hi").await.unwrap();
        assert_eq!(action, ApprovalAction::Proceed);
    }

    #[tokio::test]
    async fn test_env_injection_in_subshell_needs_approval() {
        // Regex doesn't match inside quotes, but sh is not a safe bin so
        // the mode check (OnMiss) still requires approval.
        let mgr = ApprovalManager::default();
        let action = mgr
            .check_command(r#"sh -c "LD_PRELOAD=/evil.so cat /etc/passwd""#)
            .await
            .unwrap();
        assert_eq!(action, ApprovalAction::NeedsApproval);
    }

    #[tokio::test]
    async fn test_env_injection_with_explicit_allowlist_wildcard() {
        // Wildcard allowlist still overrides dangerous env var patterns.
        let mgr = ApprovalManager {
            allowlist: vec!["*".into()],
            ..Default::default()
        };
        let action = mgr
            .check_command("LD_PRELOAD=/evil.so cat /etc/passwd")
            .await
            .unwrap();
        // Dangerous pattern matched, but allowlist wildcard overrides → falls
        // through to the mode check where safe-bin check fails (extract_first_bin
        // returns None) → NeedsApproval in default OnMiss mode.
        // Actually: check_dangerous fires, matches_allowlist("*") returns true,
        // so the dangerous block is skipped. Then security_level=Allowlist,
        // mode=OnMiss. is_safe_command returns false (extract_first_bin → None).
        // matches_allowlist("*") returns true → Proceed.
        assert_eq!(action, ApprovalAction::Proceed);
    }
}
