//! Local CA lifecycle: a long-lived, name-constrained root that survives
//! server-certificate renewals so a trusted CA stays trusted.
//!
//! The CA is restricted (RFC 5280 name constraints) to loopback, `.localhost`,
//! `.local`, private/CGNAT/link-local IP ranges, and any extra names the server
//! certificate needs at generation time. If its key ever leaks, it cannot be
//! used to impersonate arbitrary public sites.

use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    time::SystemTime,
};

use {
    rcgen::{
        BasicConstraints, Certificate, CertificateParams, CidrSubnet, DnType, GeneralSubtree, IsCa,
        KeyPair, KeyUsagePurpose, NameConstraints,
    },
    time::OffsetDateTime,
    tracing::{info, warn},
};

use super::{Context, Result, certs::ServerSan};

const CA_CERT_FILE: &str = "ca.pem";
const CA_KEY_FILE: &str = "ca-key.pem";
const CA_SCOPE_FILE: &str = "ca-scope.txt";
/// CA certificates are valid for ten years.
const CA_VALIDITY_DAYS: i64 = 365 * 10;
/// Rotate the CA one year before expiry so a fresh one-year server
/// certificate never outlives its issuer.
const CA_ROTATE_AFTER_DAYS: i64 = 365 * 9;

const DEFAULT_DNS_SCOPE: &[&str] = &["localhost", "local"];
const DEFAULT_IP_SCOPE: &[(&str, u8)] = &[
    ("127.0.0.0", 8),
    ("::1", 128),
    ("10.0.0.0", 8),
    ("172.16.0.0", 12),
    ("192.168.0.0", 16),
    ("100.64.0.0", 10),
    ("169.254.0.0", 16),
    ("fc00::", 7),
    ("fe80::", 10),
];

/// A name-constraint subtree the CA is permitted to issue for.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum PermittedName {
    Dns(String),
    Ip { net: IpAddr, prefix: u8 },
}

impl PermittedName {
    fn covers(&self, san: &ServerSan) -> bool {
        match (self, san) {
            (Self::Dns(scope), ServerSan::Dns(name)) => {
                let name = name.strip_prefix("*.").unwrap_or(name);
                name.eq_ignore_ascii_case(scope)
                    || name
                        .to_ascii_lowercase()
                        .ends_with(&format!(".{}", scope.to_ascii_lowercase()))
            },
            (Self::Ip { net, prefix }, ServerSan::Ip(ip)) => ip_in_subnet(*ip, *net, *prefix),
            _ => false,
        }
    }

    fn for_uncovered(san: &ServerSan) -> Self {
        match san {
            ServerSan::Dns(name) => Self::Dns(name.strip_prefix("*.").unwrap_or(name).to_string()),
            ServerSan::Ip(ip) => Self::Ip {
                net: *ip,
                prefix: if ip.is_ipv4() {
                    32
                } else {
                    128
                },
            },
        }
    }

    fn to_subtree(&self) -> GeneralSubtree {
        match self {
            Self::Dns(name) => GeneralSubtree::DnsName(name.clone()),
            Self::Ip { net, prefix } => {
                GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(*net, *prefix))
            },
        }
    }

    fn metadata_line(&self) -> String {
        match self {
            Self::Dns(name) => format!("dns:{name}"),
            Self::Ip { net, prefix } => format!("ip:{net}/{prefix}"),
        }
    }

    fn from_metadata_line(line: &str) -> Option<Self> {
        let (kind, value) = line.split_once(':')?;
        match kind {
            "dns" if !value.is_empty() => Some(Self::Dns(value.to_string())),
            "ip" => {
                let (net, prefix) = value.rsplit_once('/')?;
                let net: IpAddr = net.parse().ok()?;
                let prefix: u8 = prefix.parse().ok()?;
                let max = if net.is_ipv4() {
                    32
                } else {
                    128
                };
                (prefix <= max).then_some(Self::Ip { net, prefix })
            },
            _ => None,
        }
    }
}

fn ip_in_subnet(ip: IpAddr, net: IpAddr, prefix: u8) -> bool {
    match (ip, net) {
        (IpAddr::V4(ip), IpAddr::V4(net)) => {
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            u32::from(ip) & mask == u32::from(net) & mask
        },
        (IpAddr::V6(ip), IpAddr::V6(net)) => {
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            u128::from(ip) & mask == u128::from(net) & mask
        },
        _ => false,
    }
}

/// Default CA scope widened with exact entries for any required SAN it
/// does not already cover.
pub(crate) fn scope_for(required_sans: &[ServerSan]) -> Vec<PermittedName> {
    let mut scope: Vec<PermittedName> = DEFAULT_DNS_SCOPE
        .iter()
        .map(|name| PermittedName::Dns((*name).to_string()))
        .chain(DEFAULT_IP_SCOPE.iter().filter_map(|(net, prefix)| {
            net.parse().ok().map(|net| PermittedName::Ip {
                net,
                prefix: *prefix,
            })
        }))
        .collect();
    for san in required_sans {
        if !scope_covers(&scope, san) {
            scope.push(PermittedName::for_uncovered(san));
        }
    }
    scope.sort();
    scope.dedup();
    scope
}

pub(crate) fn scope_covers(scope: &[PermittedName], san: &ServerSan) -> bool {
    scope.iter().any(|entry| entry.covers(san))
}

/// The local CA, ready to sign server certificates.
pub(crate) struct LocalCa {
    pub(crate) cert: Certificate,
    pub(crate) key: KeyPair,
    pub(crate) cert_path: PathBuf,
    /// `true` when a new CA was created by this call.
    pub(crate) created: bool,
}

/// Load the persisted CA, or create a new one when it is missing, legacy
/// (unconstrained), near expiry, or does not cover a required SAN.
pub(crate) fn load_or_create(dir: &Path, required_sans: &[ServerSan]) -> Result<LocalCa> {
    restrict_dir(dir)?;
    let cert_path = dir.join(CA_CERT_FILE);
    let key_path = dir.join(CA_KEY_FILE);
    let scope_path = dir.join(CA_SCOPE_FILE);

    match reusable_ca(&cert_path, &key_path, &scope_path, required_sans) {
        Ok(Some((key, scope))) => {
            // Older releases wrote the CA key world-readable.
            restrict_permissions(&key_path)?;
            let cert = ca_params(&scope)?.self_signed(&key)?;
            return Ok(LocalCa {
                cert,
                key,
                cert_path,
                created: false,
            });
        },
        Ok(None) => {},
        Err(reason) => {
            warn!(
                %reason,
                "regenerating local CA; run `moltis trust-ca` again if you trusted the previous one"
            );
        },
    }

    info!("generating local CA");
    let scope = scope_for(required_sans);
    let key = KeyPair::generate()?;
    let cert = ca_params(&scope)?.self_signed(&key)?;
    std::fs::write(&cert_path, cert.pem()).context("write CA certificate")?;
    write_private(&key_path, &key.serialize_pem())?;
    std::fs::write(&scope_path, scope_metadata(&scope)).context("write CA scope")?;
    Ok(LocalCa {
        cert,
        key,
        cert_path,
        created: true,
    })
}

/// `Ok(None)` when no CA exists yet, `Err(reason)` when an existing CA must
/// be replaced.
fn reusable_ca(
    cert_path: &Path,
    key_path: &Path,
    scope_path: &Path,
    required_sans: &[ServerSan],
) -> std::result::Result<Option<(KeyPair, Vec<PermittedName>)>, String> {
    if !cert_path.exists() && !key_path.exists() {
        return Ok(None);
    }
    let key_pem = std::fs::read_to_string(key_path).map_err(|_| "CA key is missing")?;
    let key = KeyPair::from_pem(&key_pem).map_err(|_| "CA key is unreadable")?;
    if !cert_path.exists() {
        return Err("CA certificate is missing".into());
    }
    if !cert_matches_key(cert_path, &key) {
        return Err("CA certificate does not match the CA key".into());
    }
    let scope = read_scope(scope_path).ok_or("CA predates name constraints")?;
    if age_days(cert_path).is_none_or(|days| days > CA_ROTATE_AFTER_DAYS) {
        return Err("CA is close to expiry".into());
    }
    if let Some(san) = required_sans.iter().find(|san| !scope_covers(&scope, san)) {
        return Err(format!("CA does not cover {san:?}"));
    }
    Ok(Some((key, scope)))
}

/// Whether the certificate in `cert_path` was issued for `key`.
///
/// A crash between writing `ca.pem` and `ca-key.pem` leaves a certificate for
/// one key next to another key. Server certificates signed with the saved key
/// would then fail to verify against the advertised CA, so the pair must
/// match before reuse. The certificate embeds the key's SubjectPublicKeyInfo
/// DER verbatim, so a byte search is exact.
fn cert_matches_key(cert_path: &Path, key: &KeyPair) -> bool {
    let Ok(pem) = std::fs::read(cert_path) else {
        return false;
    };
    let spki = key.public_key_der();
    let mut reader = pem.as_slice();
    let mut certs = rustls_pemfile::certs(&mut reader);
    match (certs.next(), certs.next()) {
        (Some(Ok(der)), None) => der.windows(spki.len()).any(|window| window == spki),
        _ => false,
    }
}

/// Deterministic CA parameters: rebuilding them with the persisted key yields
/// an issuer whose subject and key identifier match the on-disk certificate.
fn ca_params(scope: &[PermittedName]) -> Result<CertificateParams> {
    let now = OffsetDateTime::now_utc();
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params
        .distinguished_name
        .push(DnType::CommonName, "Moltis Local CA");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Moltis");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.name_constraints = Some(NameConstraints {
        permitted_subtrees: scope.iter().map(PermittedName::to_subtree).collect(),
        excluded_subtrees: Vec::new(),
    });
    params.not_before = now;
    params.not_after = now + time::Duration::days(CA_VALIDITY_DAYS);
    Ok(params)
}

fn read_scope(path: &Path) -> Option<Vec<PermittedName>> {
    let contents = std::fs::read_to_string(path).ok()?;
    let scope: Vec<_> = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(PermittedName::from_metadata_line)
        .collect();
    (!scope.is_empty()).then_some(scope)
}

fn scope_metadata(scope: &[PermittedName]) -> String {
    scope
        .iter()
        .map(|entry| format!("{}\n", entry.metadata_line()))
        .collect()
}

fn age_days(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    Some(time::Duration::try_from(age).ok()?.whole_days())
}

/// Write a private key readable only by the current user.
///
/// The key goes to a sibling temp file created owner-only, then is renamed
/// into place, so it is never readable by others, even briefly.
pub(crate) fn write_private(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("key");
    let tmp_path = path.with_file_name(format!(".{file_name}.tmp"));
    let _ = std::fs::remove_file(&tmp_path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp_path).context("create private key")?;
    file.write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .context("write private key")?;
    drop(file);
    std::fs::rename(&tmp_path, path).context("install private key")?;
    Ok(())
}

/// Restrict the certificate directory to the current user.
#[cfg(unix)]
pub(crate) fn restrict_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .context("restrict certificate directory permissions")?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn restrict_dir(_dir: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .context("restrict private key permissions")?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn ip(raw: &str) -> ServerSan {
        ServerSan::Ip(raw.parse().unwrap())
    }

    fn dns(raw: &str) -> ServerSan {
        ServerSan::Dns(raw.to_string())
    }

    #[test]
    fn default_scope_covers_loopback_lan_and_local_names() {
        let scope = scope_for(&[]);
        for san in [
            dns("localhost"),
            dns("moltis.localhost"),
            dns("*.moltis.localhost"),
            dns("m4max.local"),
            ip("127.0.0.1"),
            ip("::1"),
            ip("192.168.1.9"),
            ip("10.1.2.3"),
            ip("172.20.0.5"),
            ip("100.101.102.103"),
            ip("fd12::1"),
        ] {
            assert!(scope_covers(&scope, &san), "{san:?} should be covered");
        }
    }

    #[test]
    fn default_scope_excludes_public_names_and_ips() {
        let scope = scope_for(&[]);
        for san in [
            dns("example.com"),
            dns("notlocalhost"),
            dns("evil-local"),
            ip("8.8.8.8"),
            ip("172.32.0.1"),
            ip("2001:db8::1"),
        ] {
            assert!(!scope_covers(&scope, &san), "{san:?} should be excluded");
        }
    }

    #[test]
    fn scope_widens_with_exact_entries_for_required_sans() {
        let scope = scope_for(&[dns("m4max"), ip("203.0.113.10")]);
        assert!(scope_covers(&scope, &dns("m4max")));
        assert!(scope_covers(&scope, &ip("203.0.113.10")));
        assert!(!scope_covers(&scope, &ip("203.0.113.11")));
    }

    #[test]
    fn scope_metadata_round_trips() {
        let scope = scope_for(&[ip("203.0.113.10")]);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(CA_SCOPE_FILE);
        std::fs::write(&path, scope_metadata(&scope)).unwrap();
        assert_eq!(read_scope(&path).unwrap(), scope);
    }

    #[test]
    fn invalid_scope_lines_are_ignored() {
        for line in ["ip:10.0.0.0/33", "ip:nope/8", "dns:", "bogus"] {
            assert_eq!(PermittedName::from_metadata_line(line), None, "{line}");
        }
    }

    #[test]
    fn ca_is_reused_across_calls() {
        let tmp = tempfile::tempdir().unwrap();
        let first = load_or_create(tmp.path(), &[]).unwrap();
        assert!(first.created);
        let pem = std::fs::read_to_string(&first.cert_path).unwrap();

        let second = load_or_create(tmp.path(), &[ip("192.168.1.20")]).unwrap();
        assert!(!second.created, "LAN IP is already in scope");
        assert_eq!(std::fs::read_to_string(&second.cert_path).unwrap(), pem);
    }

    #[test]
    fn ca_is_regenerated_for_uncovered_san() {
        let tmp = tempfile::tempdir().unwrap();
        let first = load_or_create(tmp.path(), &[]).unwrap();
        let pem = std::fs::read_to_string(&first.cert_path).unwrap();

        let second = load_or_create(tmp.path(), &[ip("203.0.113.10")]).unwrap();
        assert!(second.created);
        assert_ne!(std::fs::read_to_string(&second.cert_path).unwrap(), pem);
    }

    #[test]
    fn legacy_ca_without_scope_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        load_or_create(tmp.path(), &[]).unwrap();
        std::fs::remove_file(tmp.path().join(CA_SCOPE_FILE)).unwrap();
        assert!(load_or_create(tmp.path(), &[]).unwrap().created);
    }

    #[test]
    fn ca_whose_cert_and_key_disagree_is_replaced() {
        // Simulates a crash after `ca.pem` was replaced but before the new
        // key was written: the certificate belongs to a different key.
        let tmp = tempfile::tempdir().unwrap();
        let first = load_or_create(tmp.path(), &[]).unwrap();
        let other = tempfile::tempdir().unwrap();
        let foreign = load_or_create(other.path(), &[]).unwrap();
        std::fs::copy(&foreign.cert_path, &first.cert_path).unwrap();
        assert!(!cert_matches_key(&first.cert_path, &first.key));

        let repaired = load_or_create(tmp.path(), &[]).unwrap();
        assert!(repaired.created);
        assert!(cert_matches_key(&repaired.cert_path, &repaired.key));
    }

    #[test]
    fn unreadable_ca_cert_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let first = load_or_create(tmp.path(), &[]).unwrap();
        std::fs::write(&first.cert_path, "-----BEGIN CERTIFICATE-----\ntrunc").unwrap();
        assert!(load_or_create(tmp.path(), &[]).unwrap().created);
    }

    #[cfg(unix)]
    #[test]
    fn ca_key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        load_or_create(tmp.path(), &[]).unwrap();
        let key_path = tmp.path().join(CA_KEY_FILE);
        let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // Loosened permissions from older releases are tightened on reuse.
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        load_or_create(tmp.path(), &[]).unwrap();
        let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir_mode = std::fs::metadata(tmp.path()).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);
    }

    #[test]
    fn write_private_replaces_existing_key_and_cleans_up_temp_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("server-key.pem");
        write_private(&path, "old").unwrap();
        write_private(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert!(!tmp.path().join(".server-key.pem.tmp").exists());
    }
}
