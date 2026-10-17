//! Reconcile asynchronous history snapshots with the live event stream.

use std::collections::{HashMap, HashSet};

use moltis_protocol::{
    chat_event::{ChatEvent, ChatEventKind},
    message::PersistedMessage,
};

use crate::transcript::{Change, Transcript};

#[derive(Default)]
struct Entry {
    transcript: Transcript,
    fresh: bool,
    request: Option<u64>,
    pending: Vec<ChatEvent>,
    coverage: Coverage,
}

/// Identifiers already represented by the last snapshot. Keep these after
/// loading too: a persisted final can arrive in history before its event.
#[derive(Default)]
struct Coverage {
    users: HashSet<String>,
    completed: HashSet<String>,
    tools: HashSet<String>,
}

impl Coverage {
    fn from_history(history: &[PersistedMessage]) -> Self {
        let mut coverage = Self::default();
        for message in history {
            match message {
                PersistedMessage::User {
                    run_id: Some(id), ..
                } => {
                    coverage.users.insert(id.clone());
                },
                PersistedMessage::Assistant {
                    run_id: Some(id),
                    tool_calls,
                    ..
                } if tool_calls.as_ref().is_none_or(Vec::is_empty) => {
                    coverage.completed.insert(id.clone());
                },
                PersistedMessage::ToolResult { tool_call_id, .. } => {
                    coverage.tools.insert(tool_call_id.clone());
                },
                _ => {},
            }
        }
        coverage
    }

    fn contains_tool(&self, event: &ChatEvent) -> bool {
        match &event.kind {
            ChatEventKind::ToolCallStart { tool_call_id, .. }
            | ChatEventKind::ToolCallEnd { tool_call_id, .. } => self.tools.contains(tool_call_id),
            _ => false,
        }
    }

    fn apply(&mut self, transcript: &mut Transcript, event: &ChatEvent) -> Change {
        if matches!(event.kind, ChatEventKind::SessionCleared) {
            *self = Self::default();
        }
        if event
            .run_id
            .as_ref()
            .is_some_and(|id| self.completed.contains(id))
        {
            return Change::None;
        }
        if matches!(event.kind, ChatEventKind::UserMessage { .. })
            && event
                .run_id
                .as_ref()
                .is_some_and(|id| self.users.contains(id))
        {
            return transcript.apply(&ChatEventKind::Thinking);
        }
        if self.contains_tool(event) {
            return Change::None;
        }
        transcript.apply(&event.kind)
    }
}

#[derive(Default)]
pub(super) struct TranscriptCache {
    entries: HashMap<String, Entry>,
    next_request: u64,
}

impl TranscriptCache {
    pub fn get(&self, key: &str) -> Option<&Transcript> {
        self.entries.get(key).map(|entry| &entry.transcript)
    }

    pub fn insert_draft(&mut self, key: &str) {
        self.entries.insert(key.into(), Entry {
            fresh: true,
            ..Entry::default()
        });
    }

    pub fn invalidate(&mut self) {
        for entry in self.entries.values_mut() {
            entry.fresh = false;
            entry.request = None;
            entry.pending.clear();
        }
    }

    pub fn needs_load(&self, key: &str) -> bool {
        self.entries
            .get(key)
            .is_none_or(|entry| !entry.fresh && entry.request.is_none())
    }

    pub fn begin_load(&mut self, key: &str) -> Option<u64> {
        if !self.needs_load(key) {
            return None;
        }
        self.next_request += 1;
        self.entries.entry(key.into()).or_default().request = Some(self.next_request);
        Some(self.next_request)
    }

    pub fn fail_load(&mut self, key: &str, request: u64) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if entry.request != Some(request) {
            return false;
        }
        entry.request = None;
        true
    }

    pub fn finish_load(&mut self, key: &str, request: u64, history: &[PersistedMessage]) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if entry.request != Some(request) {
            return false;
        }
        let mut coverage = Coverage::from_history(history);
        let mut transcript = Transcript::from_history(history);
        // If history includes a tool result, earlier streaming fragments of
        // that run belong before that tool, not after the snapshot. Deduplicate
        // by identifiers, never by text (users can send identical messages).
        let mut covered_through = HashMap::new();
        for (index, event) in entry.pending.iter().enumerate() {
            if coverage.contains_tool(event)
                && let Some(run) = &event.run_id
            {
                covered_through.insert(run, index);
            }
        }
        for (index, event) in entry.pending.iter().enumerate() {
            if event
                .run_id
                .as_ref()
                .and_then(|id| covered_through.get(id))
                .is_some_and(|covered| index <= *covered)
            {
                continue;
            }
            coverage.apply(&mut transcript, event);
        }
        entry.transcript = transcript;
        entry.coverage = coverage;
        entry.pending.clear();
        entry.fresh = true;
        entry.request = None;
        true
    }

    pub fn apply(&mut self, event: &ChatEvent) -> Change {
        // Remember events even before a session's first history request.
        let entry = self.entries.entry(event.session_key.clone()).or_default();
        if !entry.fresh {
            entry.pending.push(event.clone());
        }
        entry.coverage.apply(&mut entry.transcript, event)
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
