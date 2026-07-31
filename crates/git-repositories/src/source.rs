use std::{fmt, path::PathBuf};

use {
    gix::bstr::ByteSlice,
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    url::Url,
};

use crate::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Public,
    Private,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpsSource {
    url: Url,
    authority: String,
    access: Access,
}

impl HttpsSource {
    pub fn new(value: &str, access: Access) -> Result<Self, Error> {
        let url = Url::parse(value).map_err(|error| Error::InvalidSource(error.to_string()))?;
        if url.scheme() != "https" {
            return Err(Error::InvalidSource("only HTTPS URLs are allowed".into()));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Error::InvalidSource("URL userinfo is not allowed".into()));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(Error::InvalidSource(
                "URL query strings and fragments are not allowed".into(),
            ));
        }
        let host = url
            .host_str()
            .ok_or_else(|| Error::InvalidSource("HTTPS URL has no host".into()))?;
        if url.path().is_empty() || url.path() == "/" {
            return Err(Error::InvalidSource(
                "HTTPS URL has no repository path".into(),
            ));
        }
        let host = match url.host() {
            Some(url::Host::Ipv6(address)) => format!("[{address}]"),
            Some(_) => host.to_string(),
            None => return Err(Error::InvalidSource("HTTPS URL has no host".into())),
        };
        let authority = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host,
        };
        Ok(Self {
            url,
            authority,
            access,
        })
    }

    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    #[must_use]
    pub fn authority(&self) -> &str {
        &self.authority
    }

    #[must_use]
    pub fn access(&self) -> Access {
        self.access
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshSource {
    remote: String,
    host: String,
    access: Access,
}

impl SshSource {
    pub fn new(value: &str, access: Access) -> Result<Self, Error> {
        let host = if value.starts_with("ssh://") {
            validate_ssh_url(value)?
        } else {
            validate_scp_remote(value)?
        };
        Ok(Self {
            remote: value.to_string(),
            host,
            access,
        })
    }

    #[must_use]
    pub fn remote(&self) -> &str {
        &self.remote
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub fn access(&self) -> Access {
        self.access
    }
}

fn validate_ssh_url(value: &str) -> Result<String, Error> {
    let url = Url::parse(value).map_err(|error| Error::InvalidSource(error.to_string()))?;
    if url.scheme() != "ssh" || url.password().is_some() {
        return Err(Error::InvalidSource("invalid SSH URL".into()));
    }
    validate_ssh_user(url.username())?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(Error::InvalidSource(
            "URL query strings and fragments are not allowed".into(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::InvalidSource("SSH URL has no host".into()))?;
    validate_ssh_host(host)?;
    if url.path().is_empty() || url.path() == "/" {
        return Err(Error::InvalidSource(
            "SSH URL has no repository path".into(),
        ));
    }
    validate_ssh_path(url.path())?;
    let (host, bracket_for_port) = match url.host() {
        Some(url::Host::Ipv6(address)) => (address.to_string(), true),
        Some(_) => (host.to_ascii_lowercase(), false),
        None => return Err(Error::InvalidSource("SSH URL has no host".into())),
    };
    Ok(match url.port() {
        Some(port) => format!("[{host}]:{port}"),
        None if bracket_for_port => format!("[{host}]"),
        None => host,
    })
}

fn validate_scp_remote(value: &str) -> Result<String, Error> {
    if value.chars().any(char::is_whitespace) || value.starts_with('-') {
        return Err(Error::InvalidSource("invalid SCP-like SSH remote".into()));
    }
    let (destination, path) = value
        .split_once(':')
        .ok_or_else(|| Error::InvalidSource("SCP-like SSH remote has no path".into()))?;
    if destination.is_empty() || path.is_empty() || path.starts_with('/') && path == "/" {
        return Err(Error::InvalidSource("invalid SCP-like SSH remote".into()));
    }
    let (user, host) = destination
        .rsplit_once('@')
        .map_or(("", destination), |(user, host)| (user, host));
    validate_ssh_user(user)?;
    validate_ssh_host(host)?;
    validate_ssh_path(path)?;
    Ok(host.to_ascii_lowercase())
}

fn validate_ssh_path(path: &str) -> Result<(), Error> {
    if path
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"/._-~".contains(&byte))
    {
        return Ok(());
    }
    Err(Error::InvalidSource("invalid SSH repository path".into()))
}

fn validate_ssh_user(user: &str) -> Result<(), Error> {
    if user
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Ok(());
    }
    Err(Error::InvalidSource("invalid SSH username".into()))
}

fn validate_ssh_host(host: &str) -> Result<(), Error> {
    if host.is_empty()
        || host.starts_with('-')
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".:-[]".contains(&byte))
    {
        return Err(Error::InvalidSource("invalid SSH host".into()));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepositorySource {
    Https(HttpsSource),
    Ssh(SshSource),
    Local(LocalSource),
}

impl RepositorySource {
    pub fn local(path: impl Into<PathBuf>) -> Result<Self, Error> {
        Ok(Self::Local(LocalSource::new(path)?))
    }

    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Https(_) => "https",
            Self::Ssh(_) => "ssh",
            Self::Local(_) => "local",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSource {
    path: PathBuf,
}

impl LocalSource {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, Error> {
        let path = path.into();
        let canonical = std::fs::canonicalize(&path).map_err(|error| Error::io(&path, error))?;
        gix::open_opts(&canonical, gix::open::Options::isolated()).map_err(|source| {
            Error::OpenRepository {
                path: canonical.clone(),
                source: Box::new(source),
            }
        })?;
        Ok(Self { path: canonical })
    }

    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestedRevision {
    value: String,
}

impl RequestedRevision {
    pub fn parse(value: &str) -> Result<Self, Error> {
        if value == "HEAD" {
            return Ok(Self::head());
        }
        if value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(Self {
                value: value.to_ascii_lowercase(),
            });
        }
        gix::validate::reference::name_partial(value.as_bytes().as_bstr())
            .map_err(|error| Error::InvalidRevision(error.to_string()))?;
        Ok(Self {
            value: value.to_string(),
        })
    }

    #[must_use]
    pub fn head() -> Self {
        Self {
            value: "HEAD".to_string(),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

#[derive(Clone)]
pub struct HttpsCredentials {
    host: String,
    username: String,
    token: Secret<String>,
}

impl HttpsCredentials {
    pub fn new(
        host: impl Into<String>,
        username: impl Into<String>,
        token: Secret<String>,
    ) -> Result<Self, Error> {
        let host = host.into();
        let username = username.into();
        if host.is_empty()
            || host.starts_with('-')
            || !host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".:-[]".contains(&byte))
            || username.is_empty()
            || username.bytes().any(|byte| byte.is_ascii_control())
            || token.expose_secret().is_empty()
        {
            return Err(Error::InvalidSource("invalid HTTPS credentials".into()));
        }
        Ok(Self {
            host: host.to_ascii_lowercase(),
            username,
            token,
        })
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    #[must_use]
    pub fn token(&self) -> &Secret<String> {
        &self.token
    }
}

impl fmt::Debug for HttpsCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpsCredentials")
            .field("host", &self.host)
            .field("username", &self.username)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone)]
pub struct SshCredentials {
    host: String,
    private_key: Secret<String>,
    known_hosts: Secret<String>,
}

impl SshCredentials {
    pub fn new(
        host: impl Into<String>,
        private_key: Secret<String>,
        known_hosts: Secret<String>,
    ) -> Result<Self, Error> {
        let host = host.into().to_ascii_lowercase();
        validate_ssh_host(&host)?;
        if private_key.expose_secret().trim().is_empty()
            || known_hosts.expose_secret().lines().all(|line| {
                let line = line.trim();
                line.is_empty() || line.starts_with('#')
            })
            || known_hosts.expose_secret().contains('\0')
        {
            return Err(Error::InvalidSource(
                "SSH credentials require a private key and known_hosts entry".into(),
            ));
        }
        Ok(Self {
            host,
            private_key,
            known_hosts,
        })
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub fn private_key(&self) -> &Secret<String> {
        &self.private_key
    }

    #[must_use]
    pub fn known_hosts(&self) -> &Secret<String> {
        &self.known_hosts
    }
}

impl fmt::Debug for SshCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SshCredentials")
            .field("host", &self.host)
            .field("private_key", &"[REDACTED]")
            .field("known_hosts", &"[REDACTED]")
            .finish()
    }
}
