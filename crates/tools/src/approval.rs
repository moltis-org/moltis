use std::{collections::HashSet, sync::Arc, time::Duration};

use {
    anyhow::{Result, bail},
    serde::{Deserialize, Serialize},
    tokio::sync::{RwLock, oneshot},
    tracing::{debug, warn},
};

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

/// Extract the first command/binary from a shell command string.
fn extract_first_bin(command: &str) -> Option<&str> {
    let trimmed = command.trim();
    // Skip env var assignments at the start (e.g. `FOO=bar cmd`).
    let mut parts = trimmed.split_whitespace();
    for part in parts.by_ref() {
        if !part.contains('=') {
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
    let bin = extract_first_bin(command).unwrap_or("");
    for pattern in allowlist {
        if pattern == "*" {
            return true;
        }
        if pattern == bin {
            return true;
        }
        // Prefix match with wildcard.
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            if command.starts_with(prefix) || bin.starts_with(prefix) {
                return true;
            }
        }
    }
    false
}

/// Pending approval request waiting for gateway resolution.
struct PendingApproval {
    tx: oneshot::Sender<ApprovalDecision>,
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
        match self.security_level {
            SecurityLevel::Deny => bail!("exec denied: security level is 'deny'"),
            SecurityLevel::Full => return Ok(ApprovalAction::Proceed),
            SecurityLevel::Allowlist => {},
        }

        match self.mode {
            ApprovalMode::Off => Ok(ApprovalAction::Proceed),
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
    ) -> (String, oneshot::Receiver<ApprovalDecision>) {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending
            .write()
            .await
            .insert(id.clone(), PendingApproval { tx });
        debug!(id = %id, command, "approval request created");
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
        self.pending.read().await.keys().cloned().collect()
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
        let action = mgr.check_command("rm -rf /").await.unwrap();
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
}
