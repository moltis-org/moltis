//! Mapping between ACP session ids and Moltis session keys.
//!
//! The two are the same string. Moltis session keys created for ACP live under
//! a dedicated `acp:` namespace (see [`SessionKey::namespaced`]), which keeps
//! `moltis acp` runs from colliding with Web UI sessions while still letting a
//! client hand a `SessionId` straight back to `session/load`.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use agent_client_protocol as acp;

/// Prefix marking a Moltis session as owned by the ACP surface.
pub const ACP_SESSION_NAMESPACE: &str = "acp";

/// Maximum sessions one stdio connection may retain.
pub const MAX_SESSIONS: usize = 64;

/// Maximum turns or setup operations that may run concurrently per connection.
pub const MAX_CONCURRENT_OPERATIONS: usize = 4;

const MAX_SESSION_ID_SUFFIX_BYTES: usize = 128;

/// A Moltis session key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionKey(String);

impl SessionKey {
    /// Wraps an existing key verbatim.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Builds a key inside the ACP namespace.
    #[must_use]
    pub fn namespaced(id: impl AsRef<str>) -> Self {
        Self(format!("{ACP_SESSION_NAMESPACE}:{}", id.as_ref()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this key belongs to the ACP namespace.
    #[must_use]
    pub fn is_namespaced(&self) -> bool {
        self.0.split_once(':').is_some_and(|(prefix, rest)| {
            prefix == ACP_SESSION_NAMESPACE
                && !rest.is_empty()
                && rest.len() <= MAX_SESSION_ID_SUFFIX_BYTES
                && rest
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    }
}

impl std::fmt::Display for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<SessionKey> for acp::SessionId {
    fn from(key: SessionKey) -> Self {
        Self::from(key.0)
    }
}

impl From<&acp::SessionId> for SessionKey {
    fn from(id: &acp::SessionId) -> Self {
        Self(id.to_string())
    }
}

/// Tracks the sessions this connection has opened.
///
/// Lives on the `LocalSet` thread alongside the protocol handler, so it uses
/// `Rc`/`RefCell` rather than `Arc`/`Mutex` — there is no cross-thread sharing
/// to guard against, and a poisoned-lock path would be dead code.
#[derive(Clone, Debug, Default)]
pub struct SessionRegistry {
    inner: Rc<RefCell<RegistryState>>,
}

#[derive(Debug, Default)]
struct RegistryState {
    known: HashSet<SessionKey>,
    in_flight: HashSet<SessionKey>,
    /// Sessions whose in-flight turn has been cancelled but not yet observed.
    cancelled: HashMap<SessionKey, bool>,
    next_id: u64,
}

impl SessionRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates the next connection-local session identifier.
    ///
    /// Backends are free to ignore this and mint their own keys; it exists so a
    /// backend without its own id source stays collision-free within a run.
    pub fn next_local_id(&self) -> u64 {
        let mut state = self.inner.borrow_mut();
        state.next_id = state.next_id.saturating_add(1);
        state.next_id
    }

    pub fn insert(&self, key: SessionKey) -> acp::Result<()> {
        self.ensure_capacity_for(&key)?;
        let mut state = self.inner.borrow_mut();
        state.known.insert(key);
        Ok(())
    }

    pub fn ensure_capacity(&self) -> acp::Result<()> {
        if self.inner.borrow().known.len() < MAX_SESSIONS {
            return Ok(());
        }
        Err(acp::Error::invalid_params().data(format!(
            "connection cannot retain more than {MAX_SESSIONS} sessions"
        )))
    }

    /// Rejects setup work that would exceed the connection's session budget.
    pub fn ensure_capacity_for(&self, key: &SessionKey) -> acp::Result<()> {
        let state = self.inner.borrow();
        if state.known.contains(key) || state.known.len() < MAX_SESSIONS {
            return Ok(());
        }
        Err(acp::Error::invalid_params().data(format!(
            "connection cannot retain more than {MAX_SESSIONS} sessions"
        )))
    }

    #[must_use]
    pub fn contains(&self, key: &SessionKey) -> bool {
        self.inner.borrow().known.contains(key)
    }

    /// Resolves an incoming `SessionId`, rejecting ids this connection never
    /// handed out with `invalid_params` rather than panicking.
    pub fn resolve(&self, id: &acp::SessionId) -> acp::Result<SessionKey> {
        let key = SessionKey::from(id);
        if self.contains(&key) {
            return Ok(key);
        }
        Err(acp::Error::invalid_params().data(format!("unknown session id {id}")))
    }

    /// Marks a prompt active, rejecting overlapping turns for one session.
    pub fn begin_prompt(&self, key: &SessionKey) -> acp::Result<PromptGuard> {
        let mut state = self.inner.borrow_mut();
        if state.in_flight.len() >= MAX_CONCURRENT_OPERATIONS {
            return Err(acp::Error::invalid_params().data(format!(
                "connection cannot run more than {MAX_CONCURRENT_OPERATIONS} operations concurrently"
            )));
        }
        if !state.in_flight.insert(key.clone()) {
            return Err(acp::Error::invalid_params()
                .data(format!("session {key} already has an active prompt")));
        }
        Ok(PromptGuard {
            registry: self.clone(),
            key: key.clone(),
        })
    }

    /// Blocks session setup changes while another setup or prompt is active.
    pub fn begin_setup(&self, key: &SessionKey) -> acp::Result<PromptGuard> {
        let mut state = self.inner.borrow_mut();
        if state.in_flight.len() >= MAX_CONCURRENT_OPERATIONS {
            return Err(acp::Error::invalid_params().data(format!(
                "connection cannot run more than {MAX_CONCURRENT_OPERATIONS} operations concurrently"
            )));
        }
        if !state.in_flight.insert(key.clone()) {
            return Err(acp::Error::invalid_params()
                .data(format!("session {key} already has an active operation")));
        }
        Ok(PromptGuard {
            registry: self.clone(),
            key: key.clone(),
        })
    }

    /// Marks the session's in-flight turn as cancelled.
    pub fn mark_cancelled(&self, key: &SessionKey) {
        self.inner.borrow_mut().cancelled.insert(key.clone(), true);
    }

    /// Clears any cancellation flag, called when a turn starts.
    pub fn clear_cancelled(&self, key: &SessionKey) {
        self.inner.borrow_mut().cancelled.remove(key);
    }

    /// Takes the cancellation flag, called when a turn finishes.
    pub fn take_cancelled(&self, key: &SessionKey) -> bool {
        self.inner
            .borrow_mut()
            .cancelled
            .remove(key)
            .unwrap_or(false)
    }
}

/// Removes a session's active-prompt marker on every exit path.
pub struct PromptGuard {
    registry: SessionRegistry,
    key: SessionKey,
}

impl Drop for PromptGuard {
    fn drop(&mut self) {
        self.registry.inner.borrow_mut().in_flight.remove(&self.key);
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaced_keys_are_prefixed() {
        let key = SessionKey::namespaced("abc");
        assert_eq!(key.as_str(), "acp:abc");
        assert!(key.is_namespaced());
    }

    #[test]
    fn plain_keys_are_not_namespaced() {
        assert!(!SessionKey::new("web:abc").is_namespaced());
        assert!(!SessionKey::new("abc").is_namespaced());
        assert!(!SessionKey::new("acp:").is_namespaced());
        assert!(!SessionKey::new("acp:../escape").is_namespaced());
        assert!(!SessionKey::new("acp:line\nbreak").is_namespaced());
    }

    #[test]
    fn session_key_round_trips_through_acp_session_id() {
        let key = SessionKey::namespaced("round-trip");
        let id = acp::SessionId::from(key.clone());
        assert_eq!(SessionKey::from(&id), key);
    }

    #[test]
    fn unknown_session_id_is_invalid_params() {
        let registry = SessionRegistry::new();
        let error = registry
            .resolve(&acp::SessionId::from("acp:nope".to_string()))
            .expect_err("unknown session must be rejected");
        assert_eq!(error.code, acp::Error::invalid_params().code);
    }

    #[test]
    fn known_session_id_resolves() {
        let registry = SessionRegistry::new();
        let key = SessionKey::namespaced("known");
        registry.insert(key.clone()).expect("insert known session");
        let id = acp::SessionId::from(key.clone());
        assert_eq!(registry.resolve(&id).expect("known session"), key);
    }

    #[test]
    fn cancellation_flag_is_take_once() {
        let registry = SessionRegistry::new();
        let key = SessionKey::namespaced("cancel");
        registry.insert(key.clone()).expect("insert cancel session");
        assert!(!registry.take_cancelled(&key));
        registry.mark_cancelled(&key);
        assert!(registry.take_cancelled(&key));
        assert!(!registry.take_cancelled(&key));
    }

    #[test]
    fn local_ids_are_unique() {
        let registry = SessionRegistry::new();
        let first = registry.next_local_id();
        let second = registry.next_local_id();
        assert_ne!(first, second);
    }

    #[test]
    fn setup_and_prompt_operations_are_mutually_exclusive() {
        let registry = SessionRegistry::new();
        let key = SessionKey::namespaced("busy");
        registry.insert(key.clone()).expect("insert busy session");

        let setup = registry.begin_setup(&key).expect("first operation");
        assert!(registry.begin_prompt(&key).is_err());
        drop(setup);

        let prompt = registry.begin_prompt(&key).expect("operation released");
        assert!(registry.begin_setup(&key).is_err());
        drop(prompt);
        assert!(registry.begin_setup(&key).is_ok());
    }

    #[test]
    fn session_count_is_bounded() {
        let registry = SessionRegistry::new();
        for index in 0..MAX_SESSIONS {
            registry
                .insert(SessionKey::namespaced(format!("session-{index}")))
                .expect("session within limit");
        }
        assert!(
            registry
                .ensure_capacity_for(&SessionKey::namespaced("one-too-many"))
                .is_err()
        );
        assert!(registry.ensure_capacity().is_err());
    }

    #[test]
    fn concurrent_operations_are_bounded() {
        let registry = SessionRegistry::new();
        let guards = (0..MAX_CONCURRENT_OPERATIONS)
            .map(|index| {
                registry
                    .begin_setup(&SessionKey::namespaced(format!("setup-{index}")))
                    .expect("operation within limit")
            })
            .collect::<Vec<_>>();
        assert!(
            registry
                .begin_setup(&SessionKey::namespaced("one-too-many"))
                .is_err()
        );
        drop(guards);
        assert!(
            registry
                .begin_setup(&SessionKey::namespaced("after-release"))
                .is_ok()
        );
    }
}
