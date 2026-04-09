//! In-memory OTP state for self-approval of non-allowlisted DM users.
//!
//! When `dm_policy = Allowlist` and `otp_self_approval = true`, the bot issues
//! a 6-digit OTP challenge to unknown users. If they reply with the correct
//! code they are automatically added to the allowlist.

use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime},
};

use rand::Rng;

use crate::{
    ChannelType,
    plugin::{ChannelEvent, ChannelEventSink},
};

/// How long an OTP code stays valid.
const OTP_TTL: Duration = Duration::from_secs(300);

/// Maximum wrong-code attempts before lockout.
const MAX_ATTEMPTS: u32 = 3;

/// Per-account OTP state.
pub struct OtpState {
    challenges: HashMap<String, OtpChallenge>,
    lockouts: HashMap<String, Lockout>,
    cooldown: Duration,
}

/// A pending OTP challenge for a single peer.
pub struct OtpChallenge {
    pub code: String,
    pub peer_id: String,
    pub username: Option<String>,
    pub sender_name: Option<String>,
    pub created_at: Instant,
    pub expires_at: Instant,
    pub attempts: u32,
}

/// Lockout state after too many failed attempts.
struct Lockout {
    until: Instant,
}

/// Result of initiating a challenge.
#[derive(Debug, PartialEq, Eq)]
pub enum OtpInitResult {
    /// Challenge created; contains the 6-digit code.
    Created(String),
    /// A challenge already exists for this peer.
    AlreadyPending,
    /// Peer is locked out.
    LockedOut,
}

/// Result of verifying a code.
#[derive(Debug, PartialEq, Eq)]
pub enum OtpVerifyResult {
    /// Code matched — peer should be approved.
    Approved,
    /// Wrong code; `attempts_left` remaining before lockout.
    WrongCode { attempts_left: u32 },
    /// Peer is locked out after too many failures.
    LockedOut,
    /// No pending challenge for this peer.
    NoPending,
    /// The challenge has expired.
    Expired,
}

/// Snapshot of a pending challenge for external consumers (API/UI).
#[derive(Debug, Clone, serde::Serialize)]
pub struct OtpChallengeInfo {
    pub peer_id: String,
    pub username: Option<String>,
    pub sender_name: Option<String>,
    pub code: String,
    pub expires_at: i64,
}

impl OtpState {
    pub fn new(cooldown_secs: u64) -> Self {
        Self {
            challenges: HashMap::new(),
            lockouts: HashMap::new(),
            cooldown: Duration::from_secs(cooldown_secs),
        }
    }

    /// Update the lockout cooldown duration (takes effect on future lockouts).
    pub fn set_cooldown(&mut self, cooldown_secs: u64) {
        self.cooldown = Duration::from_secs(cooldown_secs);
    }

    /// Initiate an OTP challenge for `peer_id`.
    pub fn initiate(
        &mut self,
        peer_id: &str,
        username: Option<String>,
        sender_name: Option<String>,
    ) -> OtpInitResult {
        let now = Instant::now();

        // Check lockout first.
        if let Some(lockout) = self.lockouts.get(peer_id) {
            if now < lockout.until {
                return OtpInitResult::LockedOut;
            }
            self.lockouts.remove(peer_id);
        }

        // Check for existing unexpired challenge.
        if let Some(existing) = self.challenges.get(peer_id) {
            if now < existing.expires_at {
                return OtpInitResult::AlreadyPending;
            }
            // Expired — remove and issue a new one.
            self.challenges.remove(peer_id);
        }

        let code = generate_otp_code();
        let challenge = OtpChallenge {
            code: code.clone(),
            peer_id: peer_id.to_string(),
            username,
            sender_name,
            created_at: now,
            expires_at: now + OTP_TTL,
            attempts: 0,
        };
        self.challenges.insert(peer_id.to_string(), challenge);
        OtpInitResult::Created(code)
    }

    /// Verify a code submitted by `peer_id`.
    pub fn verify(&mut self, peer_id: &str, code: &str) -> OtpVerifyResult {
        let now = Instant::now();

        // Check lockout.
        if let Some(lockout) = self.lockouts.get(peer_id) {
            if now < lockout.until {
                return OtpVerifyResult::LockedOut;
            }
            self.lockouts.remove(peer_id);
        }

        let challenge = match self.challenges.get_mut(peer_id) {
            Some(c) => c,
            None => return OtpVerifyResult::NoPending,
        };

        // Check expiry.
        if now >= challenge.expires_at {
            self.challenges.remove(peer_id);
            return OtpVerifyResult::Expired;
        }

        // Check code (constant-time-ish comparison not needed for 6-digit OTP).
        if challenge.code == code {
            self.challenges.remove(peer_id);
            return OtpVerifyResult::Approved;
        }

        // Wrong code.
        challenge.attempts += 1;
        if challenge.attempts >= MAX_ATTEMPTS {
            self.challenges.remove(peer_id);
            self.lockouts.insert(peer_id.to_string(), Lockout {
                until: now + self.cooldown,
            });
            return OtpVerifyResult::LockedOut;
        }

        OtpVerifyResult::WrongCode {
            attempts_left: MAX_ATTEMPTS - challenge.attempts,
        }
    }

    /// Check if a challenge is pending (and not expired) for `peer_id`.
    pub fn has_pending(&self, peer_id: &str) -> bool {
        self.challenges
            .get(peer_id)
            .is_some_and(|c| Instant::now() < c.expires_at)
    }

    /// Check if `peer_id` is currently locked out.
    pub fn is_locked_out(&self, peer_id: &str) -> bool {
        self.lockouts
            .get(peer_id)
            .is_some_and(|l| Instant::now() < l.until)
    }

    /// List all pending (non-expired) challenges with epoch timestamps.
    pub fn list_pending(&self) -> Vec<OtpChallengeInfo> {
        let now_instant = Instant::now();
        let now_system = SystemTime::now();

        self.challenges
            .values()
            .filter(|c| now_instant < c.expires_at)
            .map(|c| {
                // Convert Instant expiry to epoch by computing delta from now.
                let remaining = c.expires_at.saturating_duration_since(now_instant);
                let expires_epoch = now_system
                    .checked_add(remaining)
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);

                OtpChallengeInfo {
                    peer_id: c.peer_id.clone(),
                    username: c.username.clone(),
                    sender_name: c.sender_name.clone(),
                    code: c.code.clone(),
                    expires_at: expires_epoch,
                }
            })
            .collect()
    }

    /// Remove expired challenges and elapsed lockouts.
    pub fn evict_expired(&mut self) {
        let now = Instant::now();
        self.challenges.retain(|_, c| now < c.expires_at);
        self.lockouts.retain(|_, l| now < l.until);
    }
}

/// Generate a random 6-digit OTP code.
fn generate_otp_code() -> String {
    let code: u32 = rand::rng().random_range(100_000..1_000_000);
    code.to_string()
}

pub async fn emit_otp_challenge(
    sink: Option<&dyn ChannelEventSink>,
    channel_type: ChannelType,
    account_id: &str,
    peer_id: &str,
    username: Option<&str>,
    sender_name: Option<&str>,
    code: String,
    expires_at: i64,
) {
    if let Some(sink) = sink {
        sink.emit(ChannelEvent::OtpChallenge {
            channel_type,
            account_id: account_id.to_string(),
            peer_id: peer_id.to_string(),
            username: username.map(String::from),
            sender_name: sender_name.map(String::from),
            code,
            expires_at,
        })
        .await;
    }
}

pub async fn emit_otp_resolution(
    sink: Option<&dyn ChannelEventSink>,
    channel_type: ChannelType,
    account_id: &str,
    peer_id: &str,
    username: Option<&str>,
    resolution: &str,
) {
    if let Some(sink) = sink {
        sink.emit(ChannelEvent::OtpResolved {
            channel_type,
            account_id: account_id.to_string(),
            peer_id: peer_id.to_string(),
            username: username.map(String::from),
            resolution: resolution.to_string(),
        })
        .await;
    }
}

pub async fn approve_sender_via_otp(
    sink: Option<&dyn ChannelEventSink>,
    channel_type: ChannelType,
    account_id: &str,
    approval_identifier: &str,
    peer_id: &str,
    username: Option<&str>,
) {
    if let Some(sink) = sink {
        sink.request_sender_approval(channel_type.as_str(), account_id, approval_identifier)
            .await;
        emit_otp_resolution(
            Some(sink),
            channel_type,
            account_id,
            peer_id,
            username,
            "approved",
        )
        .await;
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use {
        super::*,
        crate::{ChannelEvent, ChannelEventSink, ChannelMessageMeta, ChannelReplyTarget, Result},
        async_trait::async_trait,
    };

    #[derive(Default)]
    struct TestSinkState {
        events: Mutex<Vec<ChannelEvent>>,
        approvals: Mutex<Vec<(String, String, String)>>,
    }

    struct TestSink {
        state: Arc<TestSinkState>,
    }

    #[async_trait]
    impl ChannelEventSink for TestSink {
        async fn emit(&self, event: ChannelEvent) {
            self.state
                .events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(event);
        }

        async fn dispatch_to_chat(
            &self,
            _text: &str,
            _reply_to: ChannelReplyTarget,
            _meta: ChannelMessageMeta,
        ) {
        }

        async fn dispatch_command(
            &self,
            _command: &str,
            _reply_to: ChannelReplyTarget,
        ) -> Result<String> {
            Ok(String::new())
        }

        async fn request_disable_account(
            &self,
            _channel_type: &str,
            _account_id: &str,
            _reason: &str,
        ) {
        }

        async fn request_sender_approval(
            &self,
            channel_type: &str,
            account_id: &str,
            identifier: &str,
        ) {
            self.state
                .approvals
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((
                    channel_type.to_string(),
                    account_id.to_string(),
                    identifier.to_string(),
                ));
        }

        async fn save_channel_voice(
            &self,
            _audio_data: &[u8],
            _filename: &str,
            _reply_to: &ChannelReplyTarget,
        ) -> Option<String> {
            None
        }

        async fn transcribe_voice(&self, _audio_data: &[u8], _format: &str) -> Result<String> {
            Err(crate::Error::unavailable("not needed"))
        }

        async fn voice_stt_available(&self) -> bool {
            false
        }

        async fn update_location(
            &self,
            _reply_to: &ChannelReplyTarget,
            _lat: f64,
            _lon: f64,
        ) -> bool {
            false
        }
    }

    #[test]
    fn initiate_creates_challenge() {
        let mut state = OtpState::new(300);
        match state.initiate("user1", Some("alice".into()), Some("Alice".into())) {
            OtpInitResult::Created(code) => {
                assert_eq!(code.len(), 6);
                assert!(code.chars().all(|c| c.is_ascii_digit()));
            },
            other => panic!("expected Created, got {other:?}"),
        }
        assert!(state.has_pending("user1"));
    }

    #[test]
    fn initiate_already_pending() {
        let mut state = OtpState::new(300);
        assert!(matches!(
            state.initiate("user1", None, None),
            OtpInitResult::Created(_)
        ));
        assert_eq!(
            state.initiate("user1", None, None),
            OtpInitResult::AlreadyPending
        );
    }

    #[test]
    fn verify_correct_code() {
        let mut state = OtpState::new(300);
        let code = match state.initiate("user1", None, None) {
            OtpInitResult::Created(c) => c,
            _ => unreachable!(),
        };
        assert_eq!(state.verify("user1", &code), OtpVerifyResult::Approved);
        assert!(!state.has_pending("user1"));
    }

    #[test]
    fn verify_wrong_code() {
        let mut state = OtpState::new(300);
        let _code = match state.initiate("user1", None, None) {
            OtpInitResult::Created(c) => c,
            _ => unreachable!(),
        };
        assert_eq!(
            state.verify("user1", "000000"),
            OtpVerifyResult::WrongCode { attempts_left: 2 }
        );
        assert_eq!(
            state.verify("user1", "000001"),
            OtpVerifyResult::WrongCode { attempts_left: 1 }
        );
        // Third wrong attempt triggers lockout.
        assert_eq!(state.verify("user1", "000002"), OtpVerifyResult::LockedOut);
        assert!(!state.has_pending("user1"));
        assert!(state.is_locked_out("user1"));
    }

    #[test]
    fn verify_no_pending() {
        let mut state = OtpState::new(300);
        assert_eq!(state.verify("ghost", "123456"), OtpVerifyResult::NoPending);
    }

    #[test]
    fn verify_expired() {
        let mut state = OtpState::new(300);
        let _code = match state.initiate("user1", None, None) {
            OtpInitResult::Created(c) => c,
            _ => unreachable!(),
        };
        // Manually expire the challenge.
        state.challenges.get_mut("user1").unwrap().expires_at =
            Instant::now() - Duration::from_secs(1);

        assert_eq!(state.verify("user1", &_code), OtpVerifyResult::Expired);
        assert!(!state.has_pending("user1"));
    }

    #[test]
    fn lockout_prevents_initiate() {
        let mut state = OtpState::new(300);
        let _code = match state.initiate("user1", None, None) {
            OtpInitResult::Created(c) => c,
            _ => unreachable!(),
        };
        // Exhaust attempts.
        state.verify("user1", "000000");
        state.verify("user1", "000001");
        state.verify("user1", "000002");

        assert_eq!(
            state.initiate("user1", None, None),
            OtpInitResult::LockedOut
        );
    }

    #[test]
    fn lockout_prevents_verify() {
        let mut state = OtpState::new(300);
        let _code = match state.initiate("user1", None, None) {
            OtpInitResult::Created(c) => c,
            _ => unreachable!(),
        };
        state.verify("user1", "000000");
        state.verify("user1", "000001");
        state.verify("user1", "000002");

        assert_eq!(state.verify("user1", "123456"), OtpVerifyResult::LockedOut);
    }

    #[test]
    fn evict_expired_clears_old_entries() {
        let mut state = OtpState::new(300);
        state.initiate("user1", None, None);
        state.initiate("user2", None, None);

        // Expire user1's challenge.
        state.challenges.get_mut("user1").unwrap().expires_at =
            Instant::now() - Duration::from_secs(1);

        state.evict_expired();
        assert!(!state.has_pending("user1"));
        assert!(state.has_pending("user2"));
    }

    #[test]
    fn evict_expired_clears_elapsed_lockouts() {
        let mut state = OtpState::new(0); // 0s cooldown for test
        let _code = match state.initiate("user1", None, None) {
            OtpInitResult::Created(c) => c,
            _ => unreachable!(),
        };
        state.verify("user1", "000000");
        state.verify("user1", "000001");
        state.verify("user1", "000002");

        // Lockout should have elapsed immediately (0s cooldown).
        state.evict_expired();
        assert!(!state.is_locked_out("user1"));
    }

    #[test]
    fn list_pending_returns_active_challenges() {
        let mut state = OtpState::new(300);
        state.initiate("user1", Some("alice".into()), Some("Alice".into()));
        state.initiate("user2", None, None);

        let pending = state.list_pending();
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().any(|c| c.peer_id == "user1"));
        assert!(pending.iter().any(|c| c.peer_id == "user2"));

        // All have valid expiry epochs.
        for c in &pending {
            assert!(c.expires_at > 0);
        }
    }

    #[test]
    fn expired_challenge_allows_new_initiate() {
        let mut state = OtpState::new(300);
        state.initiate("user1", None, None);

        // Expire the challenge.
        state.challenges.get_mut("user1").unwrap().expires_at =
            Instant::now() - Duration::from_secs(1);

        // Should create a new one.
        assert!(matches!(
            state.initiate("user1", None, None),
            OtpInitResult::Created(_)
        ));
    }

    #[test]
    fn otp_code_is_six_digits() {
        for _ in 0..100 {
            let code = generate_otp_code();
            assert_eq!(code.len(), 6);
            let n: u32 = code.parse().unwrap();
            assert!(n >= 100_000);
            assert!(n < 1_000_000);
        }
    }

    #[tokio::test]
    async fn approve_sender_via_otp_requests_approval_and_emits_resolution() {
        let state = Arc::new(TestSinkState::default());
        let sink = TestSink {
            state: Arc::clone(&state),
        };

        approve_sender_via_otp(
            Some(&sink),
            ChannelType::Matrix,
            "bot1",
            "@alice:matrix.org",
            "@alice:matrix.org",
            Some("@alice:matrix.org"),
        )
        .await;

        let approvals = state.approvals.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(approvals.as_slice(), [(
            "matrix".to_string(),
            "bot1".to_string(),
            "@alice:matrix.org".to_string()
        )]);
        drop(approvals);

        let events = state.events.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(events.len(), 1);
        match &events[0] {
            ChannelEvent::OtpResolved {
                channel_type,
                account_id,
                peer_id,
                username,
                resolution,
            } => {
                assert_eq!(*channel_type, ChannelType::Matrix);
                assert_eq!(account_id, "bot1");
                assert_eq!(peer_id, "@alice:matrix.org");
                assert_eq!(username.as_deref(), Some("@alice:matrix.org"));
                assert_eq!(resolution, "approved");
            },
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
