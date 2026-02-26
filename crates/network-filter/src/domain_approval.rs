//! Domain approval manager for trusted-network mode.
//!
//! Manages per-session domain allow/deny decisions with a config-based
//! allowlist, session-approved domains, and an interactive approval flow.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use {
    async_trait::async_trait,
    tokio::sync::{RwLock, oneshot},
    tracing::{debug, instrument, warn},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram};

use crate::{ApprovalSource, DomainDecision, DomainFilter, DomainPattern, FilterAction};

struct PendingDomainRequest {
    tx: oneshot::Sender<DomainDecision>,
    domain: String,
    session: String,
}

/// Manages domain allow/deny decisions for trusted network mode.
///
/// Follows the same pattern as `ApprovalManager` in `approval.rs`:
/// pending requests are stored with a UUID key and resolved via oneshot channels.
pub struct DomainApprovalManager {
    config_allowlist: Vec<DomainPattern>,
    session_allowlist: RwLock<HashMap<String, HashSet<String>>>,
    pending: RwLock<HashMap<String, PendingDomainRequest>>,
    timeout: Duration,
}

impl DomainApprovalManager {
    pub fn new(allowed_domains: &[String], timeout: Duration) -> Self {
        let config_allowlist = allowed_domains
            .iter()
            .map(|s| DomainPattern::parse(s))
            .collect();
        Self {
            config_allowlist,
            session_allowlist: RwLock::new(HashMap::new()),
            pending: RwLock::new(HashMap::new()),
            timeout,
        }
    }

    /// Check a domain against the config allowlist and session-approved domains.
    #[instrument(skip(self), fields(session = %session, domain = %domain))]
    pub async fn check_domain(&self, session: &str, domain: &str) -> FilterAction {
        self.check_domain_with_source(session, domain).await.0
    }

    /// Like [`check_domain`] but also returns the approval source when allowed.
    #[instrument(skip(self), fields(session = %session, domain = %domain))]
    pub async fn check_domain_with_source(
        &self,
        session: &str,
        domain: &str,
    ) -> (FilterAction, Option<ApprovalSource>) {
        // Empty config allowlist means allow-all (audit-only mode).
        if self.config_allowlist.is_empty() {
            #[cfg(feature = "metrics")]
            counter!("domain_checks_total", "result" => "allowed", "source" => "config")
                .increment(1);
            return (FilterAction::Allow, Some(ApprovalSource::Config));
        }

        // Config allowlist.
        for pattern in &self.config_allowlist {
            if pattern.matches(domain) {
                #[cfg(feature = "metrics")]
                counter!("domain_checks_total", "result" => "allowed", "source" => "config")
                    .increment(1);
                return (FilterAction::Allow, Some(ApprovalSource::Config));
            }
        }
        // Session-approved domains.
        if let Some(domains) = self.session_allowlist.read().await.get(session)
            && domains.contains(&domain.to_lowercase())
        {
            #[cfg(feature = "metrics")]
            counter!("domain_checks_total", "result" => "allowed", "source" => "session")
                .increment(1);
            return (FilterAction::Allow, Some(ApprovalSource::Session));
        }
        #[cfg(feature = "metrics")]
        counter!("domain_checks_total", "result" => "needs_approval", "source" => "none")
            .increment(1);
        (FilterAction::NeedsApproval, None)
    }

    /// Create a pending approval request. Returns a UUID and a receiver for the decision.
    #[instrument(skip(self), fields(session = %session, domain = %domain))]
    pub async fn create_request(
        &self,
        session: &str,
        domain: &str,
    ) -> (String, oneshot::Receiver<DomainDecision>) {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending
            .write()
            .await
            .insert(id.clone(), PendingDomainRequest {
                tx,
                domain: domain.to_string(),
                session: session.to_string(),
            });
        debug!(id = %id, "domain approval request created");
        #[cfg(feature = "metrics")]
        counter!("domain_approval_requests_total").increment(1);
        (id, rx)
    }

    /// Resolve a pending domain approval request.
    /// If approved, the domain is added to the session allowlist for future requests.
    #[instrument(skip(self), fields(id = %id, decision = ?decision))]
    pub async fn resolve(&self, id: &str, decision: DomainDecision) {
        if let Some(pending) = self.pending.write().await.remove(id) {
            #[cfg(feature = "metrics")]
            {
                let decision_label = match decision {
                    DomainDecision::Approved => "approved",
                    DomainDecision::Denied => "denied",
                    DomainDecision::Timeout => "timeout",
                };
                counter!("domain_approval_decisions_total", "decision" => decision_label)
                    .increment(1);
            }

            if decision == DomainDecision::Approved {
                self.session_allowlist
                    .write()
                    .await
                    .entry(pending.session.clone())
                    .or_default()
                    .insert(pending.domain.to_lowercase());
            }
            let _ = pending.tx.send(decision);
            debug!("domain approval resolved");
        } else {
            warn!("domain approval resolve: no pending request");
        }
    }

    /// Wait for a domain approval decision with timeout.
    #[instrument(skip(self, rx))]
    pub async fn wait_for_decision(&self, rx: oneshot::Receiver<DomainDecision>) -> DomainDecision {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();

        let result = match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => {
                warn!("domain approval channel closed");
                DomainDecision::Denied
            },
            Err(_) => {
                warn!("domain approval timed out");
                DomainDecision::Timeout
            },
        };

        #[cfg(feature = "metrics")]
        histogram!("domain_approval_wait_duration_seconds").record(start.elapsed().as_secs_f64());

        result
    }

    /// Return all pending request IDs with their domains and sessions.
    pub async fn pending_requests(&self) -> Vec<(String, String, String)> {
        self.pending
            .read()
            .await
            .iter()
            .map(|(id, req)| (id.clone(), req.domain.clone(), req.session.clone()))
            .collect()
    }

    /// Add a domain to the session allowlist without going through the approval flow.
    pub async fn add_trusted_domain(&self, session: &str, domain: &str) {
        self.session_allowlist
            .write()
            .await
            .entry(session.to_string())
            .or_default()
            .insert(domain.to_lowercase());
    }

    /// Remove a domain from the session allowlist.
    pub async fn remove_trusted_domain(&self, session: &str, domain: &str) {
        if let Some(domains) = self.session_allowlist.write().await.get_mut(session) {
            domains.remove(&domain.to_lowercase());
        }
    }

    /// List all trusted domains for a session (config + session-approved).
    pub async fn list_trusted_domains(&self, session: &str) -> Vec<String> {
        let mut result: Vec<String> = self
            .config_allowlist
            .iter()
            .map(|p| match p {
                DomainPattern::Exact(d) => d.clone(),
                DomainPattern::WildcardSubdomain(d) => format!("*.{d}"),
                DomainPattern::Wildcard => "*".to_string(),
            })
            .collect();

        if let Some(domains) = self.session_allowlist.read().await.get(session) {
            for d in domains {
                if !result.contains(d) {
                    result.push(d.clone());
                }
            }
        }
        result
    }

    /// Access the configured timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

#[async_trait]
impl DomainFilter for DomainApprovalManager {
    async fn check(&self, session: &str, domain: &str) -> FilterAction {
        self.check_domain(session, domain).await
    }
}

#[async_trait]
impl DomainFilter for Arc<DomainApprovalManager> {
    async fn check(&self, session: &str, domain: &str) -> FilterAction {
        self.check_domain(session, domain).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_check_domain_config_allowlist() {
        let mgr = DomainApprovalManager::new(&["github.com".into()], Duration::from_secs(60));
        assert_eq!(
            mgr.check_domain("sess1", "github.com").await,
            FilterAction::Allow
        );
        assert_eq!(
            mgr.check_domain("sess1", "evil.com").await,
            FilterAction::NeedsApproval
        );
    }

    #[tokio::test]
    async fn test_empty_allowlist_allows_all_domains() {
        // Empty config allowlist = audit-only mode: all domains allowed.
        let mgr = DomainApprovalManager::new(&[], Duration::from_secs(60));
        assert_eq!(
            mgr.check_domain("sess1", "anything.com").await,
            FilterAction::Allow
        );
        assert_eq!(
            mgr.check_domain("sess1", "evil.org").await,
            FilterAction::Allow
        );
    }

    #[tokio::test]
    async fn test_check_domain_session_allowlist() {
        // Use a non-empty config allowlist so empty-allowlist-allows-all doesn't kick in.
        let mgr = DomainApprovalManager::new(&["github.com".into()], Duration::from_secs(60));
        mgr.add_trusted_domain("sess1", "example.com").await;
        assert_eq!(
            mgr.check_domain("sess1", "example.com").await,
            FilterAction::Allow
        );
        // Different session should not have access to session-approved domains.
        assert_eq!(
            mgr.check_domain("sess2", "example.com").await,
            FilterAction::NeedsApproval
        );
    }

    #[tokio::test]
    async fn test_create_and_resolve_approved() {
        let mgr = DomainApprovalManager::new(&[], Duration::from_secs(60));
        let (id, rx) = mgr.create_request("sess1", "example.com").await;

        // Resolve as approved.
        mgr.resolve(&id, DomainDecision::Approved).await;
        let decision = rx.await.unwrap();
        assert_eq!(decision, DomainDecision::Approved);

        // Domain should now be in session allowlist.
        assert_eq!(
            mgr.check_domain("sess1", "example.com").await,
            FilterAction::Allow
        );
    }

    #[tokio::test]
    async fn test_create_and_resolve_denied() {
        // Use a non-empty config allowlist so empty-allowlist-allows-all doesn't kick in.
        let mgr = DomainApprovalManager::new(&["github.com".into()], Duration::from_secs(60));
        let (id, rx) = mgr.create_request("sess1", "evil.com").await;

        mgr.resolve(&id, DomainDecision::Denied).await;
        let decision = rx.await.unwrap();
        assert_eq!(decision, DomainDecision::Denied);

        // Domain should NOT be in session allowlist.
        assert_eq!(
            mgr.check_domain("sess1", "evil.com").await,
            FilterAction::NeedsApproval
        );
    }

    #[tokio::test]
    async fn test_wait_for_decision_timeout() {
        let mgr = DomainApprovalManager::new(&[], Duration::from_millis(50));
        let (_id, rx) = mgr.create_request("sess1", "slow.com").await;

        // Don't resolve — should timeout.
        let decision = mgr.wait_for_decision(rx).await;
        assert_eq!(decision, DomainDecision::Timeout);
    }

    #[tokio::test]
    async fn test_list_trusted_domains() {
        let mgr = DomainApprovalManager::new(
            &["github.com".into(), "*.npmjs.org".into()],
            Duration::from_secs(60),
        );
        mgr.add_trusted_domain("sess1", "example.com").await;

        let domains = mgr.list_trusted_domains("sess1").await;
        assert!(domains.contains(&"github.com".to_string()));
        assert!(domains.contains(&"*.npmjs.org".to_string()));
        assert!(domains.contains(&"example.com".to_string()));
    }

    #[tokio::test]
    async fn test_remove_trusted_domain() {
        // Use a non-empty config allowlist so empty-allowlist-allows-all doesn't kick in.
        let mgr = DomainApprovalManager::new(&["github.com".into()], Duration::from_secs(60));
        mgr.add_trusted_domain("sess1", "example.com").await;
        assert_eq!(
            mgr.check_domain("sess1", "example.com").await,
            FilterAction::Allow
        );

        mgr.remove_trusted_domain("sess1", "example.com").await;
        assert_eq!(
            mgr.check_domain("sess1", "example.com").await,
            FilterAction::NeedsApproval
        );
    }

    #[tokio::test]
    async fn test_pending_requests() {
        let mgr = DomainApprovalManager::new(&[], Duration::from_secs(60));
        let (id1, _rx1) = mgr.create_request("sess1", "a.com").await;
        let (id2, _rx2) = mgr.create_request("sess1", "b.com").await;

        let pending = mgr.pending_requests().await;
        assert_eq!(pending.len(), 2);

        let ids: HashSet<String> = pending.iter().map(|(id, ..)| id.clone()).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }
}
