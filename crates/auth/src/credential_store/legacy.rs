use {
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

use crate::credential_store::util::safe_equal;

/// Result of an authentication attempt.
#[derive(Debug, Clone)]
pub struct AuthResult {
    pub ok: bool,
    pub reason: Option<String>,
}

/// Legacy resolved auth from environment vars (kept for migration).
#[derive(Clone)]
pub struct ResolvedAuth {
    pub mode: AuthMode,
    pub token: Option<Secret<String>>,
    pub password: Option<Secret<String>>,
}

impl std::fmt::Debug for ResolvedAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedAuth")
            .field("mode", &self.mode)
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    Token,
    Password,
}

/// Authenticate an incoming WebSocket connect request against legacy env-var auth.
pub fn authorize_connect(
    auth: &ResolvedAuth,
    provided_token: Option<&str>,
    provided_password: Option<&str>,
    _remote_ip: Option<&str>,
) -> AuthResult {
    match auth.mode {
        AuthMode::Token => {
            let Some(ref expected) = auth.token else {
                return AuthResult {
                    ok: true,
                    reason: None,
                };
            };
            match provided_token {
                Some(t) if safe_equal(t, expected.expose_secret()) => AuthResult {
                    ok: true,
                    reason: None,
                },
                Some(_) => AuthResult {
                    ok: false,
                    reason: Some("invalid token".into()),
                },
                None => AuthResult {
                    ok: false,
                    reason: Some("token required".into()),
                },
            }
        },
        AuthMode::Password => {
            let Some(ref expected) = auth.password else {
                return AuthResult {
                    ok: true,
                    reason: None,
                };
            };
            match provided_password {
                Some(p) if safe_equal(p, expected.expose_secret()) => AuthResult {
                    ok: true,
                    reason: None,
                },
                Some(_) => AuthResult {
                    ok: false,
                    reason: Some("invalid password".into()),
                },
                None => AuthResult {
                    ok: false,
                    reason: Some("password required".into()),
                },
            }
        },
    }
}

/// Resolve auth config from environment / config values.
pub fn resolve_auth(token: Option<String>, password: Option<String>) -> ResolvedAuth {
    let mode = if password.is_some() {
        AuthMode::Password
    } else {
        AuthMode::Token
    };
    ResolvedAuth {
        mode,
        token: token.map(Secret::new),
        password: password.map(Secret::new),
    }
}
