//! TLS certificate management and HTTPS server support.
//!
//! On first run, generates a local CA and server certificate (mkcert-style)
//! so the gateway can serve HTTPS out of the box. A companion plain-HTTP
//! server on a secondary port serves the CA cert for easy download and
//! redirects everything else to HTTPS.

use std::{
    io::BufReader,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use {
    anyhow::{Context, Result},
    axum::{Router, extract::State, response::IntoResponse, routing::get},
    rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SanType},
    rustls::ServerConfig,
    time::OffsetDateTime,
    tracing::info,
};

/// The hostname used for loopback URLs instead of raw `127.0.0.1`.
/// Subdomains of `.localhost` resolve to loopback per RFC 6761.
pub const LOCALHOST_DOMAIN: &str = "moltis.localhost";

/// Trait for TLS certificate management, allowing alternative implementations.
pub trait CertManager: Send + Sync {
    /// Returns (ca_cert_path, server_cert_path, server_key_path).
    /// Generates certificates if they don't exist or are expired.
    fn ensure_certs(&self) -> Result<(PathBuf, PathBuf, PathBuf)>;

    /// Build a `rustls::ServerConfig` from the given cert and key PEM files.
    fn build_rustls_config(&self, cert: &Path, key: &Path) -> Result<ServerConfig>;
}

/// Default file-system-backed certificate manager.
pub struct FsCertManager {
    cert_dir: PathBuf,
}

impl FsCertManager {
    pub fn new() -> Result<Self> {
        let dir = cert_dir()?;
        Ok(Self { cert_dir: dir })
    }

    #[cfg(test)]
    pub fn with_dir(dir: PathBuf) -> Self {
        Self { cert_dir: dir }
    }
}

/// Returns the certificate storage directory (`~/.config/moltis/certs/`).
pub fn cert_dir() -> Result<PathBuf> {
    let dir = moltis_config::config_dir()
        .unwrap_or_else(|| PathBuf::from(".moltis"))
        .join("certs");
    std::fs::create_dir_all(&dir).context("failed to create certs directory")?;
    Ok(dir)
}

impl CertManager for FsCertManager {
    fn ensure_certs(&self) -> Result<(PathBuf, PathBuf, PathBuf)> {
        let ca_cert_path = self.cert_dir.join("ca.pem");
        let ca_key_path = self.cert_dir.join("ca-key.pem");
        let server_cert_path = self.cert_dir.join("server.pem");
        let server_key_path = self.cert_dir.join("server-key.pem");

        let need_regen = !ca_cert_path.exists()
            || !server_cert_path.exists()
            || !server_key_path.exists()
            || is_expired(&server_cert_path, 30);

        if need_regen {
            info!("generating TLS certificates");
            let (ca_cert_pem, ca_key_pem, server_cert_pem, server_key_pem) = generate_all()?;
            std::fs::write(&ca_cert_path, &ca_cert_pem)?;
            std::fs::write(&ca_key_path, &ca_key_pem)?;
            std::fs::write(&server_cert_path, &server_cert_pem)?;
            std::fs::write(&server_key_path, &server_key_pem)?;
            info!(dir = %self.cert_dir.display(), "certificates written");
        }

        Ok((ca_cert_path, server_cert_path, server_key_path))
    }

    fn build_rustls_config(&self, cert: &Path, key: &Path) -> Result<ServerConfig> {
        load_rustls_config(cert, key)
    }
}

/// Check if a PEM cert file needs regeneration.
///
/// Returns `true` when the file is older than `days` days (proxy for
/// approaching expiry) **or** when it was generated before the
/// `moltis.localhost` SAN was added. The DER-encoded cert contains
/// DNS names as raw ASCII (IA5String), so a byte search on the decoded
/// DER is sufficient to detect the missing SAN.
fn is_expired(path: &Path, days: u64) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    if age.as_secs() > days * 86400 {
        return true;
    }
    // Regenerate if the cert predates the moltis.localhost SAN migration.
    needs_san_update(path)
}

/// Returns `true` if the cert at `path` does not contain the
/// `moltis.localhost` SAN (i.e. was generated before the migration).
fn needs_san_update(path: &Path) -> bool {
    let Ok(pem_bytes) = std::fs::read(path) else {
        return true;
    };
    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(pem_bytes.as_slice()))
        .filter_map(|r| r.ok())
        .collect();
    if certs.is_empty() {
        return true;
    }
    let der = certs[0].as_ref();
    !der.windows(LOCALHOST_DOMAIN.len())
        .any(|w| w == LOCALHOST_DOMAIN.as_bytes())
}

/// Generate CA + server certificates. Returns (ca_cert, ca_key, server_cert, server_key) PEM strings.
fn generate_all() -> Result<(String, String, String, String)> {
    let now = OffsetDateTime::now_utc();

    // --- CA ---
    let ca_key = KeyPair::generate()?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Moltis Local CA");
    ca_params
        .distinguished_name
        .push(DnType::OrganizationName, "Moltis");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    // 10-year validity from today.
    ca_params.not_before = now;
    ca_params.not_after = now + time::Duration::days(365 * 10);
    let ca_cert = ca_params.self_signed(&ca_key)?;

    // --- Server cert signed by CA ---
    let server_key = KeyPair::generate()?;
    let mut server_params = CertificateParams::new(vec![LOCALHOST_DOMAIN.to_string()])?;
    server_params
        .distinguished_name
        .push(DnType::CommonName, LOCALHOST_DOMAIN);
    server_params.subject_alt_names = vec![
        SanType::DnsName(LOCALHOST_DOMAIN.try_into()?),
        SanType::DnsName(format!("*.{LOCALHOST_DOMAIN}").as_str().try_into()?),
        SanType::DnsName("localhost".try_into()?),
        SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        SanType::IpAddress(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)),
    ];
    // 1-year validity from today.
    server_params.not_before = now;
    server_params.not_after = now + time::Duration::days(365);
    let server_cert = server_params.signed_by(&server_key, &ca_cert, &ca_key)?;

    Ok((
        ca_cert.pem(),
        ca_key.serialize_pem(),
        server_cert.pem(),
        server_key.serialize_pem(),
    ))
}

/// Load cert + key PEM files into a `rustls::ServerConfig`.
fn load_rustls_config(cert_path: &Path, key_path: &Path) -> Result<ServerConfig> {
    // Ensure a crypto provider is installed (ring via feature flag).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cert_file = std::fs::File::open(cert_path).context("open server cert")?;
    let key_file = std::fs::File::open(key_path).context("open server key")?;

    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_file))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("parse certs")?;

    let key = rustls_pemfile::private_key(&mut BufReader::new(key_file))
        .context("parse private key")?
        .context("no private key found")?;

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("build rustls ServerConfig")?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

// ── Plain-HTTP redirect server ──────────────────────────────────────────────

#[derive(Clone)]
struct HttpRedirectState {
    https_port: u16,
    ca_pem: Arc<Vec<u8>>,
}

/// Start a plain-HTTP server that serves the CA cert and redirects to HTTPS.
pub async fn start_http_redirect_server(
    bind: &str,
    http_port: u16,
    https_port: u16,
    ca_cert_path: &Path,
) -> Result<()> {
    let ca_pem = std::fs::read(ca_cert_path).context("read CA cert")?;
    let state = HttpRedirectState {
        https_port,
        ca_pem: Arc::new(ca_pem),
    };

    let app = Router::new()
        .route("/certs/ca.pem", get(serve_ca_cert))
        .fallback(redirect_to_https)
        .with_state(state);

    let addr: SocketAddr = format!("{bind}:{http_port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(
        "HTTP redirect server listening http://{}:{http_port}/certs/ca.pem",
        LOCALHOST_DOMAIN
    );
    axum::serve(listener, app).await?;
    Ok(())
}

async fn serve_ca_cert(State(state): State<HttpRedirectState>) -> impl IntoResponse {
    (
        [
            ("content-type", "application/x-pem-file"),
            (
                "content-disposition",
                "attachment; filename=\"moltis-ca.pem\"",
            ),
        ],
        state.ca_pem.as_ref().clone(),
    )
}

async fn redirect_to_https(
    State(state): State<HttpRedirectState>,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    let path = uri.path();
    let target = format!("https://{}:{}{}", LOCALHOST_DOMAIN, state.https_port, path);
    axum::response::Redirect::temporary(&target)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_all_produces_valid_pems() {
        let (ca_cert, ca_key, server_cert, server_key) = generate_all().unwrap();
        assert!(ca_cert.contains("BEGIN CERTIFICATE"));
        assert!(ca_key.contains("BEGIN PRIVATE KEY"));
        assert!(server_cert.contains("BEGIN CERTIFICATE"));
        assert!(server_key.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn test_certs_persist_to_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = FsCertManager::with_dir(tmp.path().to_path_buf());
        let (ca, cert, key) = mgr.ensure_certs().unwrap();
        assert!(ca.exists());
        assert!(cert.exists());
        assert!(key.exists());
    }

    #[test]
    fn test_certs_not_regenerated_if_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = FsCertManager::with_dir(tmp.path().to_path_buf());
        let (_, cert1, _) = mgr.ensure_certs().unwrap();
        let mtime1 = std::fs::metadata(&cert1).unwrap().modified().unwrap();

        // Second call should not regenerate.
        let (_, cert2, _) = mgr.ensure_certs().unwrap();
        let mtime2 = std::fs::metadata(&cert2).unwrap().modified().unwrap();
        assert_eq!(mtime1, mtime2);
    }

    #[test]
    fn test_load_rustls_config() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = FsCertManager::with_dir(tmp.path().to_path_buf());
        let (_ca, cert, key) = mgr.ensure_certs().unwrap();
        let config = mgr.build_rustls_config(&cert, &key);
        assert!(config.is_ok());
    }

    #[test]
    fn test_is_expired_missing_file() {
        assert!(is_expired(Path::new("/nonexistent/file.pem"), 30));
    }
}
