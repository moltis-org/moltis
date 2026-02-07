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

#[cfg(test)]
mod tests {
    use super::*;

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
}
