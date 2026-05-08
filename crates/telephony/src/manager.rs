//! Call manager — orchestrates call lifecycle and state tracking.

use {
    dashmap::DashMap,
    std::sync::Arc,
    time::OffsetDateTime,
    tokio::sync::RwLock,
    tracing::{debug, info, warn},
    uuid::Uuid,
};

use crate::{
    provider::TelephonyProvider,
    types::{
        CallDirection, CallEndReason, CallEvent, CallId, CallMode, CallRecord, CallState, Speaker,
        TranscriptEntry,
    },
};

/// Central call lifecycle manager.
///
/// Maintains active calls, maps provider call IDs to internal IDs,
/// and enforces max-duration timeouts.
pub struct CallManager {
    /// Active calls keyed by internal `CallId`.
    active_calls: Arc<DashMap<CallId, CallRecord>>,
    /// Provider call ID → internal `CallId`.
    provider_index: Arc<DashMap<String, CallId>>,
    /// Max-duration timeout handles.
    timeout_handles: DashMap<CallId, tokio::task::JoinHandle<()>>,
    /// The telephony provider backend.
    provider: Arc<RwLock<Box<dyn TelephonyProvider>>>,
    /// Default max call duration (seconds).
    max_duration_secs: u64,
}

impl CallManager {
    pub fn new(provider: Box<dyn TelephonyProvider>, max_duration_secs: u64) -> Self {
        Self {
            active_calls: Arc::new(DashMap::new()),
            provider_index: Arc::new(DashMap::new()),
            timeout_handles: DashMap::new(),
            provider: Arc::new(RwLock::new(provider)),
            max_duration_secs,
        }
    }

    /// Initiate an outbound call.
    pub async fn initiate(
        &self,
        from: &str,
        to: &str,
        mode: CallMode,
        message: Option<&str>,
        account_id: &str,
        status_callback_url: &str,
        answer_url: &str,
    ) -> anyhow::Result<CallId> {
        let call_id = Uuid::new_v4().to_string();

        let params = crate::provider::InitiateCallParams {
            from: from.to_string(),
            to: to.to_string(),
            mode,
            message: message.map(String::from),
            status_callback_url: status_callback_url.to_string(),
            answer_url: answer_url.to_string(),
        };

        let result = self.provider.read().await.initiate_call(params).await?;

        let record = CallRecord {
            call_id: call_id.clone(),
            provider_call_id: Some(result.provider_call_id.clone()),
            direction: CallDirection::Outbound,
            from: from.to_string(),
            to: to.to_string(),
            mode,
            state: CallState::Initiated,
            initial_message: message.map(String::from),
            session_key: None,
            account_id: account_id.to_string(),
            transcript: Vec::new(),
            started_at: OffsetDateTime::now_utc(),
            ended_at: None,
            end_reason: None,
        };

        self.active_calls.insert(call_id.clone(), record);
        self.provider_index
            .insert(result.provider_call_id, call_id.clone());

        self.start_timeout(&call_id);

        info!(call_id = %call_id, "outbound call initiated");
        Ok(call_id)
    }

    /// Register an inbound call (created when we receive the first webhook).
    pub fn register_inbound(
        &self,
        provider_call_id: &str,
        from: &str,
        to: &str,
        account_id: &str,
    ) -> CallId {
        let call_id = Uuid::new_v4().to_string();

        let record = CallRecord {
            call_id: call_id.clone(),
            provider_call_id: Some(provider_call_id.to_string()),
            direction: CallDirection::Inbound,
            from: from.to_string(),
            to: to.to_string(),
            mode: CallMode::Conversation,
            state: CallState::Answered,
            initial_message: None,
            session_key: None,
            account_id: account_id.to_string(),
            transcript: Vec::new(),
            started_at: OffsetDateTime::now_utc(),
            ended_at: None,
            end_reason: None,
        };

        self.active_calls.insert(call_id.clone(), record);
        self.provider_index
            .insert(provider_call_id.to_string(), call_id.clone());

        self.start_timeout(&call_id);

        info!(call_id = %call_id, from = %from, "inbound call registered");
        call_id
    }

    /// Process a normalized call event.
    pub fn handle_event(&self, event: &CallEvent) {
        let provider_id = event.provider_call_id();
        let Some(call_id) = self.provider_index.get(provider_id).map(|r| r.clone()) else {
            debug!(provider_call_id = %provider_id, "event for unknown call");
            return;
        };

        let Some(mut record) = self.active_calls.get_mut(&call_id) else {
            return;
        };

        match event {
            CallEvent::Initiated { .. } => {
                record.state = CallState::Initiated;
            },
            CallEvent::Ringing { .. } => {
                record.state = CallState::Ringing;
            },
            CallEvent::Answered { .. } => {
                record.state = CallState::Answered;
            },
            CallEvent::Speaking { .. } => {
                record.state = CallState::Speaking;
            },
            CallEvent::Speech { text, .. } => {
                record.state = CallState::Active;
                record.transcript.push(TranscriptEntry {
                    speaker: Speaker::User,
                    text: text.clone(),
                    timestamp: OffsetDateTime::now_utc(),
                });
            },
            CallEvent::Silence { .. } => {
                // Stay in current state; silence doesn't change it.
            },
            CallEvent::Dtmf { .. } => {
                record.state = CallState::Active;
            },
            CallEvent::Ended { reason, .. } => {
                let state = reason.to_state();
                record.state = state;
                record.ended_at = Some(OffsetDateTime::now_utc());
                record.end_reason = Some(*reason);
                info!(call_id = %call_id, reason = ?reason, "call ended");
            },
            CallEvent::Error { message, .. } => {
                record.state = CallState::Error;
                record.ended_at = Some(OffsetDateTime::now_utc());
                record.end_reason = Some(CallEndReason::Error);
                warn!(call_id = %call_id, error = %message, "call error");
            },
        }

        // Clean up terminated calls from the index (but keep the record).
        if record.state.is_terminal() {
            if let Some(pid) = &record.provider_call_id {
                self.provider_index.remove(pid);
            }
            drop(record);
            self.active_calls.remove(&call_id);
            self.cancel_timeout(&call_id);
        }
    }

    /// Hang up a call by internal ID.
    pub async fn hangup(&self, call_id: &str) -> anyhow::Result<()> {
        let record = self
            .active_calls
            .get(call_id)
            .ok_or_else(|| crate::Error::CallNotFound(call_id.to_string()))?;

        let provider_call_id = record
            .provider_call_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no provider call ID"))?;

        if record.state.is_terminal() {
            return Ok(());
        }
        drop(record);

        self.provider
            .read()
            .await
            .hangup_call(&provider_call_id)
            .await?;

        // Mark as bot hangup and clean up provider index.
        if let Some(mut rec) = self.active_calls.get_mut(call_id) {
            rec.state = CallState::HangupBot;
            rec.ended_at = Some(OffsetDateTime::now_utc());
            rec.end_reason = Some(CallEndReason::HangupBot);
        }
        self.provider_index.remove(&provider_call_id);
        self.active_calls.remove(call_id);
        self.cancel_timeout(call_id);
        Ok(())
    }

    /// Add a bot transcript entry.
    pub fn record_bot_speech(&self, call_id: &str, text: &str) {
        if let Some(mut record) = self.active_calls.get_mut(call_id) {
            record.transcript.push(TranscriptEntry {
                speaker: Speaker::Bot,
                text: text.to_string(),
                timestamp: OffsetDateTime::now_utc(),
            });
        }
    }

    /// Get a snapshot of a call record.
    #[must_use]
    pub fn get_call(&self, call_id: &str) -> Option<CallRecord> {
        self.active_calls.get(call_id).map(|r| r.clone())
    }

    /// Resolve internal call ID from provider call ID.
    #[must_use]
    pub fn resolve_call_id(&self, provider_call_id: &str) -> Option<CallId> {
        self.provider_index.get(provider_call_id).map(|r| r.clone())
    }

    /// List all active (non-terminal) calls.
    #[must_use]
    pub fn active_calls(&self) -> Vec<CallRecord> {
        self.active_calls
            .iter()
            .filter(|r| !r.state.is_terminal())
            .map(|r| r.clone())
            .collect()
    }

    /// Access the underlying provider.
    pub fn provider(&self) -> &Arc<RwLock<Box<dyn TelephonyProvider>>> {
        &self.provider
    }

    fn start_timeout(&self, call_id: &str) {
        let call_id_owned = call_id.to_string();
        let call_id_key = call_id_owned.clone();
        let calls = Arc::clone(&self.active_calls);
        let provider = Arc::clone(&self.provider);
        let provider_index = Arc::clone(&self.provider_index);
        let max_secs = self.max_duration_secs;

        let handle = tokio::spawn(async move {
            let call_id = call_id_owned;
            tokio::time::sleep(std::time::Duration::from_secs(max_secs)).await;
            let provider_call_id = if let Some(mut rec) = calls.get_mut(&call_id)
                && !rec.state.is_terminal()
            {
                warn!(call_id = %call_id, "max duration exceeded, hanging up");
                rec.state = CallState::Timeout;
                rec.ended_at = Some(OffsetDateTime::now_utc());
                rec.end_reason = Some(CallEndReason::Timeout);
                rec.provider_call_id.clone()
            } else {
                None
            };

            if let Some(pid) = provider_call_id {
                provider_index.remove(&pid);
                let _ = provider.read().await.hangup_call(&pid).await;
            }
            calls.remove(&call_id);
        });

        self.timeout_handles.insert(call_id_key, handle);
    }

    fn cancel_timeout(&self, call_id: &str) {
        if let Some((_, handle)) = self.timeout_handles.remove(call_id) {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::providers::mock::MockProvider};

    fn test_manager() -> CallManager {
        CallManager::new(Box::new(MockProvider::new()), 60)
    }

    #[tokio::test]
    async fn initiate_creates_call_record() {
        let mgr = test_manager();
        let call_id = mgr
            .initiate(
                "+15551111111",
                "+15552222222",
                CallMode::Conversation,
                Some("Hello"),
                "test-account",
                "https://example.com/status",
                "https://example.com/answer",
            )
            .await
            .unwrap_or_default();

        assert!(!call_id.is_empty());
        let record = mgr.get_call(&call_id);
        assert!(record.is_some());
        let record = record.unwrap_or_else(|| panic!("call record missing"));
        assert_eq!(record.from, "+15551111111");
        assert_eq!(record.to, "+15552222222");
        assert_eq!(record.state, CallState::Initiated);
        assert_eq!(record.direction, CallDirection::Outbound);
    }

    #[tokio::test]
    async fn register_inbound_creates_call() {
        let mgr = test_manager();
        let call_id = mgr.register_inbound("PROV123", "+15551111111", "+15552222222", "acct");
        let record = mgr.get_call(&call_id);
        assert!(record.is_some());
        let record = record.unwrap_or_else(|| panic!("call record missing"));
        assert_eq!(record.direction, CallDirection::Inbound);
        assert_eq!(record.state, CallState::Answered);
    }

    #[tokio::test]
    async fn handle_event_transitions_state() {
        let mgr = test_manager();
        let call_id = mgr.register_inbound("PROV456", "+1", "+2", "acct");

        mgr.handle_event(&CallEvent::Speech {
            provider_call_id: "PROV456".into(),
            text: "hello there".into(),
            confidence: Some(0.9),
        });

        let record = mgr.get_call(&call_id).unwrap_or_else(|| panic!("missing"));
        assert_eq!(record.state, CallState::Active);
        assert_eq!(record.transcript.len(), 1);
        assert_eq!(record.transcript[0].text, "hello there");
    }

    #[tokio::test]
    async fn handle_ended_event_removes_call() {
        let mgr = test_manager();
        let call_id = mgr.register_inbound("PROV789", "+1", "+2", "acct");
        assert!(mgr.get_call(&call_id).is_some());

        mgr.handle_event(&CallEvent::Ended {
            provider_call_id: "PROV789".into(),
            reason: CallEndReason::HangupUser,
        });

        // Terminal calls are cleaned up from active_calls.
        assert!(mgr.get_call(&call_id).is_none());
        assert!(mgr.resolve_call_id("PROV789").is_none());
    }

    #[tokio::test]
    async fn resolve_call_id_works() {
        let mgr = test_manager();
        let call_id = mgr.register_inbound("PROV-ABC", "+1", "+2", "acct");
        let resolved = mgr.resolve_call_id("PROV-ABC");
        assert_eq!(resolved, Some(call_id));
        assert!(mgr.resolve_call_id("UNKNOWN").is_none());
    }

    #[tokio::test]
    async fn active_calls_excludes_terminal() {
        let mgr = test_manager();
        let _cid = mgr.register_inbound("P1", "+1", "+2", "a");
        assert_eq!(mgr.active_calls().len(), 1);

        mgr.handle_event(&CallEvent::Ended {
            provider_call_id: "P1".into(),
            reason: CallEndReason::Completed,
        });
        assert_eq!(mgr.active_calls().len(), 0);
    }

    #[tokio::test]
    async fn initiate_stores_initial_message() {
        let mgr = test_manager();
        let call_id = mgr
            .initiate(
                "+1",
                "+2",
                CallMode::Notify,
                Some("Hello from bot"),
                "acct",
                "http://status",
                "http://answer",
            )
            .await
            .unwrap_or_default();
        let record = mgr.get_call(&call_id).unwrap_or_else(|| panic!("missing"));
        assert_eq!(record.initial_message.as_deref(), Some("Hello from bot"));
        assert_eq!(record.mode, CallMode::Notify);
    }

    #[tokio::test]
    async fn hangup_cleans_up_both_maps() {
        let mgr = test_manager();
        let call_id = mgr
            .initiate(
                "+1",
                "+2",
                CallMode::Conversation,
                None,
                "acct",
                "http://s",
                "http://a",
            )
            .await
            .unwrap_or_default();
        assert!(mgr.get_call(&call_id).is_some());

        mgr.hangup(&call_id).await.unwrap_or_else(|e| panic!("{e}"));
        // Both active_calls and provider_index should be cleaned up
        assert!(mgr.get_call(&call_id).is_none());
    }

    #[tokio::test]
    async fn timeout_cleans_up_live_call_maps() {
        let mgr = CallManager::new(Box::new(MockProvider::new()), 0);
        let call_id = mgr.register_inbound("PROV-TIMEOUT", "+1", "+2", "acct");

        for _ in 0..20 {
            if mgr.get_call(&call_id).is_none() && mgr.resolve_call_id("PROV-TIMEOUT").is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        assert!(mgr.get_call(&call_id).is_none());
        assert!(mgr.resolve_call_id("PROV-TIMEOUT").is_none());
        assert!(mgr.active_calls().is_empty());
    }

    #[tokio::test]
    async fn record_bot_speech_adds_transcript() {
        let mgr = test_manager();
        let call_id = mgr.register_inbound("P1", "+1", "+2", "acct");
        mgr.record_bot_speech(&call_id, "Hello caller");
        let record = mgr.get_call(&call_id).unwrap_or_else(|| panic!("missing"));
        assert_eq!(record.transcript.len(), 1);
        assert_eq!(record.transcript[0].text, "Hello caller");
        assert_eq!(record.transcript[0].speaker, Speaker::Bot);
    }
}
