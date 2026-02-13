use std::time::Instant;

use {dashmap::DashMap, webauthn_rs::prelude::*};

use crate::auth::CredentialStore;

/// Challenge TTL: 5 minutes.
const CHALLENGE_TTL_SECS: u64 = 300;

/// Pending registration challenge.
struct PendingRegistration {
    state: PasskeyRegistration,
    created_at: Instant,
}

/// Pending authentication challenge.
struct PendingAuthentication {
    state: PasskeyAuthentication,
    created_at: Instant,
}

/// WebAuthn state manager. Wraps `webauthn-rs` and stores in-flight
/// challenges in a `DashMap` with TTL-based expiry.
pub struct WebAuthnState {
    webauthn: Webauthn,
    pending_registrations: DashMap<String, PendingRegistration>,
    pending_authentications: DashMap<String, PendingAuthentication>,
}

impl WebAuthnState {
    /// Create a new WebAuthn state.
    ///
    /// `rp_id` is typically the hostname (e.g. "localhost" or "moltis.example.com").
    /// `rp_origin` is the full origin URL (e.g. "https://localhost:18080").
    /// `extra_origins` are additional origins accepted during verification (e.g.
    /// `http://m4max.local:18080` when accessing via mDNS hostname).
    pub fn new(
        rp_id: &str,
        rp_origin: &webauthn_rs::prelude::Url,
        extra_origins: &[webauthn_rs::prelude::Url],
    ) -> anyhow::Result<Self> {
        let mut builder = WebauthnBuilder::new(rp_id, rp_origin)
            .map_err(|e| anyhow::anyhow!("webauthn builder error: {e}"))?;
        for origin in extra_origins {
            builder = builder.append_allowed_origin(origin);
        }
        let webauthn = builder
            .rp_name("moltis")
            .build()
            .map_err(|e| anyhow::anyhow!("webauthn build error: {e}"))?;

        Ok(Self {
            webauthn,
            pending_registrations: DashMap::new(),
            pending_authentications: DashMap::new(),
        })
    }

    /// Begin passkey registration. Returns (challenge_id, creation_options_json).
    pub fn start_registration(
        &self,
        existing_passkeys: &[Passkey],
    ) -> anyhow::Result<(String, CreationChallengeResponse)> {
        self.cleanup_expired();

        // Single-user model: fixed user ID.
        let user_id = Uuid::new_v4();

        let exclude: Vec<CredentialID> = existing_passkeys
            .iter()
            .map(|pk| pk.cred_id().clone())
            .collect();

        let exclude_opt = if exclude.is_empty() {
            None
        } else {
            Some(exclude)
        };

        let (ccr, reg_state) = self
            .webauthn
            .start_passkey_registration(user_id, "owner", "Owner", exclude_opt)
            .map_err(|e| anyhow::anyhow!("start_passkey_registration: {e}"))?;

        let challenge_id = uuid::Uuid::new_v4().to_string();
        self.pending_registrations
            .insert(challenge_id.clone(), PendingRegistration {
                state: reg_state,
                created_at: Instant::now(),
            });

        Ok((challenge_id, ccr))
    }

    /// Finish passkey registration. Returns the new Passkey credential.
    pub fn finish_registration(
        &self,
        challenge_id: &str,
        response: &RegisterPublicKeyCredential,
    ) -> anyhow::Result<Passkey> {
        let (_, pending) = self
            .pending_registrations
            .remove(challenge_id)
            .ok_or_else(|| anyhow::anyhow!("no pending registration for this challenge"))?;

        if pending.created_at.elapsed().as_secs() > CHALLENGE_TTL_SECS {
            anyhow::bail!("registration challenge expired");
        }

        let passkey = self
            .webauthn
            .finish_passkey_registration(response, &pending.state)
            .map_err(|e| anyhow::anyhow!("finish_passkey_registration: {e}"))?;

        Ok(passkey)
    }

    /// Begin passkey authentication. Returns (challenge_id, request_options_json).
    pub fn start_authentication(
        &self,
        credentials: &[Passkey],
    ) -> anyhow::Result<(String, RequestChallengeResponse)> {
        self.cleanup_expired();

        if credentials.is_empty() {
            anyhow::bail!("no passkeys registered");
        }

        let (rcr, auth_state) = self
            .webauthn
            .start_passkey_authentication(credentials)
            .map_err(|e| anyhow::anyhow!("start_passkey_authentication: {e}"))?;

        let challenge_id = uuid::Uuid::new_v4().to_string();
        self.pending_authentications
            .insert(challenge_id.clone(), PendingAuthentication {
                state: auth_state,
                created_at: Instant::now(),
            });

        Ok((challenge_id, rcr))
    }

    /// Finish passkey authentication. Returns the authenticated result.
    pub fn finish_authentication(
        &self,
        challenge_id: &str,
        response: &PublicKeyCredential,
    ) -> anyhow::Result<AuthenticationResult> {
        let (_, pending) = self
            .pending_authentications
            .remove(challenge_id)
            .ok_or_else(|| anyhow::anyhow!("no pending authentication for this challenge"))?;

        if pending.created_at.elapsed().as_secs() > CHALLENGE_TTL_SECS {
            anyhow::bail!("authentication challenge expired");
        }

        let result = self
            .webauthn
            .finish_passkey_authentication(response, &pending.state)
            .map_err(|e| anyhow::anyhow!("finish_passkey_authentication: {e}"))?;

        Ok(result)
    }

    /// Return all allowed WebAuthn origins as strings (primary + extras).
    ///
    /// Strips the trailing `/` that `Url::to_string()` appends for path-less
    /// URLs so the frontend can display clean `host:port` values.
    pub fn get_allowed_origins(&self) -> Vec<String> {
        self.webauthn
            .get_allowed_origins()
            .iter()
            .map(|url| {
                let s = url.to_string();
                s.strip_suffix('/').unwrap_or(&s).to_string()
            })
            .collect()
    }

    fn cleanup_expired(&self) {
        let cutoff = std::time::Duration::from_secs(CHALLENGE_TTL_SECS);
        self.pending_registrations
            .retain(|_, v| v.created_at.elapsed() < cutoff);
        self.pending_authentications
            .retain(|_, v| v.created_at.elapsed() < cutoff);
    }
}

/// Load all stored passkeys from the credential store as `webauthn_rs::Passkey` objects.
pub async fn load_passkeys(store: &CredentialStore) -> anyhow::Result<Vec<Passkey>> {
    let rows = store.load_all_passkey_data().await?;
    let mut passkeys = Vec::with_capacity(rows.len());
    for (_id, data) in rows {
        let pk: Passkey = serde_json::from_slice(&data)?;
        passkeys.push(pk);
    }
    Ok(passkeys)
}
