use std::sync::Arc;

use {
    moltis_auth::{AuthMode, CredentialStore, ResolvedAuth},
    sqlx::SqlitePool,
};

#[tokio::test]
async fn sync_runtime_webauthn_host_registers_new_origin() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let credential_store = Arc::new(CredentialStore::new(pool).await.unwrap());
    let gateway = crate::state::GatewayState::with_options(
        ResolvedAuth {
            mode: AuthMode::Token,
            token: None,
            password: None,
        },
        crate::services::GatewayServices::noop(),
        moltis_config::MoltisConfig::default(),
        None,
        Some(Arc::clone(&credential_store)),
        None,
        false,
        false,
        false,
        None,
        None,
        18789,
        false,
        None,
        None,
        #[cfg(feature = "metrics")]
        None,
        #[cfg(feature = "metrics")]
        None,
        #[cfg(feature = "vault")]
        None,
    );
    let registry = Arc::new(tokio::sync::RwLock::new(
        crate::auth_webauthn::WebAuthnRegistry::new(),
    ));

    let notice = crate::server::startup::sync_runtime_webauthn_host_and_notice(
        &gateway,
        Some(&registry),
        Some("team-gateway.ngrok.app"),
        Some("https://team-gateway.ngrok.app"),
        "test",
    )
    .await;

    assert!(notice.is_none(), "unexpected notice: {notice:?}");
    assert!(
        registry
            .read()
            .await
            .contains_host("team-gateway.ngrok.app")
    );
    assert!(
        gateway.passkey_host_update_pending().await.is_empty(),
        "passkey warning should not be queued without existing passkeys"
    );
}

#[tokio::test]
async fn sync_runtime_webauthn_host_rejects_invalid_origin() {
    let gateway = crate::state::GatewayState::new(
        ResolvedAuth {
            mode: AuthMode::Token,
            token: None,
            password: None,
        },
        crate::services::GatewayServices::noop(),
    );
    let registry = Arc::new(tokio::sync::RwLock::new(
        crate::auth_webauthn::WebAuthnRegistry::new(),
    ));

    let notice = crate::server::startup::sync_runtime_webauthn_host_and_notice(
        &gateway,
        Some(&registry),
        Some("team-gateway.ngrok.app"),
        Some("not a url"),
        "test",
    )
    .await;

    assert!(notice.is_none());
    assert!(
        !registry
            .read()
            .await
            .contains_host("team-gateway.ngrok.app")
    );
}
