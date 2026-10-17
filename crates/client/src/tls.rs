//! TLS trust for daemon connections.
//!
//! Moltis daemons usually serve a certificate signed by their own local CA.
//! A client can trust that CA's PEM (local daemon, read from disk), pin the
//! server certificate's SHA-256 fingerprint after the user confirmed it (trust
//! on first use, like SSH host keys), or rely on the platform trust store.

use std::{
    fmt::Write as _,
    sync::{Arc, Mutex},
    time::Duration,
};

use {
    rustls::{
        ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
        client::{
            WebPkiServerVerifier,
            danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        },
        crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature},
        pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject},
    },
    rustls_platform_verifier::BuilderVerifierExt,
    serde::{Deserialize, Serialize},
    sha2::{Digest, Sha256},
};

use crate::{
    endpoint::Endpoint,
    error::{Error, Result},
};

/// How the server certificate of an `https` endpoint is verified.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TlsTrust {
    /// The operating system's trust store.
    #[default]
    System,
    /// Accept exactly the server certificate with this SHA-256 fingerprint
    /// (hex, colons and case ignored).
    Fingerprint { sha256: String },
    /// Certificates chaining to this CA (PEM), with hostname verification.
    CaPem { pem: String },
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// SHA-256 of a DER certificate as lowercase hex.
#[must_use]
pub fn fingerprint(cert: &[u8]) -> String {
    Sha256::digest(cert)
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Lowercase hex with separators removed, so user-pasted fingerprints compare.
#[must_use]
pub fn normalize_fingerprint(input: &str) -> String {
    input
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// `AB:CD:...` for display.
#[must_use]
pub fn display_fingerprint(hex: &str) -> String {
    normalize_fingerprint(hex)
        .as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(":")
}

/// Build the rustls client config for `trust`.
pub fn client_config(trust: &TlsTrust) -> Result<Arc<ClientConfig>> {
    let provider = provider();
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?;
    let config = match trust {
        TlsTrust::System => builder
            .with_platform_verifier()
            .map_err(|e| Error::Tls(e.to_string()))?
            .with_no_client_auth(),
        TlsTrust::Fingerprint { sha256 } => {
            let expected = normalize_fingerprint(sha256);
            if expected.len() != 64 {
                return Err(Error::Tls(
                    "pinned fingerprint must be a SHA-256 hash".into(),
                ));
            }
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(PinnedVerifier {
                    expected: Some(expected),
                    seen: Mutex::new(None),
                    provider,
                }))
                .with_no_client_auth()
        },
        TlsTrust::CaPem { pem } => {
            let mut roots = RootCertStore::empty();
            for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
                let cert = cert.map_err(|e| Error::Tls(format!("invalid CA PEM: {e}")))?;
                roots
                    .add(cert)
                    .map_err(|e| Error::Tls(format!("invalid CA certificate: {e}")))?;
            }
            if roots.is_empty() {
                return Err(Error::Tls("CA PEM contains no certificate".into()));
            }
            let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
                .build()
                .map_err(|e| Error::Tls(e.to_string()))?;
            builder.with_webpki_verifier(verifier).with_no_client_auth()
        },
    };
    Ok(Arc::new(config))
}

/// Connect to `endpoint` without verifying its certificate and return the
/// SHA-256 fingerprint of the leaf certificate, so the user can confirm it
/// before it is pinned. Nothing is sent over the connection.
pub async fn probe_fingerprint(endpoint: &Endpoint) -> Result<String> {
    if !endpoint.is_tls() {
        return Err(Error::Tls(format!("{endpoint} does not use TLS")));
    }
    let verifier = Arc::new(PinnedVerifier {
        expected: None,
        seen: Mutex::new(None),
        provider: provider(),
    });
    let config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::clone(&verifier) as Arc<dyn ServerCertVerifier>)
        .with_no_client_auth();
    let server_name = ServerName::try_from(endpoint.host().trim_matches(['[', ']']).to_string())
        .map_err(|e| Error::Tls(format!("invalid server name: {e}")))?;
    let connect = async {
        let tcp = tokio::net::TcpStream::connect((
            endpoint.host().trim_matches(['[', ']']),
            endpoint.port(),
        ))
        .await
        .map_err(|e| Error::Connect(format!("{endpoint}: {e}")))?;
        tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(server_name, tcp)
            .await
            .map_err(|e| Error::Tls(e.to_string()))
    };
    tokio::time::timeout(Duration::from_secs(10), connect)
        .await
        .map_err(|_| Error::Timeout)??;
    let seen = verifier
        .seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    seen.ok_or_else(|| Error::Tls("server sent no certificate".into()))
}

/// Accepts exactly one leaf certificate (`expected`), or records whatever
/// certificate is presented when `expected` is `None` (probing only).
/// Handshake signatures are always verified.
#[derive(Debug)]
struct PinnedVerifier {
    expected: Option<String>,
    seen: Mutex<Option<String>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let actual = fingerprint(end_entity.as_ref());
        match &self.expected {
            Some(expected) if *expected != actual => Err(rustls::Error::General(format!(
                "server certificate fingerprint changed (expected {}, got {})",
                display_fingerprint(expected),
                display_fingerprint(&actual)
            ))),
            _ => {
                *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(actual);
                Ok(ServerCertVerified::assertion())
            },
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
#[path = "tls_tests.rs"]
mod integration_tests;

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_formats() {
        let hex = fingerprint(b"hello");
        assert_eq!(
            hex,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        let shown = display_fingerprint(&hex);
        assert!(shown.starts_with("2C:F2:4D"));
        assert_eq!(normalize_fingerprint(&shown), hex);
    }

    #[test]
    fn configs_build() {
        assert!(client_config(&TlsTrust::System).is_ok());
        assert!(
            client_config(&TlsTrust::Fingerprint {
                sha256: fingerprint(b"x")
            })
            .is_ok()
        );
        assert!(
            client_config(&TlsTrust::Fingerprint {
                sha256: "abcd".into()
            })
            .is_err()
        );
        assert!(client_config(&TlsTrust::CaPem { pem: "nope".into() }).is_err());
        let ca = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        assert!(client_config(&TlsTrust::CaPem { pem: ca.cert.pem() }).is_ok());
    }

    #[test]
    fn pinned_verifier_rejects_other_certs() {
        let cert = CertificateDer::from(b"cert-a".to_vec());
        let other = CertificateDer::from(b"cert-b".to_vec());
        let verifier = PinnedVerifier {
            expected: Some(fingerprint(b"cert-a")),
            seen: Mutex::new(None),
            provider: provider(),
        };
        let name = ServerName::try_from("localhost").unwrap();
        assert!(
            verifier
                .verify_server_cert(&cert, &[], &name, &[], UnixTime::now())
                .is_ok()
        );
        let err = verifier
            .verify_server_cert(&other, &[], &name, &[], UnixTime::now())
            .unwrap_err();
        assert!(err.to_string().contains("fingerprint changed"));
    }
}
