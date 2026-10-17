#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;

use {
    super::*,
    crate::{
        connection::Connection,
        error::FailureKind,
        test_support::{MockConfig, MockServer},
    },
};

#[tokio::test]
async fn probe_returns_leaf_fingerprint() {
    let (server, leaf, _) = MockServer::start_tls(MockConfig::default()).await;
    let seen = probe_fingerprint(&server.endpoint()).await.unwrap();
    assert_eq!(seen, fingerprint(&leaf));
}

#[tokio::test]
async fn probe_rejects_plain_http() {
    let server = MockServer::start(MockConfig::default()).await;
    assert!(matches!(
        probe_fingerprint(&server.endpoint()).await.unwrap_err(),
        Error::Tls(_)
    ));
}

#[tokio::test]
async fn pinned_fingerprint_connects() {
    let (server, leaf, _) = MockServer::start_tls(MockConfig::default()).await;
    let mut options = server.options();
    options.endpoint = options.endpoint.with_tls(TlsTrust::Fingerprint {
        sha256: display_fingerprint(&fingerprint(&leaf)),
    });
    let conn = Connection::connect(&options).await.unwrap();
    assert_eq!(conn.call("echo", json!(7)).await.unwrap(), json!(7));
}

#[tokio::test]
async fn changed_fingerprint_is_a_tls_failure() {
    let (server, ..) = MockServer::start_tls(MockConfig::default()).await;
    let mut options = server.options();
    options.endpoint = options.endpoint.with_tls(TlsTrust::Fingerprint {
        sha256: fingerprint(b"some other certificate"),
    });
    let error = Connection::connect(&options).await.unwrap_err();
    assert_eq!(error.failure_kind(), FailureKind::Tls, "{error:?}");
    assert!(error.to_string().contains("fingerprint changed"), "{error}");
}

#[tokio::test]
async fn local_ca_is_trusted() {
    let (server, _, ca) = MockServer::start_tls(MockConfig::default()).await;
    let mut options = server.options();
    options.endpoint = options.endpoint.with_tls(TlsTrust::CaPem { pem: ca });
    Connection::connect(&options).await.unwrap();
}

#[tokio::test]
async fn unknown_ca_is_a_tls_failure() {
    let (server, ..) = MockServer::start_tls(MockConfig::default()).await;
    let error = Connection::connect(&server.options()).await.unwrap_err();
    assert_eq!(error.failure_kind(), FailureKind::Tls, "{error:?}");
}
