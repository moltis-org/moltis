//! The session list shown in the sidebar. Pure state, no UI.

use moltis_protocol::{
    chat_event::{ChatEvent, ChatEventKind},
    client_api::{SessionEntry, SessionEvent, SessionEventKind},
};

/// Session key the daemon uses when none is given.
pub const MAIN_SESSION: &str = "main";

#[derive(Debug, Default)]
pub struct Sessions {
    entries: Vec<SessionEntry>,
    selected: Option<String>,
}

impl Sessions {
    #[must_use]
    pub fn entries(&self) -> &[SessionEntry] {
        &self.entries
    }

    #[must_use]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    #[must_use]
    pub fn selected_entry(&self) -> Option<&SessionEntry> {
        let key = self.selected.as_deref()?;
        self.entries.iter().find(|entry| entry.key == key)
    }

    /// Replace the list from `sessions.list`. Archived sessions are hidden;
    /// newest first. Keeps the selection when it still exists, else selects
    /// the newest session (or `main` on an empty daemon).
    pub fn replace(&mut self, mut entries: Vec<SessionEntry>) {
        entries.retain(|entry| !entry.archived);
        entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.key.cmp(&b.key)));
        self.entries = entries;
        let keep = self
            .selected
            .as_deref()
            .is_some_and(|key| self.entries.iter().any(|entry| entry.key == key));
        if !keep {
            self.selected = Some(
                self.entries
                    .first()
                    .map_or_else(|| MAIN_SESSION.to_string(), |entry| entry.key.clone()),
            );
        }
    }

    /// Select `key`; returns whether the selection changed.
    pub fn select(&mut self, key: &str) -> bool {
        if self.selected.as_deref() == Some(key) {
            return false;
        }
        self.selected = Some(key.to_string());
        true
    }

    /// Add a session that exists only locally until its first message.
    pub fn insert_draft(&mut self, key: &str) {
        if !self.entries.iter().any(|entry| entry.key == key) {
            self.entries.insert(0, SessionEntry {
                key: key.into(),
                ..SessionEntry::default()
            });
        }
        self.select(key);
    }

    /// Apply a `session` event; returns whether anything changed.
    pub fn apply_session_event(&mut self, event: &SessionEvent) -> bool {
        if event.kind != SessionEventKind::Patched {
            return false;
        }
        let Some(entry) = self.entries.iter_mut().find(|e| e.key == event.session_key) else {
            return false;
        };
        if let Some(version) = event.version
            && version < entry.version
        {
            return false;
        }
        if let Some(label) = &event.label {
            entry.label = Some(label.clone());
        }
        if let Some(version) = event.version {
            entry.version = version;
        }
        true
    }

    /// Track activity from a `chat` event; returns whether the row changed,
    /// and whether the session is unknown (so the list should be refetched).
    pub fn apply_chat_event(&mut self, event: &ChatEvent) -> (bool, bool) {
        let Some(entry) = self.entries.iter_mut().find(|e| e.key == event.session_key) else {
            return (false, !event.session_key.is_empty());
        };
        let replying = match &event.kind {
            ChatEventKind::UserMessage { text } => {
                if entry.preview.is_none() {
                    entry.preview = Some(text.chars().take(80).collect());
                }
                entry.message_count += 1;
                true
            },
            ChatEventKind::Final(_) => {
                entry.message_count += 1;
                false
            },
            ChatEventKind::Error { .. } | ChatEventKind::Aborted { .. } => false,
            ChatEventKind::SessionCleared => {
                entry.message_count = 0;
                entry.preview = None;
                false
            },
            _ => entry.replying,
        };
        let changed = entry.replying != replying;
        entry.replying = replying;
        let changed = changed
            || matches!(
                event.kind,
                ChatEventKind::UserMessage { .. }
                    | ChatEventKind::Final(_)
                    | ChatEventKind::SessionCleared
            );
        (changed, false)
    }
}

/// Which sidebar group a session falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recency {
    Today,
    Earlier,
}

impl Recency {
    /// `updated_at_ms` against the start of the local day. A draft has no
    /// timestamp yet and was just created, so it counts as today.
    #[must_use]
    pub fn of(updated_at_ms: u64, today_start_ms: u64) -> Self {
        if updated_at_ms == 0 || updated_at_ms >= today_start_ms {
            Self::Today
        } else {
            Self::Earlier
        }
    }
}

/// Milliseconds since the epoch at the last local midnight (UTC when the
/// local offset is unknown).
#[must_use]
pub fn today_start_ms() -> u64 {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let midnight = now.replace_time(time::Time::MIDNIGHT);
    u64::try_from(midnight.unix_timestamp_nanos() / 1_000_000).unwrap_or_default()
}

/// A fresh session key, like the web UI's `session:<uuid>`.
#[must_use]
pub fn new_session_key() -> String {
    format!("session:{}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use {super::*, moltis_protocol::chat_event::ChatFinal};

    #[test]
    fn recency_splits_at_midnight() {
        assert_eq!(Recency::of(1_000, 1_000), Recency::Today);
        assert_eq!(Recency::of(999, 1_000), Recency::Earlier);
        assert_eq!(Recency::of(0, 1_000), Recency::Today);
        let start = today_start_ms();
        assert!(start > 0);
        assert_eq!(start % 60_000, 0);
    }

    fn entry(key: &str, updated_at: u64) -> SessionEntry {
        SessionEntry {
            key: key.into(),
            updated_at,
            ..SessionEntry::default()
        }
    }

    fn chat(key: &str, kind: ChatEventKind) -> ChatEvent {
        ChatEvent {
            session_key: key.into(),
            run_id: None,
            seq: None,
            kind,
        }
    }

    #[test]
    fn replace_sorts_hides_archived_and_selects() {
        let mut sessions = Sessions::default();
        let mut archived = entry("old", 99);
        archived.archived = true;
        sessions.replace(vec![entry("a", 1), entry("b", 3), archived]);
        let keys: Vec<_> = sessions.entries().iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["b", "a"]);
        assert_eq!(sessions.selected(), Some("b"));

        sessions.select("a");
        sessions.replace(vec![entry("a", 1), entry("c", 9)]);
        assert_eq!(sessions.selected(), Some("a"), "selection is kept");

        sessions.replace(vec![entry("c", 9)]);
        assert_eq!(sessions.selected(), Some("c"), "vanished selection moves");

        sessions.replace(vec![]);
        assert_eq!(sessions.selected(), Some(MAIN_SESSION));
    }

    #[test]
    fn select_reports_change() {
        let mut sessions = Sessions::default();
        assert!(sessions.select("x"));
        assert!(!sessions.select("x"));
        assert!(sessions.selected_entry().is_none());
    }

    #[test]
    fn drafts_are_inserted_once_and_selected() {
        let mut sessions = Sessions::default();
        sessions.replace(vec![entry("a", 1)]);
        sessions.insert_draft("session:new");
        sessions.insert_draft("session:new");
        assert_eq!(sessions.entries().len(), 2);
        assert_eq!(sessions.entries()[0].key, "session:new");
        assert_eq!(sessions.selected(), Some("session:new"));
    }

    #[test]
    fn session_patch_updates_label_unless_stale() {
        let mut sessions = Sessions::default();
        let mut e = entry("a", 1);
        e.version = 5;
        sessions.replace(vec![e]);
        let patch = |version| SessionEvent {
            kind: SessionEventKind::Patched,
            session_key: "a".into(),
            label: Some(format!("v{version}")),
            version: Some(version),
        };
        assert!(!sessions.apply_session_event(&patch(4)));
        assert!(sessions.apply_session_event(&patch(6)));
        assert_eq!(sessions.entries()[0].title(), "v6");
        assert!(!sessions.apply_session_event(&SessionEvent {
            kind: SessionEventKind::Other,
            ..patch(7)
        }));
        assert!(!sessions.apply_session_event(&SessionEvent {
            session_key: "missing".into(),
            ..patch(8)
        }));
    }

    #[test]
    fn chat_events_track_replying_and_preview() {
        let mut sessions = Sessions::default();
        sessions.replace(vec![entry("a", 1)]);
        let (changed, unknown) =
            sessions.apply_chat_event(&chat("a", ChatEventKind::UserMessage {
                text: "hello".into(),
            }));
        assert!(changed && !unknown);
        let row = &sessions.entries()[0];
        assert!(row.replying);
        assert_eq!(row.preview.as_deref(), Some("hello"));

        let (changed, _) =
            sessions.apply_chat_event(&chat("a", ChatEventKind::Delta { text: "x".into() }));
        assert!(!changed, "deltas do not touch the row");
        assert!(sessions.entries()[0].replying);

        let (changed, _) =
            sessions.apply_chat_event(&chat("a", ChatEventKind::Final(ChatFinal::default())));
        assert!(changed);
        assert!(!sessions.entries()[0].replying);
        assert_eq!(sessions.entries()[0].message_count, 2);

        let (_, unknown) = sessions.apply_chat_event(&chat("zzz", ChatEventKind::Thinking));
        assert!(unknown);

        sessions.apply_chat_event(&chat("a", ChatEventKind::SessionCleared));
        assert_eq!(sessions.entries()[0].message_count, 0);
        assert!(sessions.entries()[0].preview.is_none());
    }

    #[test]
    fn new_keys_are_unique() {
        let a = new_session_key();
        assert!(a.starts_with("session:"));
        assert_ne!(a, new_session_key());
    }
}
