use std::{
    collections::BTreeSet,
    io::BufReader,
    net::IpAddr,
    path::{Path, PathBuf},
    time::SystemTime,
};

use {
    rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SanType},
    rustls::ServerConfig,
    time::OffsetDateTime,
    tracing::info,
};

use super::{Context, Result};

/// The hostname used for loopback URLs instead of raw `127.0.0.1`.
/// Subdomains of `.localhost` resolve to loopback per RFC 6761.
pub const LOCALHOST_DOMAIN: &str = "moltis.localhost";

/// Additional SAN entries that should be present on the auto-generated server
/// certificate for this runtime.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ServerSan {
    Dns(String),
    Ip(IpAddr),
}

impl ServerSan {
    fn metadata_line(&self) -> String {
        match self {
            Self::Dns(name) => format!("dns:{name}"),
            Self::Ip(ip) => format!("ip:{ip}"),
        }
    }

    fn from_metadata_line(line: &str) -> Option<Self> {
        let (kind, value) = line.split_once(':')?;
        match kind {
            "dns" if !value.is_empty() => Some(Self::Dns(value.to_string())),
            "ip" => value.parse().ok().map(Self::Ip),
            _ => None,
        }
    }
}

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

fn default_required_sans() -> Vec<ServerSan> {
    let mut sans = required_dns_san_names()
        .into_iter()
        .map(ServerSan::Dns)
        .collect::<Vec<_>>();
    sans.push(ServerSan::Ip(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)));
    sans.push(ServerSan::Ip(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)));
    normalize_sans(sans)
}

fn required_server_sans(runtime_sans: &[ServerSan]) -> Vec<ServerSan> {
    normalize_sans(
        default_required_sans()
            .into_iter()
            .chain(runtime_sans.iter().cloned()),
    )
}

fn normalize_sans(sans: impl IntoIterator<Item = ServerSan>) -> Vec<ServerSan> {
    sans.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn is_default_loopback_ip(ip: &IpAddr) -> bool {
    matches!(
        ip,
        IpAddr::V4(v4) if *v4 == std::net::Ipv4Addr::LOCALHOST
    ) || matches!(
        ip,
        IpAddr::V6(v6) if *v6 == std::net::Ipv6Addr::LOCALHOST
    )
}

fn der_contains_ascii(der: &[u8], needle: &str) -> bool {
    der.windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Trait for TLS certificate management, allowing alternative implementations.
pub trait CertManager: Send + Sync {
    /// Returns (ca_cert_path, server_cert_path, server_key_path).
    /// Generates certificates if they don't exist or are expired.
    fn ensure_certs(&self, runtime_sans: &[ServerSan]) -> Result<(PathBuf, PathBuf, PathBuf)>;

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
    fn ensure_certs(&self, runtime_sans: &[ServerSan]) -> Result<(PathBuf, PathBuf, PathBuf)> {
        let ca_cert_path = self.cert_dir.join("ca.pem");
        let ca_key_path = self.cert_dir.join("ca-key.pem");
        let server_cert_path = self.cert_dir.join("server.pem");
        let server_key_path = self.cert_dir.join("server-key.pem");
        let server_sans_path = self.cert_dir.join("server-sans.txt");
        let required_sans = required_server_sans(runtime_sans);

        let need_regen = !ca_cert_path.exists()
            || !server_cert_path.exists()
            || !server_key_path.exists()
            || is_expired(&server_cert_path, &server_sans_path, 30, &required_sans);

        if need_regen {
            info!("generating TLS certificates");
            let (ca_cert_pem, ca_key_pem, server_cert_pem, server_key_pem) =
                generate_all(&required_sans)?;
            std::fs::write(&ca_cert_path, &ca_cert_pem)?;
            std::fs::write(&ca_key_path, &ca_key_pem)?;
            std::fs::write(&server_cert_path, &server_cert_pem)?;
            std::fs::write(&server_key_path, &server_key_pem)?;
            write_san_metadata(&server_sans_path, &required_sans)?;
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
fn is_expired(
    path: &Path,
    san_metadata_path: &Path,
    days: u64,
    required_sans: &[ServerSan],
) -> bool {
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
    needs_san_update(path, san_metadata_path, required_sans)
}

/// Returns `true` if the cert at `path` does not contain the currently
/// required DNS SANs (i.e. was generated before the latest SAN migration).
fn needs_san_update(path: &Path, san_metadata_path: &Path, required_sans: &[ServerSan]) -> bool {
    if let Some(existing_sans) = read_san_metadata(san_metadata_path) {
        return required_sans
            .iter()
            .any(|entry| !existing_sans.contains(entry));
    }

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
    required_sans.iter().any(|entry| match entry {
        // Wildcard entries are not required for compatibility checks.
        ServerSan::Dns(name) if !name.starts_with("*.") => !der_contains_ascii(der, name),
        // Legacy certs do not carry SAN metadata, so require one regeneration
        // to pick up any non-loopback runtime IPs.
        ServerSan::Ip(ip) if !is_default_loopback_ip(ip) => true,
        _ => false,
    })
}

fn read_san_metadata(path: &Path) -> Option<Vec<ServerSan>> {
    let contents = std::fs::read_to_string(path).ok()?;
    let mut sans = Vec::new();
    sans.extend(
        contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter_map(ServerSan::from_metadata_line),
    );
    Some(normalize_sans(sans))
}

fn write_san_metadata(path: &Path, sans: &[ServerSan]) -> Result<()> {
    let contents = sans
        .iter()
        .map(ServerSan::metadata_line)
        .collect::<Vec<_>>()
        .join("\n");
    let contents = if contents.is_empty() {
        String::new()
    } else {
        format!("{contents}\n")
    };
    std::fs::write(path, contents).context("write server SAN metadata")?;
    Ok(())
}

/// Generate CA + server certificates. Returns (ca_cert, ca_key, server_cert, server_key) PEM strings.
fn generate_all(required_sans: &[ServerSan]) -> Result<(String, String, String, String)> {
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
    let subject_alt_names: Vec<SanType> = required_sans
        .iter()
        .filter_map(|entry| match entry {
            ServerSan::Dns(name) => name.as_str().try_into().ok().map(SanType::DnsName),
            ServerSan::Ip(ip) => Some(SanType::IpAddress(*ip)),
        })
        .collect();
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
pub(crate) fn load_rustls_config(cert_path: &Path, key_path: &Path) -> Result<ServerConfig> {
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

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_all_produces_valid_pems() {
        let (ca_cert, ca_key, server_cert, server_key) =
            generate_all(&default_required_sans()).unwrap();
        assert!(ca_cert.contains("BEGIN CERTIFICATE"));
        assert!(ca_key.contains("BEGIN PRIVATE KEY"));
        assert!(server_cert.contains("BEGIN CERTIFICATE"));
        assert!(server_key.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn test_certs_persist_to_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = FsCertManager::with_dir(tmp.path().to_path_buf());
        let (ca, cert, key) = mgr.ensure_certs(&[]).unwrap();
        assert!(ca.exists());
        assert!(cert.exists());
        assert!(key.exists());
    }

    #[test]
    fn test_certs_not_regenerated_if_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = FsCertManager::with_dir(tmp.path().to_path_buf());
        let (_, cert1, _) = mgr.ensure_certs(&[]).unwrap();
        let mtime1 = std::fs::metadata(&cert1).unwrap().modified().unwrap();

        // Second call should not regenerate.
        let (_, cert2, _) = mgr.ensure_certs(&[]).unwrap();
        let mtime2 = std::fs::metadata(&cert2).unwrap().modified().unwrap();
        assert_eq!(mtime1, mtime2);
    }

    #[test]
    fn test_load_rustls_config() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = FsCertManager::with_dir(tmp.path().to_path_buf());
        let (_ca, cert, key) = mgr.ensure_certs(&[]).unwrap();
        let config = mgr.build_rustls_config(&cert, &key);
        assert!(config.is_ok());
    }

    #[test]
    fn test_is_expired_missing_file() {
        assert!(is_expired(
            Path::new("/nonexistent/file.pem"),
            Path::new("/nonexistent/server-sans.txt"),
            30,
            &default_required_sans()
        ));
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
    fn required_server_sans_include_runtime_ip_without_duplicates() {
        let sans = required_server_sans(&[
            ServerSan::Ip("192.168.1.9".parse().unwrap()),
            ServerSan::Ip("192.168.1.9".parse().unwrap()),
        ]);
        let count = sans
            .iter()
            .filter(|entry| matches!(entry, ServerSan::Ip(ip) if ip.to_string() == "192.168.1.9"))
            .count();
        assert_eq!(count, 1);
        assert!(
            sans.contains(&ServerSan::Ip(std::net::Ipv4Addr::LOCALHOST.into())),
            "default loopback SANs should remain present"
        );
    }

    #[test]
    fn certs_regenerate_when_runtime_sans_change() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = FsCertManager::with_dir(tmp.path().to_path_buf());

        let runtime_a = [ServerSan::Ip("192.168.1.9".parse().unwrap())];
        let (_, cert1, _) = mgr.ensure_certs(&runtime_a).unwrap();
        let cert1_pem = std::fs::read_to_string(&cert1).unwrap();

        let (_, cert2, _) = mgr.ensure_certs(&runtime_a).unwrap();
        let cert2_pem = std::fs::read_to_string(&cert2).unwrap();
        assert_eq!(
            cert1_pem, cert2_pem,
            "same SAN set should not regenerate certs"
        );

        let runtime_b = [ServerSan::Ip("192.168.1.10".parse().unwrap())];
        let (_, cert3, _) = mgr.ensure_certs(&runtime_b).unwrap();
        let cert3_pem = std::fs::read_to_string(&cert3).unwrap();
        assert!(
            cert3_pem != cert2_pem,
            "changing runtime SANs should regenerate certs"
        );
    }

    #[test]
    fn read_san_metadata_ignores_invalid_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let metadata_path = tmp.path().join("server-sans.txt");
        std::fs::write(
            &metadata_path,
            "dns:gateway.local\nbogus line\nip:192.168.1.8\n\nip:not-an-ip\n",
        )
        .unwrap();

        let sans = read_san_metadata(&metadata_path).unwrap();
        assert_eq!(sans, vec![
            ServerSan::Dns("gateway.local".to_string()),
            ServerSan::Ip("192.168.1.8".parse().unwrap()),
        ]);
    }
}
