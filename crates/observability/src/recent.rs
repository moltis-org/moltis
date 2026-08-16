//! The most recent trace per session.
//!
//! A channel reply is delivered by code far from the agent loop that produced
//! it — the runner returns text, and a separate dispatcher sends it. Feedback
//! attribution needs the trace id at that send point, and threading it through
//! every outbound signature would touch every channel for the benefit of one
//! feature.
//!
//! Sessions process turns serially, so "the current trace for this session" is
//! unambiguous at the moment its reply goes out. That is what this records.
//!
//! Bounded, because an install with many channel conversations would otherwise
//! accumulate one entry per session forever. Eviction is by insertion order,
//! and losing an entry costs a feedback attribution, never a message.

use std::{
    collections::HashMap,
    sync::{OnceLock, RwLock},
};

use crate::model::TraceId;

/// How many sessions keep a remembered trace.
const CAPACITY: usize = 4_096;

/// Insertion-ordered bounded map of session key to trace id.
#[derive(Default)]
struct RecentTraces {
    by_session: HashMap<String, TraceId>,
    order: Vec<String>,
}

impl RecentTraces {
    fn remember(&mut self, session_key: String, trace_id: TraceId) {
        if self
            .by_session
            .insert(session_key.clone(), trace_id)
            .is_none()
        {
            self.order.push(session_key);
        }
        while self.order.len() > CAPACITY {
            let oldest = self.order.remove(0);
            self.by_session.remove(&oldest);
        }
    }

    fn get(&self, session_key: &str) -> Option<TraceId> {
        self.by_session.get(session_key).cloned()
    }
}

fn registry() -> &'static RwLock<RecentTraces> {
    static REGISTRY: OnceLock<RwLock<RecentTraces>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(RecentTraces::default()))
}

/// Record `trace_id` as the session's current trace.
///
/// No-op for an empty session key: those would all collapse onto one entry and
/// cross-attribute unrelated turns.
pub fn remember_trace(session_key: &str, trace_id: &TraceId) {
    if session_key.is_empty() {
        return;
    }
    let mut guard = registry()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.remember(session_key.to_string(), trace_id.clone());
}

/// The session's most recent trace, if one is still remembered.
#[must_use]
pub fn recent_trace(session_key: &str) -> Option<TraceId> {
    let guard = registry()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.get(session_key)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_remembered_trace_is_retrievable() {
        let mut recent = RecentTraces::default();
        recent.remember("s1".into(), TraceId("t1".into()));

        assert_eq!(recent.get("s1"), Some(TraceId("t1".into())));
    }

    #[test]
    fn a_newer_turn_replaces_the_previous_trace_for_the_session() {
        let mut recent = RecentTraces::default();
        recent.remember("s1".into(), TraceId("t1".into()));
        recent.remember("s1".into(), TraceId("t2".into()));

        assert_eq!(recent.get("s1"), Some(TraceId("t2".into())));
    }

    #[test]
    fn sessions_do_not_share_traces() {
        let mut recent = RecentTraces::default();
        recent.remember("s1".into(), TraceId("t1".into()));
        recent.remember("s2".into(), TraceId("t2".into()));

        assert_eq!(recent.get("s1"), Some(TraceId("t1".into())));
        assert_eq!(recent.get("s2"), Some(TraceId("t2".into())));
    }

    #[test]
    fn an_unknown_session_has_no_trace() {
        assert_eq!(RecentTraces::default().get("nope"), None);
    }

    #[test]
    fn the_map_stays_bounded_under_many_sessions() {
        // A busy install has one session per chat; without a bound this grows
        // for the life of the process.
        let mut recent = RecentTraces::default();
        for i in 0..(CAPACITY + 100) {
            recent.remember(format!("s{i}"), TraceId(format!("t{i}")));
        }

        assert_eq!(recent.by_session.len(), CAPACITY);
        assert_eq!(recent.order.len(), CAPACITY);
        // The oldest were evicted, the newest retained.
        assert!(recent.get("s0").is_none());
        assert!(recent.get(&format!("s{}", CAPACITY + 99)).is_some());
    }

    #[test]
    fn repeated_turns_on_one_session_do_not_grow_the_map() {
        let mut recent = RecentTraces::default();
        for i in 0..1_000 {
            recent.remember("s1".into(), TraceId(format!("t{i}")));
        }

        assert_eq!(recent.by_session.len(), 1);
        assert_eq!(recent.order.len(), 1, "insertion order list leaked entries");
    }

    #[test]
    fn empty_session_keys_are_not_remembered() {
        // Otherwise every unattributed turn collapses onto one entry and
        // feedback lands on whichever turn happened to be last.
        remember_trace("", &TraceId("t1".into()));
        assert_eq!(recent_trace(""), None);
    }

    #[test]
    fn the_global_registry_round_trips() {
        remember_trace("global-session", &TraceId("global-trace".into()));
        assert_eq!(
            recent_trace("global-session"),
            Some(TraceId("global-trace".into()))
        );
    }
}
