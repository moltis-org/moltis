//! Webhook authentication verification.

use {
    axum::http::HeaderMap,
    hmac::{Hmac, Mac},
    sha2::Sha256,
    subtle::ConstantTimeEq,
};

use crate::{Error, Result, types::AuthMode};

type HmacSha256 = Hmac<Sha256>;

/// Verify an inbound webhook request against the configured auth mode.
pub fn verify(
    mode: &AuthMode,
    config: Option<&serde_json::Value>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<()> {
    match mode {
        AuthMode::None => Ok(()),
        AuthMode::StaticHeader => verify_static_header(config, headers),
        AuthMode::Bearer => verify_bearer(config, headers),
        AuthMode::GithubHmacSha256 => verify_github_hmac(config, headers, body),
        AuthMode::GitlabToken => verify_gitlab_token(config, headers),
        AuthMode::StripeWebhookSignature => verify_stripe_signature(config, headers, body),
        AuthMode::LinearWebhookSignature => verify_linear_signature(config, headers, body),
        AuthMode::PagerdutyV2Signature => verify_pagerduty_signature(config, headers, body),
        AuthMode::SentryWebhookSignature => verify_sentry_signature(config, headers, body),
    }
}

fn get_config_str<'a>(config: Option<&'a serde_json::Value>, key: &str) -> Result<&'a str> {
    config
        .and_then(|c| c.get(key))
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::auth_failed(format!("missing auth config key: {key}")))
}

fn get_header_str<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| Error::auth_failed(format!("missing header: {name}")))
}

fn verify_static_header(config: Option<&serde_json::Value>, headers: &HeaderMap) -> Result<()> {
    let header_name = get_config_str(config, "header")?;
    let expected = get_config_str(config, "value")?;
    let actual = get_header_str(headers, header_name)?;
    if actual.as_bytes().ct_eq(expected.as_bytes()).into() {
        Ok(())
    } else {
        Err(Error::auth_failed("static header mismatch"))
    }
}

fn verify_bearer(config: Option<&serde_json::Value>, headers: &HeaderMap) -> Result<()> {
    let expected = get_config_str(config, "token")?;
    let auth = get_header_str(headers, "authorization")?;
    let token = auth
        .strip_prefix("Bearer ")
        .ok_or_else(|| Error::auth_failed("invalid Bearer format"))?;
    if token.as_bytes().ct_eq(expected.as_bytes()).into() {
        Ok(())
    } else {
        Err(Error::auth_failed("bearer token mismatch"))
    }
}

fn verify_github_hmac(
    config: Option<&serde_json::Value>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<()> {
    let secret = get_config_str(config, "secret")?;
    let sig_header = get_header_str(headers, "x-hub-signature-256")?;
    let sig_hex = sig_header
        .strip_prefix("sha256=")
        .ok_or_else(|| Error::auth_failed("invalid signature format"))?;
    let sig_bytes =
        hex::decode(sig_hex).map_err(|_| Error::auth_failed("invalid signature hex"))?;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| Error::auth_failed(e.to_string()))?;
    mac.update(body);
    let expected = mac.finalize().into_bytes();

    if expected[..].ct_eq(&sig_bytes).into() {
        Ok(())
    } else {
        Err(Error::auth_failed("HMAC signature mismatch"))
    }
}

fn verify_gitlab_token(config: Option<&serde_json::Value>, headers: &HeaderMap) -> Result<()> {
    let expected = get_config_str(config, "token")?;
    let actual = get_header_str(headers, "x-gitlab-token")?;
    if actual.as_bytes().ct_eq(expected.as_bytes()).into() {
        Ok(())
    } else {
        Err(Error::auth_failed("GitLab token mismatch"))
    }
}

fn verify_stripe_signature(
    config: Option<&serde_json::Value>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<()> {
    let secret = get_config_str(config, "secret")?;
    let sig_header = get_header_str(headers, "stripe-signature")?;

    // Parse "t=TIMESTAMP,v1=SIG" format
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for part in sig_header.split(',') {
        let mut kv = part.splitn(2, '=');
        match (kv.next(), kv.next()) {
            (Some("t"), Some(ts)) => timestamp = Some(ts),
            (Some("v1"), Some(sig)) => signatures.push(sig.to_string()),
            _ => {},
        }
    }
    let ts =
        timestamp.ok_or_else(|| Error::auth_failed("missing timestamp in Stripe signature"))?;
    if signatures.is_empty() {
        return Err(Error::auth_failed("missing v1 signature"));
    }

    // Check timestamp tolerance (5 minutes). Fail closed on non-numeric timestamps.
    let ts_secs = ts
        .parse::<i64>()
        .map_err(|_| Error::auth_failed("invalid Stripe signature timestamp"))?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    if (now - ts_secs).unsigned_abs() > 300 {
        return Err(Error::auth_failed("Stripe signature timestamp too old"));
    }

    // Compute expected signature: HMAC-SHA256(secret, "timestamp.body")
    // Concatenate as raw bytes to avoid lossy UTF-8 conversion.
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| Error::auth_failed(e.to_string()))?;
    mac.update(format!("{ts}.").as_bytes());
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());

    if signatures
        .iter()
        .any(|s| s.as_bytes().ct_eq(expected.as_bytes()).into())
    {
        Ok(())
    } else {
        Err(Error::auth_failed("Stripe signature mismatch"))
    }
}

fn verify_linear_signature(
    config: Option<&serde_json::Value>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<()> {
    verify_hmac_header(
        config,
        headers,
        body,
        "linear-signature",
        "secret",
        "sha256=",
    )
}

fn verify_pagerduty_signature(
    config: Option<&serde_json::Value>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<()> {
    // PagerDuty sends comma-separated "v1=SIG1,v1=SIG2" during key rotation.
    let secret = get_config_str(config, "secret")?;
    let sig_header = get_header_str(headers, "x-pagerduty-signature")?;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| Error::auth_failed(e.to_string()))?;
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());

    // Check each comma-separated signature entry.
    let matched = sig_header.split(',').any(|part| {
        let part = part.trim();
        if let Some(sig_hex) = part.strip_prefix("v1=") {
            sig_hex.as_bytes().ct_eq(expected.as_bytes()).into()
        } else {
            false
        }
    });

    if matched {
        Ok(())
    } else {
        Err(Error::auth_failed("x-pagerduty-signature mismatch"))
    }
}

fn verify_sentry_signature(
    config: Option<&serde_json::Value>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<()> {
    verify_hmac_header(config, headers, body, "sentry-hook-signature", "secret", "")
}

/// Generic HMAC-SHA256 header verifier.
fn verify_hmac_header(
    config: Option<&serde_json::Value>,
    headers: &HeaderMap,
    body: &[u8],
    header_name: &str,
    config_key: &str,
    prefix: &str,
) -> Result<()> {
    let secret = get_config_str(config, config_key)?;
    let sig_header = get_header_str(headers, header_name)?;
    let sig_hex = if prefix.is_empty() {
        sig_header
    } else {
        sig_header
            .strip_prefix(prefix)
            .ok_or_else(|| Error::auth_failed(format!("invalid {header_name} format")))?
    };
    let sig_bytes =
        hex::decode(sig_hex).map_err(|_| Error::auth_failed("invalid signature hex"))?;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| Error::auth_failed(e.to_string()))?;
    mac.update(body);
    let expected = mac.finalize().into_bytes();

    if expected[..].ct_eq(&sig_bytes).into() {
        Ok(())
    } else {
        Err(Error::auth_failed(format!("{header_name} mismatch")))
    }
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn make_headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (k, v) in pairs {
            headers.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(v).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn test_static_header_ok() {
        let config = serde_json::json!({ "header": "x-secret", "value": "abc123" });
        let headers = make_headers(&[("x-secret", "abc123")]);
        assert!(verify(&AuthMode::StaticHeader, Some(&config), &headers, b"").is_ok());
    }

    #[test]
    fn test_static_header_mismatch() {
        let config = serde_json::json!({ "header": "x-secret", "value": "abc123" });
        let headers = make_headers(&[("x-secret", "wrong")]);
        assert!(verify(&AuthMode::StaticHeader, Some(&config), &headers, b"").is_err());
    }

    #[test]
    fn test_bearer_ok() {
        let config = serde_json::json!({ "token": "mytoken" });
        let headers = make_headers(&[("authorization", "Bearer mytoken")]);
        assert!(verify(&AuthMode::Bearer, Some(&config), &headers, b"").is_ok());
    }

    #[test]
    fn test_github_hmac_ok() {
        let secret = "mysecret";
        let body = b"hello world";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());

        let config = serde_json::json!({ "secret": secret });
        let headers = make_headers(&[("x-hub-signature-256", &format!("sha256={sig}"))]);
        assert!(verify(&AuthMode::GithubHmacSha256, Some(&config), &headers, body).is_ok());
    }

    #[test]
    fn test_github_hmac_bad_sig() {
        let config = serde_json::json!({ "secret": "mysecret" });
        let headers = make_headers(&[(
            "x-hub-signature-256",
            "sha256=0000000000000000000000000000000000000000000000000000000000000000",
        )]);
        assert!(
            verify(
                &AuthMode::GithubHmacSha256,
                Some(&config),
                &headers,
                b"body"
            )
            .is_err()
        );
    }

    #[test]
    fn test_gitlab_token_ok() {
        let config = serde_json::json!({ "token": "gltoken" });
        let headers = make_headers(&[("x-gitlab-token", "gltoken")]);
        assert!(verify(&AuthMode::GitlabToken, Some(&config), &headers, b"").is_ok());
    }

    #[test]
    fn test_none_always_ok() {
        let headers = HeaderMap::new();
        assert!(verify(&AuthMode::None, None, &headers, b"anything").is_ok());
    }

    #[test]
    fn test_stripe_invalid_timestamp_fails_closed() {
        let secret = "whsec_test";
        let body = b"{}";
        // Forge a signature with t=invalid — must be rejected, not silently accepted.
        let signed_payload = "invalid.{}";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signed_payload.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());

        let config = serde_json::json!({ "secret": secret });
        let headers = make_headers(&[("stripe-signature", &format!("t=invalid,v1={sig}"))]);
        let result = verify(
            &AuthMode::StripeWebhookSignature,
            Some(&config),
            &headers,
            body,
        );
        assert!(result.is_err(), "non-numeric timestamp must be rejected");
        assert!(
            result.unwrap_err().to_string().contains("invalid"),
            "error should mention invalid timestamp"
        );
    }

    #[test]
    fn test_stripe_old_timestamp_rejected() {
        let secret = "whsec_test";
        let body = b"{}";
        // Use a timestamp 10 minutes ago (> 5 min tolerance).
        let old_ts = time::OffsetDateTime::now_utc().unix_timestamp() - 600;
        let signed_payload = format!("{old_ts}.{{}}");
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signed_payload.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());

        let config = serde_json::json!({ "secret": secret });
        let headers = make_headers(&[("stripe-signature", &format!("t={old_ts},v1={sig}"))]);
        let result = verify(
            &AuthMode::StripeWebhookSignature,
            Some(&config),
            &headers,
            body,
        );
        assert!(result.is_err(), "old timestamp must be rejected");
    }

    #[test]
    fn test_pagerduty_single_signature() {
        let secret = "pd_secret";
        let body = b"test body";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());

        let config = serde_json::json!({ "secret": secret });
        let headers = make_headers(&[("x-pagerduty-signature", &format!("v1={sig}"))]);
        assert!(
            verify(
                &AuthMode::PagerdutyV2Signature,
                Some(&config),
                &headers,
                body
            )
            .is_ok()
        );
    }

    #[test]
    fn test_pagerduty_multi_signature_key_rotation() {
        let secret = "pd_new_secret";
        let body = b"test body";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let good_sig = hex::encode(mac.finalize().into_bytes());

        // Simulate key rotation: old sig first, new sig second.
        let config = serde_json::json!({ "secret": secret });
        let header_val = format!(
            "v1=0000000000000000000000000000000000000000000000000000000000000000,v1={good_sig}"
        );
        let headers = make_headers(&[("x-pagerduty-signature", &header_val)]);
        assert!(
            verify(
                &AuthMode::PagerdutyV2Signature,
                Some(&config),
                &headers,
                body
            )
            .is_ok(),
            "should accept when any v1 entry matches"
        );
    }

    #[test]
    fn test_pagerduty_bad_signature() {
        let config = serde_json::json!({ "secret": "real_secret" });
        let headers = make_headers(&[(
            "x-pagerduty-signature",
            "v1=0000000000000000000000000000000000000000000000000000000000000000",
        )]);
        assert!(
            verify(
                &AuthMode::PagerdutyV2Signature,
                Some(&config),
                &headers,
                b"body"
            )
            .is_err()
        );
    }
}
