use std::{
    fs::{self, File},
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use secrecy::ExposeSecret;

use crate::{Error, HttpsCredentials, HttpsSource, RequestedRevision, SshCredentials, SshSource};

/// Blocking transport boundary for network repository acquisition.
///
/// The system backend uses isolated child `git` processes. HTTPS is pinned to
/// the validated DNS result, while SSH uses a restrictive OpenSSH wrapper.
pub trait RepositoryBackend: Send + Sync {
    fn fetch_https(
        &self,
        source: &HttpsSource,
        credentials: Option<&HttpsCredentials>,
        revision: &RequestedRevision,
        destination: &Path,
        max_fetch_bytes: u64,
        max_fetch_duration: Duration,
    ) -> Result<(), Error>;

    fn fetch_ssh(
        &self,
        source: &SshSource,
        credentials: &SshCredentials,
        revision: &RequestedRevision,
        destination: &Path,
        max_fetch_bytes: u64,
        max_fetch_duration: Duration,
    ) -> Result<(), Error>;
}

trait HostResolver: Send + Sync {
    fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>>;
}

#[derive(Debug)]
struct SystemHostResolver;

impl HostResolver for SystemHostResolver {
    fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>> {
        (host, port)
            .to_socket_addrs()
            .map(|addresses| addresses.map(|address| address.ip()).collect())
    }
}

#[derive(Clone)]
pub struct SystemRepositoryBackend {
    resolver: Arc<dyn HostResolver>,
}

impl Default for SystemRepositoryBackend {
    fn default() -> Self {
        Self {
            resolver: Arc::new(SystemHostResolver),
        }
    }
}

impl RepositoryBackend for SystemRepositoryBackend {
    fn fetch_https(
        &self,
        source: &HttpsSource,
        credentials: Option<&HttpsCredentials>,
        revision: &RequestedRevision,
        destination: &Path,
        max_fetch_bytes: u64,
        max_fetch_duration: Duration,
    ) -> Result<(), Error> {
        let address = validated_https_address(source, self.resolver.as_ref())?;
        fetch_https(
            source,
            credentials,
            revision,
            destination,
            max_fetch_bytes,
            max_fetch_duration,
            address,
        )
    }

    fn fetch_ssh(
        &self,
        source: &SshSource,
        credentials: &SshCredentials,
        revision: &RequestedRevision,
        destination: &Path,
        max_fetch_bytes: u64,
        max_fetch_duration: Duration,
    ) -> Result<(), Error> {
        fetch_ssh(
            source,
            credentials,
            revision,
            destination,
            max_fetch_bytes,
            max_fetch_duration,
        )
    }
}

#[allow(clippy::result_large_err)] // Required by gix's credential callback result type.
fn fetch_https(
    source: &HttpsSource,
    credentials: Option<&HttpsCredentials>,
    revision: &RequestedRevision,
    destination: &Path,
    max_fetch_bytes: u64,
    max_fetch_duration: Duration,
    address: IpAddr,
) -> Result<(), Error> {
    let expected_authority = source.authority().to_ascii_lowercase();
    if let Some(credentials) = credentials
        && !credentials.host().eq_ignore_ascii_case(&expected_authority)
    {
        return Err(Error::CredentialHostMismatch {
            expected: expected_authority,
            actual: credentials.host().to_string(),
        });
    }
    let auth_dir = tempfile::Builder::new()
        .prefix("moltis-git-https-")
        .tempdir()
        .map_err(|error| Error::io(std::env::temp_dir(), error))?;
    set_directory_permissions(auth_dir.path())?;
    let askpass_path = auth_dir.path().join("askpass");
    write_restricted(&askpass_path, https_askpass_script().as_bytes(), 0o700)?;
    let host = source
        .url()
        .host_str()
        .ok_or_else(|| Error::InvalidSource("HTTPS URL has no host".into()))?;
    let port = source.url().port_or_known_default().unwrap_or(443);
    let configs = vec![format!(
        "http.curloptResolve={host}:{port}:{}",
        curl_resolve_address(address)
    )];
    let mut environment: Vec<(&str, &std::ffi::OsStr)> =
        vec![("GIT_ASKPASS", askpass_path.as_os_str())];
    if let Some(credentials) = credentials {
        let username_path = auth_dir.path().join("username");
        let token_path = auth_dir.path().join("token");
        write_restricted(&username_path, credentials.username().as_bytes(), 0o600)?;
        write_restricted(
            &token_path,
            credentials.token().expose_secret().as_bytes(),
            0o600,
        )?;
        environment.push(("MOLTIS_GIT_USERNAME_FILE", username_path.as_os_str()));
        environment.push(("MOLTIS_GIT_TOKEN_FILE", token_path.as_os_str()));
        return run_bounded_fetch(
            source.url().as_str(),
            revision,
            destination,
            max_fetch_bytes,
            max_fetch_duration,
            &configs,
            &environment,
        );
    }
    run_bounded_fetch(
        source.url().as_str(),
        revision,
        destination,
        max_fetch_bytes,
        max_fetch_duration,
        &configs,
        &environment,
    )
}

fn validated_https_address(
    source: &HttpsSource,
    resolver: &dyn HostResolver,
) -> Result<IpAddr, Error> {
    let host = source
        .url()
        .host_str()
        .ok_or_else(|| Error::InvalidSource("HTTPS URL has no host".into()))?;
    let addresses = match source.url().host() {
        Some(url::Host::Ipv4(address)) => vec![IpAddr::V4(address)],
        Some(url::Host::Ipv6(address)) => vec![IpAddr::V6(address)],
        Some(url::Host::Domain(_)) => resolver
            .resolve(host, source.url().port_or_known_default().unwrap_or(443))
            .map_err(|source| Error::DnsResolution {
                host: host.to_string(),
                source,
            })?,
        None => return Err(Error::InvalidSource("HTTPS URL has no host".into())),
    };
    if addresses.is_empty() {
        return Err(Error::Transport(format!(
            "DNS resolution returned no addresses for {host}"
        )));
    }
    if let Some(address) = addresses
        .iter()
        .find(|address| is_prohibited_address(address))
    {
        return Err(Error::ProhibitedNetworkAddress {
            host: host.to_string(),
            address: *address,
        });
    }
    addresses
        .into_iter()
        .next()
        .ok_or_else(|| Error::Transport(format!("DNS resolution returned no addresses for {host}")))
}

fn is_prohibited_address(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_prohibited_ipv4(*address),
        IpAddr::V6(address) => is_prohibited_ipv6(*address),
    }
}

fn is_prohibited_ipv4(address: Ipv4Addr) -> bool {
    const NON_GLOBAL: &[(Ipv4Addr, u8)] = &[
        (Ipv4Addr::new(0, 0, 0, 0), 8),
        (Ipv4Addr::new(10, 0, 0, 0), 8),
        (Ipv4Addr::new(100, 64, 0, 0), 10),
        (Ipv4Addr::new(127, 0, 0, 0), 8),
        (Ipv4Addr::new(169, 254, 0, 0), 16),
        (Ipv4Addr::new(172, 16, 0, 0), 12),
        (Ipv4Addr::new(192, 0, 0, 0), 24),
        (Ipv4Addr::new(192, 0, 2, 0), 24),
        (Ipv4Addr::new(192, 31, 196, 0), 24),
        (Ipv4Addr::new(192, 52, 193, 0), 24),
        (Ipv4Addr::new(192, 88, 99, 0), 24),
        (Ipv4Addr::new(192, 168, 0, 0), 16),
        (Ipv4Addr::new(192, 175, 48, 0), 24),
        (Ipv4Addr::new(198, 18, 0, 0), 15),
        (Ipv4Addr::new(198, 51, 100, 0), 24),
        (Ipv4Addr::new(203, 0, 113, 0), 24),
        (Ipv4Addr::new(224, 0, 0, 0), 4),
        (Ipv4Addr::new(240, 0, 0, 0), 4),
    ];
    NON_GLOBAL
        .iter()
        .any(|(network, prefix)| ipv4_in_prefix(address, *network, *prefix))
}

fn is_prohibited_ipv6(address: Ipv6Addr) -> bool {
    const SPECIAL_USE: &[(Ipv6Addr, u8)] = &[
        (Ipv6Addr::UNSPECIFIED, 8),
        (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23),
        (Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32),
        (Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16),
        (Ipv6Addr::new(0x2620, 0x4f, 0x8000, 0, 0, 0, 0, 0), 48),
        (Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20),
        (Ipv6Addr::new(0x5f00, 0, 0, 0, 0, 0, 0, 0), 16),
        (Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0), 7),
        (Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10),
        (Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0), 8),
    ];
    !ipv6_in_prefix(address, Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3)
        || SPECIAL_USE
            .iter()
            .any(|(network, prefix)| ipv6_in_prefix(address, *network, *prefix))
}

fn ipv4_in_prefix(address: Ipv4Addr, network: Ipv4Addr, prefix: u8) -> bool {
    let mask = u32::MAX.checked_shl(u32::from(32 - prefix)).unwrap_or(0);
    u32::from(address) & mask == u32::from(network) & mask
}

fn ipv6_in_prefix(address: Ipv6Addr, network: Ipv6Addr, prefix: u8) -> bool {
    let mask = u128::MAX.checked_shl(u32::from(128 - prefix)).unwrap_or(0);
    u128::from(address) & mask == u128::from(network) & mask
}

fn fetch_ssh(
    source: &SshSource,
    credentials: &SshCredentials,
    revision: &RequestedRevision,
    destination: &Path,
    max_fetch_bytes: u64,
    max_fetch_duration: Duration,
) -> Result<(), Error> {
    if !credentials.host().eq_ignore_ascii_case(source.host()) {
        return Err(Error::CredentialHostMismatch {
            expected: source.host().to_string(),
            actual: credentials.host().to_string(),
        });
    }
    let auth_dir = tempfile::Builder::new()
        .prefix("moltis-git-ssh-")
        .tempdir()
        .map_err(|error| Error::io(std::env::temp_dir(), error))?;
    set_directory_permissions(auth_dir.path())?;
    let key_path = auth_dir.path().join("identity");
    let known_hosts_path = auth_dir.path().join("known_hosts");
    let wrapper_path = auth_dir.path().join("ssh-wrapper");
    write_restricted(
        &key_path,
        credentials.private_key().expose_secret().as_bytes(),
        0o600,
    )?;
    write_restricted(
        &known_hosts_path,
        credentials.known_hosts().expose_secret().as_bytes(),
        0o600,
    )?;
    write_restricted(&wrapper_path, ssh_wrapper_script().as_bytes(), 0o700)?;

    let environment = [
        ("GIT_SSH", wrapper_path.as_os_str()),
        ("MOLTIS_SSH_KEY", key_path.as_os_str()),
        ("MOLTIS_KNOWN_HOSTS", known_hosts_path.as_os_str()),
    ];
    run_bounded_fetch(
        source.remote(),
        revision,
        destination,
        max_fetch_bytes,
        max_fetch_duration,
        &[],
        &environment,
    )
}

fn build_fetch_command(
    remote: &str,
    revision: &RequestedRevision,
    destination: &Path,
    configs: &[String],
    environment: &[(&str, &std::ffi::OsStr)],
) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(destination);
    for config in configs {
        command.arg("-c").arg(config);
    }
    command
        .arg("-c")
        .arg("credential.helper=")
        .arg("-c")
        .arg("http.followRedirects=false")
        .arg("fetch")
        .arg("--depth=1")
        .arg("--no-tags")
        .arg("--quiet")
        .arg("--")
        .arg(remote)
        .arg(revision.as_str())
        .env_clear()
        .env("PATH", safe_path())
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (key, value) in environment {
        command.env(key, value);
    }
    command
}

fn initialize_bare_repository(destination: &Path) -> Result<(), Error> {
    let output = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg("--quiet")
        .arg("--template=")
        .arg("--object-format=sha1")
        .arg(destination)
        .env_clear()
        .env("PATH", safe_path())
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| Error::GitSubprocess(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error(&output.stderr))
    }
}

fn run_bounded_fetch(
    remote: &str,
    revision: &RequestedRevision,
    destination: &Path,
    max_bytes: u64,
    max_duration: Duration,
    configs: &[String],
    environment: &[(&str, &std::ffi::OsStr)],
) -> Result<(), Error> {
    let started = Instant::now();
    initialize_bare_repository(destination)?;
    let mut fetch = build_fetch_command(remote, revision, destination, configs, environment);
    let mut child = fetch
        .spawn()
        .map_err(|error| Error::GitSubprocess(error.to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::GitSubprocess("failed to capture Git stderr".into()))?;
    let stderr_reader = thread::spawn(move || read_bounded_stderr(stderr));
    let Some(remaining) = max_duration.checked_sub(started.elapsed()) else {
        terminate_child(&mut child);
        return Err(Error::FetchTimeout(max_duration));
    };
    let status =
        monitor_fetch(&mut child, destination, max_bytes, remaining).map_err(
            |error| match error {
                Error::FetchTimeout(_) => Error::FetchTimeout(max_duration),
                error => error,
            },
        )?;
    let stderr = stderr_reader.join().unwrap_or_default();
    if status.success() {
        let fetched_bytes = directory_size(destination)?;
        if fetched_bytes <= max_bytes {
            return Ok(());
        }
        return Err(Error::LimitExceeded(format!(
            "fetched Git data is {fetched_bytes} bytes, limit is {max_bytes} bytes"
        )));
    }
    Err(command_error(&stderr))
}

const MAX_STDERR_BYTES: u64 = 64 * 1024;

fn read_bounded_stderr(stderr: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = stderr.take(MAX_STDERR_BYTES).read_to_end(&mut bytes);
    bytes
}

trait FetchProcess {
    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>>;
    fn kill(&mut self) -> std::io::Result<()>;
    fn wait(&mut self) -> std::io::Result<ExitStatus>;
}

impl FetchProcess for Child {
    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        Child::try_wait(self)
    }

    fn kill(&mut self) -> std::io::Result<()> {
        Child::kill(self)
    }

    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        Child::wait(self)
    }
}

fn monitor_fetch(
    child: &mut impl FetchProcess,
    destination: &Path,
    max_bytes: u64,
    max_duration: Duration,
) -> Result<ExitStatus, Error> {
    let started = Instant::now();
    loop {
        if started.elapsed() >= max_duration {
            terminate_child(child);
            return Err(Error::FetchTimeout(max_duration));
        }
        if directory_size(destination)? > max_bytes {
            terminate_child(child);
            return Err(Error::LimitExceeded(format!(
                "fetched Git data exceeds {max_bytes} bytes"
            )));
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| Error::GitSubprocess(error.to_string()))?
        {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn terminate_child(child: &mut impl FetchProcess) {
    let _ = child.kill();
    let _ = child.wait();
}

const MAX_CONCURRENT_NETWORK_FETCHES: usize = 4;

struct FetchSlots {
    max_active: usize,
    active: Mutex<usize>,
}

impl FetchSlots {
    fn new(max_active: usize) -> Self {
        Self {
            max_active,
            active: Mutex::new(0),
        }
    }

    fn acquire(&self, timeout: Duration) -> Result<FetchSlot<'_>, Error> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *active >= self.max_active {
            let _ = timeout;
            return Err(Error::OperationBusy);
        }
        *active = active.saturating_add(1);
        Ok(FetchSlot { slots: self })
    }
}

struct FetchSlot<'a> {
    slots: &'a FetchSlots,
}

impl Drop for FetchSlot<'_> {
    fn drop(&mut self) {
        let mut active = self
            .slots
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *active = active.saturating_sub(1);
    }
}

pub(crate) struct NetworkOperationGuard {
    _slot: FetchSlot<'static>,
    started: Instant,
    limit: Duration,
}

impl NetworkOperationGuard {
    pub(crate) fn acquire(limit: Duration) -> Result<Self, Error> {
        Ok(Self {
            _slot: fetch_slots().acquire(limit)?,
            started: Instant::now(),
            limit,
        })
    }

    pub(crate) fn remaining(&self) -> Result<Duration, Error> {
        self.limit
            .checked_sub(self.started.elapsed())
            .ok_or(Error::FetchTimeout(self.limit))
    }

    pub(crate) fn check(&self) -> Result<(), Error> {
        self.remaining().map(|_| ())
    }
}

fn fetch_slots() -> &'static FetchSlots {
    static FETCH_SLOTS: OnceLock<FetchSlots> = OnceLock::new();
    FETCH_SLOTS.get_or_init(|| FetchSlots::new(MAX_CONCURRENT_NETWORK_FETCHES))
}

fn directory_size(root: &Path) -> Result<u64, Error> {
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0_u64;
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(&path).map_err(|error| Error::io(&path, error))? {
            let entry = entry.map_err(|error| Error::io(&path, error))?;
            let metadata = entry
                .metadata()
                .map_err(|error| Error::io(entry.path(), error))?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

fn command_error(stderr: &[u8]) -> Error {
    let stderr = String::from_utf8_lossy(stderr);
    Error::GitSubprocess(
        stderr
            .lines()
            .next()
            .unwrap_or("git fetch failed")
            .to_string(),
    )
}

fn curl_resolve_address(address: IpAddr) -> String {
    match address {
        IpAddr::V4(address) => address.to_string(),
        IpAddr::V6(address) => format!("[{address}]"),
    }
}

fn https_askpass_script() -> &'static str {
    "#!/bin/sh\ncase \"$1\" in\n  *sername*) test -n \"$MOLTIS_GIT_USERNAME_FILE\" && cat \"$MOLTIS_GIT_USERNAME_FILE\" ;;\n  *) test -n \"$MOLTIS_GIT_TOKEN_FILE\" && cat \"$MOLTIS_GIT_TOKEN_FILE\" ;;\nesac\n"
}

fn ssh_wrapper_script() -> &'static str {
    "#!/bin/sh\nexec ssh -F /dev/null -o BatchMode=yes -o IdentitiesOnly=yes -o IdentityAgent=none -o StrictHostKeyChecking=yes -o GlobalKnownHostsFile=/dev/null -o UserKnownHostsFile=\"$MOLTIS_KNOWN_HOSTS\" -i \"$MOLTIS_SSH_KEY\" \"$@\"\n"
}

fn write_restricted(path: &Path, contents: &[u8], mode: u32) -> Result<(), Error> {
    let mut file = File::create(path).map_err(|error| Error::io(path, error))?;
    file.write_all(contents)
        .map_err(|error| Error::io(path, error))?;
    file.flush().map_err(|error| Error::io(path, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|error| Error::io(path, error))?;
    }
    Ok(())
}

fn set_directory_permissions(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| Error::io(path, error))?;
    }
    Ok(())
}

fn safe_path() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "/usr/bin:/bin:/usr/sbin:/sbin"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "/usr/bin:/bin"
    }
}

fn null_device() -> &'static str {
    #[cfg(windows)]
    {
        "NUL"
    }
    #[cfg(not(windows))]
    {
        "/dev/null"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Mutex;

    use {super::*, crate::Access};

    struct FakeResolver {
        addresses: Vec<IpAddr>,
        calls: Mutex<Vec<(String, u16)>>,
    }

    impl FakeResolver {
        fn new(addresses: &[&str]) -> Self {
            Self {
                addresses: addresses
                    .iter()
                    .map(|address| address.parse().unwrap())
                    .collect(),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl HostResolver for FakeResolver {
        fn resolve(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>> {
            self.calls.lock().unwrap().push((host.to_string(), port));
            Ok(self.addresses.clone())
        }
    }

    #[test]
    fn ssh_command_and_wrapper_are_fixed_and_non_interpolating() {
        let source = SshSource::new("git@example.com:owner/repo.git", Access::Private).unwrap();
        let revision = RequestedRevision::parse("main").unwrap();
        let environment = [
            ("GIT_SSH", std::ffi::OsStr::new("/tmp/wrapper")),
            ("MOLTIS_SSH_KEY", std::ffi::OsStr::new("/tmp/key")),
            (
                "MOLTIS_KNOWN_HOSTS",
                std::ffi::OsStr::new("/tmp/known hosts"),
            ),
        ];
        let command = build_fetch_command(
            source.remote(),
            &revision,
            Path::new("/tmp/out;touch-pwned"),
            &[],
            &environment,
        );
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args.last().map(String::as_str), Some("main"));
        assert!(args.contains(&"--".to_string()));
        assert!(args.contains(&"--depth=1".to_string()));
        assert!(
            args.windows(2)
                .any(|args| args == [source.remote(), revision.as_str()])
        );
        assert!(args.contains(&"/tmp/out;touch-pwned".to_string()));
        assert!(!ssh_wrapper_script().contains(source.remote()));
        assert!(ssh_wrapper_script().contains("IdentityAgent=none"));
        assert!(ssh_wrapper_script().contains("StrictHostKeyChecking=yes"));
        assert!(ssh_wrapper_script().contains("\"$@\""));
    }

    #[test]
    fn non_global_ip_literals_are_rejected_without_dns() {
        let resolver = FakeResolver::new(&["8.8.8.8"]);
        for url in [
            "https://0.1.2.3/owner/repo.git",
            "https://127.0.0.1/owner/repo.git",
            "https://10.0.0.1/owner/repo.git",
            "https://100.64.0.1/owner/repo.git",
            "https://192.0.0.8/owner/repo.git",
            "https://192.0.2.1/owner/repo.git",
            "https://192.31.196.1/owner/repo.git",
            "https://192.52.193.1/owner/repo.git",
            "https://192.88.99.1/owner/repo.git",
            "https://192.175.48.1/owner/repo.git",
            "https://198.18.0.1/owner/repo.git",
            "https://198.51.100.1/owner/repo.git",
            "https://203.0.113.1/owner/repo.git",
            "https://224.0.0.1/owner/repo.git",
            "https://250.1.2.3/owner/repo.git",
            "https://[::1]/owner/repo.git",
            "https://[::ffff:192.168.1.1]/owner/repo.git",
            "https://[100::1]/owner/repo.git",
            "https://[2001:2::1]/owner/repo.git",
            "https://[2001:20::1]/owner/repo.git",
            "https://[2001:db8::1]/owner/repo.git",
            "https://[2002::1]/owner/repo.git",
            "https://[2620:4f:8000::1]/owner/repo.git",
            "https://[3fff::1]/owner/repo.git",
            "https://[5f00::1]/owner/repo.git",
            "https://[fc00::1]/owner/repo.git",
            "https://[fe80::1]/owner/repo.git",
            "https://[ff0e::1]/owner/repo.git",
        ] {
            let source = HttpsSource::new(url, Access::Public).unwrap();
            assert!(matches!(
                validated_https_address(&source, &resolver),
                Err(Error::ProhibitedNetworkAddress { .. })
            ));
        }
        assert!(resolver.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn localhost_and_private_dns_answers_are_rejected() {
        let resolver = FakeResolver::new(&["127.0.0.1"]);
        let localhost =
            HttpsSource::new("https://localhost/owner/repo.git", Access::Public).unwrap();
        assert!(matches!(
            validated_https_address(&localhost, &resolver),
            Err(Error::ProhibitedNetworkAddress { .. })
        ));
        assert_eq!(*resolver.calls.lock().unwrap(), vec![(
            "localhost".to_string(),
            443
        )]);

        let mixed = FakeResolver::new(&["8.8.8.8", "fe80::1"]);
        let source =
            HttpsSource::new("https://git.example/owner/repo.git", Access::Public).unwrap();
        assert!(matches!(
            validated_https_address(&source, &mixed),
            Err(Error::ProhibitedNetworkAddress { .. })
        ));
    }

    #[test]
    fn public_dns_answers_are_accepted() {
        let resolver = FakeResolver::new(&["8.8.8.8", "2606:4700:4700::1111"]);
        let source =
            HttpsSource::new("https://git.example:8443/owner/repo.git", Access::Public).unwrap();
        assert_eq!(
            validated_https_address(&source, &resolver).unwrap(),
            "8.8.8.8".parse::<IpAddr>().unwrap()
        );
        assert_eq!(*resolver.calls.lock().unwrap(), vec![(
            "git.example".to_string(),
            8443
        )]);
    }

    #[test]
    fn representative_public_addresses_are_accepted() {
        for address in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "2001:4860:4860::8888",
            "2606:4700:4700::1111",
        ] {
            let address = address.parse::<IpAddr>().unwrap();
            assert!(!is_prohibited_address(&address), "{address}");
        }
    }

    #[test]
    fn special_use_range_boundaries_are_prohibited() {
        for address in [
            "0.0.0.0",
            "0.255.255.255",
            "100.64.0.0",
            "100.127.255.255",
            "192.0.0.0",
            "192.0.0.255",
            "192.0.2.0",
            "192.0.2.255",
            "198.18.0.0",
            "198.19.255.255",
            "198.51.100.0",
            "198.51.100.255",
            "203.0.113.0",
            "203.0.113.255",
            "224.0.0.0",
            "239.255.255.255",
            "240.0.0.0",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:8.8.8.8",
            "64:ff9b::1",
            "64:ff9b:1::1",
            "100::1",
            "2001:2::1",
            "2001:10::1",
            "2001:20::1",
            "2001:db8::1",
            "2002::1",
            "2620:4f:8000::1",
            "3fff:fff::1",
            "5f00::1",
            "fc00::1",
            "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            "fe80::1",
            "febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            "ff00::1",
        ] {
            let address = address.parse::<IpAddr>().unwrap();
            assert!(is_prohibited_address(&address), "{address}");
        }
    }

    #[test]
    fn https_transport_disables_redirects() {
        let revision = RequestedRevision::parse("main").unwrap();
        let command = build_fetch_command(
            "https://git.example/owner/repo.git",
            &revision,
            Path::new("/tmp/repo"),
            &["http.curloptResolve=git.example:443:8.8.8.8".to_string()],
            &[],
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.contains(&"http.followRedirects=false".to_string()));
        assert!(args.contains(&"http.curloptResolve=git.example:443:8.8.8.8".to_string()));
    }

    #[derive(Default)]
    struct PendingProcess {
        killed: bool,
        waited: bool,
    }

    impl FetchProcess for PendingProcess {
        fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
            Ok(None)
        }

        fn kill(&mut self) -> std::io::Result<()> {
            self.killed = true;
            Ok(())
        }

        fn wait(&mut self) -> std::io::Result<ExitStatus> {
            self.waited = true;
            Err(std::io::Error::other("fake process has no exit status"))
        }
    }

    #[test]
    fn fetch_deadline_kills_and_reaps_child_without_waiting() {
        let destination = tempfile::tempdir().unwrap();
        let mut process = PendingProcess::default();

        let result = monitor_fetch(&mut process, destination.path(), u64::MAX, Duration::ZERO);

        assert!(matches!(result, Err(Error::FetchTimeout(duration)) if duration.is_zero()));
        assert!(process.killed);
        assert!(process.waited);
    }

    #[test]
    fn fetch_slots_are_bounded_and_fail_fast() {
        let slots = FetchSlots::new(1);
        let slot = slots.acquire(Duration::from_secs(1)).unwrap();

        let result = slots.acquire(Duration::ZERO);

        assert!(matches!(result, Err(Error::OperationBusy)));
        drop(slot);
        assert!(slots.acquire(Duration::ZERO).is_ok());
    }
}
