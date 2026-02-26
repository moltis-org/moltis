//! Core types for trusted-network domain filtering and audit logging.
//!
//! These types are shared between the proxy (emits entries), the gateway
//! (buffer, persistence, UI streaming), and the macOS Swift bridge.

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    time::OffsetDateTime,
};

/// The default port the proxy listens on inside the trusted network.
pub const DEFAULT_PROXY_PORT: u16 = 18791;

// ── Network policy ──────────────────────────────────────────────────────────

/// Network policy for sandboxed containers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicy {
    /// No network access (`--network=none`).
    Blocked,
    /// Isolated network with HTTP CONNECT proxy filtering by domain allowlist.
    #[default]
    Trusted,
    /// Unrestricted network, bypasses proxy entirely (no audit logging).
    Bypass,
}

// ── Audit entry ─────────────────────────────────────────────────────────────

/// A single audited network request or connection through the proxy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkAuditEntry {
    /// When the request started.
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    /// Peer session identifier (typically peer socket address).
    pub session: String,
    /// Target domain name.
    pub domain: String,
    /// Target port.
    pub port: u16,
    /// Whether this was a CONNECT tunnel or a plain HTTP forward.
    pub protocol: NetworkProtocol,
    /// Filter decision for this request.
    pub action: FilterOutcome,
    /// HTTP method (GET, POST, etc.) — `None` for raw CONNECT tunnels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Full URL for HTTP forwards — `None` for CONNECT tunnels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// HTTP response status — `None` for CONNECT tunnels or when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Bytes transferred from client to upstream.
    pub bytes_sent: u64,
    /// Bytes transferred from upstream to client.
    pub bytes_received: u64,
    /// Wall-clock duration of the connection/request in milliseconds.
    pub duration_ms: u64,
    /// Error message if the request failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// How the domain was approved (config allowlist, session, user prompt).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_source: Option<ApprovalSource>,
}

/// The proxy protocol used for a connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProtocol {
    /// CONNECT tunnel (HTTPS, WebSocket, etc.).
    HttpConnect,
    /// Plain HTTP forwarding.
    HttpForward,
}

/// The outcome of the domain filter check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilterOutcome {
    /// Domain was on the allowlist — request went through.
    Allowed,
    /// Domain was denied — request blocked.
    Denied,
    /// User approved the domain via the approval prompt.
    ApprovedByUser,
    /// Approval timed out — request blocked.
    Timeout,
}

/// How a domain was approved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalSource {
    /// Matched a pattern in the config allowlist.
    Config,
    /// Already approved in the current session.
    Session,
    /// User approved via the interactive prompt.
    UserPrompt,
}

/// Type alias for the sender half of the audit channel.
///
/// The gateway creates the channel; the proxy uses this to emit entries.
pub type AuditSender = tokio::sync::mpsc::Sender<NetworkAuditEntry>;

impl std::fmt::Display for NetworkProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HttpConnect => write!(f, "CONNECT"),
            Self::HttpForward => write!(f, "HTTP"),
        }
    }
}

impl std::fmt::Display for FilterOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allowed => write!(f, "allowed"),
            Self::Denied => write!(f, "denied"),
            Self::ApprovedByUser => write!(f, "approved_by_user"),
            Self::Timeout => write!(f, "timeout"),
        }
    }
}

// ── Domain filtering ────────────────────────────────────────────────────────

/// Action returned by the domain filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterAction {
    Allow,
    Deny,
    NeedsApproval,
}

/// Trait for checking whether a domain should be allowed through the proxy.
#[async_trait]
pub trait DomainFilter: Send + Sync {
    async fn check(&self, session: &str, domain: &str) -> FilterAction;
}

/// A pattern for matching domain names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainPattern {
    /// Match exactly this domain (e.g. `github.com`).
    Exact(String),
    /// Match any subdomain (e.g. `*.github.com` matches `api.github.com` but not `github.com`).
    WildcardSubdomain(String),
    /// Match everything (`*`).
    Wildcard,
}

impl DomainPattern {
    /// Parse a pattern string into a `DomainPattern`.
    pub fn parse(s: &str) -> Self {
        let s = s.trim().to_lowercase();
        if s == "*" {
            return Self::Wildcard;
        }
        if let Some(suffix) = s.strip_prefix("*.") {
            return Self::WildcardSubdomain(suffix.to_string());
        }
        Self::Exact(s)
    }

    /// Check whether a domain matches this pattern.
    pub fn matches(&self, domain: &str) -> bool {
        let domain = domain.to_lowercase();
        match self {
            Self::Wildcard => true,
            Self::Exact(d) => domain == *d,
            Self::WildcardSubdomain(suffix) => {
                domain == *suffix || domain.ends_with(&format!(".{suffix}"))
            },
        }
    }
}

/// Outcome of a domain approval request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainDecision {
    Approved,
    Denied,
    Timeout,
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // -- NetworkAuditEntry serde ---

    #[test]
    fn serde_round_trip() {
        let entry = NetworkAuditEntry {
            timestamp: OffsetDateTime::now_utc(),
            session: "127.0.0.1:12345".into(),
            domain: "github.com".into(),
            port: 443,
            protocol: NetworkProtocol::HttpConnect,
            action: FilterOutcome::Allowed,
            method: None,
            url: None,
            status: None,
            bytes_sent: 1024,
            bytes_received: 4096,
            duration_ms: 320,
            error: None,
            approval_source: Some(ApprovalSource::Config),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: NetworkAuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.domain, "github.com");
        assert_eq!(back.protocol, NetworkProtocol::HttpConnect);
        assert_eq!(back.action, FilterOutcome::Allowed);
        assert_eq!(back.approval_source, Some(ApprovalSource::Config));
        assert_eq!(back.bytes_sent, 1024);
    }

    #[test]
    fn serde_http_forward_entry() {
        let entry = NetworkAuditEntry {
            timestamp: OffsetDateTime::now_utc(),
            session: "127.0.0.1:54321".into(),
            domain: "registry.npmjs.org".into(),
            port: 80,
            protocol: NetworkProtocol::HttpForward,
            action: FilterOutcome::ApprovedByUser,
            method: Some("GET".into()),
            url: Some("http://registry.npmjs.org/package".into()),
            status: Some(200),
            bytes_sent: 512,
            bytes_received: 8192,
            duration_ms: 150,
            error: None,
            approval_source: Some(ApprovalSource::UserPrompt),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: NetworkAuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.method, Some("GET".into()));
        assert_eq!(back.status, Some(200));
        assert_eq!(back.protocol, NetworkProtocol::HttpForward);
        assert_eq!(back.action, FilterOutcome::ApprovedByUser);
    }

    #[test]
    fn serde_denied_entry() {
        let entry = NetworkAuditEntry {
            timestamp: OffsetDateTime::now_utc(),
            session: "peer".into(),
            domain: "evil.com".into(),
            port: 443,
            protocol: NetworkProtocol::HttpConnect,
            action: FilterOutcome::Denied,
            method: None,
            url: None,
            status: None,
            bytes_sent: 0,
            bytes_received: 0,
            duration_ms: 5,
            error: None,
            approval_source: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        // Optional fields should be absent when None
        assert!(!json.contains("method"));
        assert!(!json.contains("url"));
        assert!(!json.contains("approval_source"));
    }

    #[test]
    fn protocol_display() {
        assert_eq!(NetworkProtocol::HttpConnect.to_string(), "CONNECT");
        assert_eq!(NetworkProtocol::HttpForward.to_string(), "HTTP");
    }

    #[test]
    fn filter_outcome_display() {
        assert_eq!(FilterOutcome::Allowed.to_string(), "allowed");
        assert_eq!(FilterOutcome::Denied.to_string(), "denied");
        assert_eq!(FilterOutcome::Timeout.to_string(), "timeout");
    }

    #[test]
    fn enum_serde_rename() {
        let json = serde_json::to_string(&NetworkProtocol::HttpConnect).unwrap();
        assert_eq!(json, r#""http_connect""#);
        let json = serde_json::to_string(&FilterOutcome::ApprovedByUser).unwrap();
        assert_eq!(json, r#""approved_by_user""#);
        let json = serde_json::to_string(&ApprovalSource::UserPrompt).unwrap();
        assert_eq!(json, r#""user_prompt""#);
    }

    // -- DomainPattern ---

    #[test]
    fn domain_pattern_exact() {
        let p = DomainPattern::parse("github.com");
        assert!(p.matches("github.com"));
        assert!(p.matches("GitHub.com"));
        assert!(!p.matches("api.github.com"));
        assert!(!p.matches("notgithub.com"));
    }

    #[test]
    fn domain_pattern_wildcard_subdomain() {
        let p = DomainPattern::parse("*.github.com");
        assert!(p.matches("api.github.com"));
        assert!(p.matches("raw.github.com"));
        assert!(p.matches("github.com"));
        assert!(!p.matches("notgithub.com"));
    }

    #[test]
    fn domain_pattern_wildcard() {
        let p = DomainPattern::parse("*");
        assert!(p.matches("anything.com"));
        assert!(p.matches("evil.org"));
    }

    #[test]
    fn domain_pattern_case_insensitive() {
        let p = DomainPattern::parse("GitHub.COM");
        assert!(p.matches("github.com"));
        assert!(p.matches("GITHUB.COM"));
    }

    // -- NetworkPolicy ---

    #[test]
    fn network_policy_default_is_trusted() {
        assert_eq!(NetworkPolicy::default(), NetworkPolicy::Trusted);
    }

    #[test]
    fn network_policy_serde() {
        let json = serde_json::to_string(&NetworkPolicy::Trusted).unwrap();
        assert_eq!(json, r#""trusted""#);
        let back: NetworkPolicy = serde_json::from_str(r#""blocked""#).unwrap();
        assert_eq!(back, NetworkPolicy::Blocked);
    }
}
