//! TLS certificate management and HTTPS server support.
//!
//! On first run, generates a local CA and server certificate (mkcert-style)
//! so the gateway can serve HTTPS out of the box. A companion plain-HTTP
//! server on a secondary port serves the CA cert for easy download and
//! redirects everything else to HTTPS.

pub mod error;

pub use error::{Context, Error, Result};

use std::{
    io::BufReader,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use {
    axum::{Router, extract::State, response::IntoResponse, routing::get},
    rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SanType},
    rustls::ServerConfig,
    time::OffsetDateTime,
    tokio::{
        io::AsyncWriteExt,
        net::{TcpListener, TcpStream},
    },
    tracing::{debug, info},
};

/// The hostname used for loopback URLs instead of raw `127.0.0.1`.
/// Subdomains of `.localhost` resolve to loopback per RFC 6761.
pub const LOCALHOST_DOMAIN: &str = "moltis.localhost";

/// DNS SAN names that must always exist on generated server certificates.
fn required_dns_san_names() -> Vec<String> {
    let mut names = vec![
        LOCALHOST_DOMAIN.to_string(),
        format!("*.{LOCALHOST_DOMAIN}"),
        "localhost".to_string(),
    ];

    if let Some(hostname) = hostname::get()
        .ok()
        .and_then(|host| host.into_string().ok())
    {
        append_system_host_sans(&mut names, &hostname);
    }

    names.sort_unstable();
    names.dedup();
    names
}

fn append_system_host_sans(names: &mut Vec<String>, hostname: &str) {
    let normalized = hostname.trim().trim_end_matches('.').to_ascii_lowercase();

    if normalized.is_empty() || normalized == "localhost" || normalized == LOCALHOST_DOMAIN {
        return;
    }

    names.push(normalized.clone());
    if !normalized.contains('.') {
        names.push(format!("{normalized}.local"));
    }
}

fn der_contains_ascii(der: &[u8], needle: &str) -> bool {
    der.windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

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

    #[allow(clippy::unwrap_used, clippy::expect_used)]
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
/// approaching expiry) **or** when it was generated before required DNS SANs
/// were added. The DER-encoded cert contains
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
    if age > time::Duration::days(days as i64).unsigned_abs() {
        return true;
    }
    // Regenerate if the cert predates the moltis.localhost SAN migration.
    needs_san_update(path)
}

/// Returns `true` if the cert at `path` does not contain the currently
/// required DNS SANs (i.e. was generated before the latest SAN migration).
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
    required_dns_san_names()
        .into_iter()
        // Wildcard entries are not required for compatibility checks.
        .filter(|name| !name.starts_with("*."))
        .any(|name| !der_contains_ascii(der, &name))
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
    let mut subject_alt_names: Vec<SanType> = required_dns_san_names()
        .into_iter()
        .filter_map(|name| name.as_str().try_into().ok().map(SanType::DnsName))
        .collect();
    subject_alt_names.push(SanType::IpAddress(std::net::IpAddr::V4(
        std::net::Ipv4Addr::LOCALHOST,
    )));
    subject_alt_names.push(SanType::IpAddress(std::net::IpAddr::V6(
        std::net::Ipv6Addr::LOCALHOST,
    )));
    server_params.subject_alt_names = subject_alt_names;
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
    localhost_mode: bool,
    bind_host: String,
}

fn is_localhost_name(name: &str) -> bool {
    matches!(name, "localhost" | "127.0.0.1" | "::1") || name.ends_with(".localhost")
}

fn host_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    use axum::http::uri::Authority;

    let raw = headers.get(axum::http::header::HOST)?.to_str().ok()?;
    let authority: Authority = raw.parse().ok()?;
    Some(authority.host().to_string())
}

fn format_url_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// Start a plain-HTTP server that serves the CA cert and redirects to HTTPS.
pub async fn start_http_redirect_server(
    bind: &str,
    http_port: u16,
    https_port: u16,
    ca_cert_path: &Path,
) -> Result<()> {
    let ca_pem = std::fs::read(ca_cert_path).context("read CA cert")?;
    let localhost_mode = is_localhost_name(bind);
    let state = HttpRedirectState {
        https_port,
        ca_pem: Arc::new(ca_pem),
        localhost_mode,
        bind_host: bind.to_string(),
    };

    let app = Router::new()
        .route("/certs/ca.pem", get(serve_ca_cert))
        .fallback(redirect_to_https)
        .with_state(state);

    let addr: SocketAddr = format!("{bind}:{http_port}").parse()?;
    let listener = TcpListener::bind(addr).await?;
    let display_host = if localhost_mode {
        "localhost"
    } else {
        bind
    };
    info!(
        "HTTP redirect server listening http://{}:{http_port}/certs/ca.pem",
        display_host
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
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    let host = if state.localhost_mode {
        "localhost".to_string()
    } else {
        host_from_headers(&headers).unwrap_or_else(|| state.bind_host.clone())
    };
    let path_and_query = uri.path_and_query().map_or("/", |pq| pq.as_str());
    let target = format!(
        "https://{}:{}{}",
        format_url_host(&host),
        state.https_port,
        path_and_query
    );
    axum::response::Redirect::temporary(&target)
}

// ── TLS server with HTTP redirect ───────────────────────────────────────────

/// Serve an axum app over TLS on a single port, with automatic HTTP-to-HTTPS
/// redirect for plain HTTP requests.
///
/// When a client connects with plain HTTP instead of TLS, a 301 redirect is
/// sent pointing to the HTTPS URL. This prevents the "broken page" that users
/// see when accidentally navigating to `http://host:port/` instead of `https://`.
pub async fn serve_tls_with_http_redirect(
    listener: TcpListener,
    tls_config: Arc<ServerConfig>,
    app: Router,
    port: u16,
    bind_host: &str,
) -> Result<()> {
    let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
    let localhost_mode = is_localhost_name(bind_host);
    let bind_host = bind_host.to_string();

    // Create the make_service so ConnectInfo<SocketAddr> works in handlers.
    let mut make_service = app.into_make_service_with_connect_info::<SocketAddr>();

    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                if is_accept_error(&e) {
                    continue;
                }
                tracing::error!("accept error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            },
        };

        // Peek at the first byte to distinguish TLS from plain HTTP.
        // TLS ClientHello always starts with 0x16 (ContentType::Handshake).
        let mut peek_buf = [0u8; 1];
        match stream.peek(&mut peek_buf).await {
            Ok(1) if peek_buf[0] == 0x16 => {
                // TLS — hand off to a per-connection task.
                let acceptor = acceptor.clone();
                let service = <_ as tower::Service<SocketAddr>>::call(&mut make_service, addr)
                    .await
                    .unwrap_or_else(|e| match e {});
                tokio::spawn(async move {
                    let Ok(tls_stream) = acceptor.accept(stream).await.inspect_err(|e| {
                        debug!("TLS handshake failed from {addr}: {e}");
                    }) else {
                        return;
                    };
                    let io = hyper_util::rt::TokioIo::new(tls_stream);
                    let hyper_service = hyper_util::service::TowerToHyperService::new(service);
                    if let Err(e) = hyper_util::server::conn::auto::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    )
                    .serve_connection_with_upgrades(io, hyper_service)
                    .await
                    {
                        debug!("connection error from {addr}: {e}");
                    }
                });
            },
            Ok(_) => {
                // Plain HTTP — send redirect in a background task.
                let redirect_host = bind_host.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        send_http_redirect(stream, port, &redirect_host, localhost_mode).await
                    {
                        debug!("HTTP redirect to {addr} failed: {e}");
                    }
                });
            },
            Err(e) => {
                debug!("peek failed from {addr}: {e}");
            },
        }
    }
}

fn is_accept_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    )
}

/// Read a plain HTTP request from a TCP stream and send a 301 redirect to HTTPS.
async fn send_http_redirect(
    mut stream: TcpStream,
    https_port: u16,
    bind_host: &str,
    localhost_mode: bool,
) -> Result<()> {
    // Read enough to parse the request line and Host header.
    let mut buf = vec![0u8; 4096];
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), stream.peek(&mut buf))
        .await
        .context("timeout reading HTTP request")?
        .context("reading HTTP request")?;

    let raw = String::from_utf8_lossy(&buf[..n]);

    // Parse request path from first line: "GET /path HTTP/1.x"
    let path = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    // Parse Host header, stripping port if present.
    let host = if localhost_mode {
        "localhost".to_string()
    } else {
        raw.lines()
            .find_map(|line| {
                if line.get(..5)?.eq_ignore_ascii_case("host:") {
                    let value = line[5..].trim();
                    // Use Authority parser to correctly handle IPv6, ports, etc.
                    if let Ok(authority) = value.parse::<axum::http::uri::Authority>() {
                        Some(authority.host().to_string())
                    } else {
                        Some(value.to_string())
                    }
                } else {
                    None
                }
            })
            .unwrap_or_else(|| bind_host.to_string())
    };

    let location = format!("https://{}:{}{}", format_url_host(&host), https_port, path);
    let body =
        format!("<html><body>Redirecting to <a href=\"{location}\">{location}</a></body></html>");
    let response = format!(
        "HTTP/1.1 301 Moved Permanently\r\n\
         Location: {location}\r\n\
         Content-Type: text/html\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );

    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, axum::response::IntoResponse, tokio::io::AsyncReadExt as _};

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

    #[test]
    fn required_dns_sans_always_include_loopback_defaults() {
        let names = required_dns_san_names();
        assert!(names.contains(&LOCALHOST_DOMAIN.to_string()));
        assert!(names.contains(&format!("*.{LOCALHOST_DOMAIN}")));
        assert!(names.contains(&"localhost".to_string()));
    }

    #[test]
    fn append_system_host_sans_adds_local_suffix_for_plain_hostname() {
        let mut names = Vec::new();
        append_system_host_sans(&mut names, "m4max");
        assert_eq!(names, vec!["m4max".to_string(), "m4max.local".to_string()]);
    }

    #[test]
    fn append_system_host_sans_skips_localhost_aliases() {
        let mut names = Vec::new();
        append_system_host_sans(&mut names, "localhost");
        append_system_host_sans(&mut names, LOCALHOST_DOMAIN);
        assert!(names.is_empty());
    }

    #[test]
    fn host_from_headers_strips_port() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "example.com:8080".parse().unwrap(),
        );
        assert_eq!(host_from_headers(&headers).as_deref(), Some("example.com"));
    }

    #[test]
    fn host_from_headers_extracts_ipv6() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "[::1]:8080".parse().unwrap());
        assert_eq!(host_from_headers(&headers).as_deref(), Some("[::1]"));
    }

    #[tokio::test]
    async fn redirect_uses_localhost_for_loopback_mode() {
        let state = HttpRedirectState {
            https_port: 3443,
            ca_pem: Arc::new(Vec::new()),
            localhost_mode: true,
            bind_host: "127.0.0.1".to_string(),
        };
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "moltis.localhost:18080".parse().unwrap(),
        );
        let uri: axum::http::Uri = "/foo?bar=baz".parse().unwrap();
        let response = redirect_to_https(State(state), headers, uri)
            .await
            .into_response();

        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert_eq!(location, "https://localhost:3443/foo?bar=baz");
    }

    #[tokio::test]
    async fn redirect_uses_request_host_for_non_loopback_mode() {
        let state = HttpRedirectState {
            https_port: 3443,
            ca_pem: Arc::new(Vec::new()),
            localhost_mode: false,
            bind_host: "0.0.0.0".to_string(),
        };
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "gateway.example.com:8080".parse().unwrap(),
        );
        let uri: axum::http::Uri = "/foo".parse().unwrap();
        let response = redirect_to_https(State(state), headers, uri)
            .await
            .into_response();

        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert_eq!(location, "https://gateway.example.com:3443/foo");
    }

    #[tokio::test]
    async fn send_http_redirect_sends_301_with_correct_location() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            send_http_redirect(stream, 13131, "0.0.0.0", false)
                .await
                .unwrap();
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        // Send a minimal HTTP request.
        AsyncWriteExt::write_all(
            &mut client,
            b"GET /setup?token=abc HTTP/1.1\r\nHost: 192.168.1.50:13131\r\n\r\n",
        )
        .await
        .unwrap();

        let mut buf = vec![0u8; 4096];
        let n = client.read(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf[..n]);

        assert!(response.starts_with("HTTP/1.1 301"));
        assert!(response.contains("Location: https://192.168.1.50:13131/setup?token=abc"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn send_http_redirect_uses_localhost_in_localhost_mode() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            send_http_redirect(stream, 13131, "127.0.0.1", true)
                .await
                .unwrap();
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        AsyncWriteExt::write_all(
            &mut client,
            b"GET / HTTP/1.1\r\nHost: 192.168.1.50:13131\r\n\r\n",
        )
        .await
        .unwrap();

        let mut buf = vec![0u8; 4096];
        let n = client.read(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf[..n]);

        assert!(response.contains("Location: https://localhost:13131/"));
        server.await.unwrap();
    }

    #[test]
    fn is_accept_error_matches_connection_errors() {
        assert!(is_accept_error(&std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset"
        )));
        assert!(is_accept_error(&std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "aborted"
        )));
        assert!(!is_accept_error(&std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "in use"
        )));
    }
}
