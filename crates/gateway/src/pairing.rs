//! Device pairing state machine and device token management.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("pair request not found")]
    PairRequestNotFound,

    #[error("pair request already {0:?}")]
    PairRequestNotPending(PairStatus),

    #[error("pair request expired")]
    PairRequestExpired,

    #[error("device not found")]
    DeviceNotFound,
}

pub type Result<T> = std::result::Result<T, Error>;

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PairStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

#[derive(Debug, Clone)]
pub struct PairRequest {
    pub id: String,
    pub device_id: String,
    pub display_name: Option<String>,
    pub platform: String,
    pub public_key: Option<String>,
    pub nonce: String,
    pub status: PairStatus,
    pub created_at: Instant,
    pub expires_at: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceToken {
    pub token: String,
    pub device_id: String,
    pub scopes: Vec<String>,
    pub issued_at_ms: u64,
    pub revoked: bool,
}

// ── Pairing state ───────────────────────────────────────────────────────────

/// In-memory pairing state; tracks pending pair requests and issued device tokens.
pub struct PairingState {
    pending: HashMap<String, PairRequest>,
    devices: HashMap<String, DeviceToken>,
    pair_ttl: Duration,
}

impl Default for PairingState {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingState {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            devices: HashMap::new(),
            pair_ttl: Duration::from_secs(300), // 5 min
        }
    }

    /// Submit a new pairing request. Returns the generated nonce.
    pub fn request_pair(
        &mut self,
        device_id: &str,
        display_name: Option<&str>,
        platform: &str,
        public_key: Option<&str>,
    ) -> PairRequest {
        let id = uuid::Uuid::new_v4().to_string();
        let nonce = uuid::Uuid::new_v4().to_string();
        let now = Instant::now();
        let req = PairRequest {
            id: id.clone(),
            device_id: device_id.to_string(),
            display_name: display_name.map(|s| s.to_string()),
            platform: platform.to_string(),
            public_key: public_key.map(|s| s.to_string()),
            nonce,
            status: PairStatus::Pending,
            created_at: now,
            expires_at: now + self.pair_ttl,
        };
        self.pending.insert(id, req.clone());
        req
    }

    /// List all non-expired pending requests.
    pub fn list_pending(&self) -> Vec<&PairRequest> {
        let now = Instant::now();
        self.pending
            .values()
            .filter(|r| r.status == PairStatus::Pending && now < r.expires_at)
            .collect()
    }

    /// Approve a pending pair request. Issues a device token.
    pub fn approve(&mut self, pair_id: &str) -> Result<DeviceToken> {
        let req = self
            .pending
            .get_mut(pair_id)
            .ok_or(Error::PairRequestNotFound)?;
        if req.status != PairStatus::Pending {
            return Err(Error::PairRequestNotPending(req.status));
        }
        if Instant::now() > req.expires_at {
            req.status = PairStatus::Expired;
            return Err(Error::PairRequestExpired);
        }
        req.status = PairStatus::Approved;

        let token = DeviceToken {
            token: uuid::Uuid::new_v4().to_string(),
            device_id: req.device_id.clone(),
            scopes: vec![
                "operator.read".into(),
                "operator.write".into(),
                "operator.approvals".into(),
            ],
            issued_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            revoked: false,
        };
        self.devices.insert(req.device_id.clone(), token.clone());
        Ok(token)
    }

    /// Reject a pending pair request.
    pub fn reject(&mut self, pair_id: &str) -> Result<()> {
        let req = self
            .pending
            .get_mut(pair_id)
            .ok_or(Error::PairRequestNotFound)?;
        if req.status != PairStatus::Pending {
            return Err(Error::PairRequestNotPending(req.status));
        }
        req.status = PairStatus::Rejected;
        Ok(())
    }

    /// List all approved (non-revoked) devices.
    pub fn list_devices(&self) -> Vec<&DeviceToken> {
        self.devices.values().filter(|d| !d.revoked).collect()
    }

    /// Rotate a device token: revoke old, issue new.
    pub fn rotate_token(&mut self, device_id: &str) -> Result<DeviceToken> {
        let existing = self
            .devices
            .get_mut(device_id)
            .ok_or(Error::DeviceNotFound)?;
        existing.revoked = true;

        let new_token = DeviceToken {
            token: uuid::Uuid::new_v4().to_string(),
            device_id: device_id.to_string(),
            scopes: existing.scopes.clone(),
            issued_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            revoked: false,
        };
        self.devices
            .insert(device_id.to_string(), new_token.clone());
        Ok(new_token)
    }

    /// Revoke a device token permanently.
    pub fn revoke_token(&mut self, device_id: &str) -> Result<()> {
        let existing = self
            .devices
            .get_mut(device_id)
            .ok_or(Error::DeviceNotFound)?;
        existing.revoked = true;
        Ok(())
    }

    /// Evict expired pending requests.
    pub fn evict_expired(&mut self) {
        let now = Instant::now();
        self.pending
            .retain(|_, r| !(r.status == PairStatus::Pending && now > r.expires_at));
    }
}
