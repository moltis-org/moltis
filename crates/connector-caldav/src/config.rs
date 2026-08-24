use std::{collections::HashSet, fmt, net::SocketAddr};

use {
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    time::{Date, format_description::well_known::Iso8601},
    url::Url,
};

use crate::{CalDavConnectorError, Result};

const fn default_timeout_seconds() -> u64 {
    30
}

#[derive(Clone, Deserialize)]
pub struct CalDavAccountConfig {
    pub server_url: String,
    pub username: String,
    pub password: Secret<String>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub allow_insecure_http: bool,
    #[serde(default)]
    pub allow_private_network: bool,
}

impl fmt::Debug for CalDavAccountConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CalDavAccountConfig")
            .field("server_url", &"[REDACTED]")
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .field("timeout_seconds", &self.timeout_seconds)
            .field("allow_insecure_http", &self.allow_insecure_http)
            .field("allow_private_network", &self.allow_private_network)
            .finish()
    }
}

impl CalDavAccountConfig {
    pub fn validate(&self) -> Result<Url> {
        if self.password.expose_secret().is_empty() {
            return Err(CalDavConnectorError::AccountConfig(
                "password must not be empty",
            ));
        }
        Self::validate_non_secret_fields(
            &self.server_url,
            &self.username,
            self.timeout_seconds,
            self.allow_insecure_http,
        )
    }

    /// Validates account fields without requiring access to the password.
    pub fn validate_non_secret_fields(
        server_url: &str,
        username: &str,
        timeout_seconds: u64,
        allow_insecure_http: bool,
    ) -> Result<Url> {
        if server_url.trim().is_empty() {
            return Err(CalDavConnectorError::AccountConfig(
                "server_url must not be empty",
            ));
        }
        if username.trim().is_empty() {
            return Err(CalDavConnectorError::AccountConfig(
                "username must not be empty",
            ));
        }
        if !(1..=300).contains(&timeout_seconds) {
            return Err(CalDavConnectorError::AccountConfig(
                "timeout_seconds must be between 1 and 300",
            ));
        }

        let url = Url::parse(server_url)
            .map_err(|_| CalDavConnectorError::AccountConfig("server_url must be a valid URL"))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(CalDavConnectorError::AccountConfig(
                "server_url must not contain credentials",
            ));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(CalDavConnectorError::AccountConfig(
                "server_url must not contain a query string or fragment",
            ));
        }
        if url.host_str().is_none() {
            return Err(CalDavConnectorError::AccountConfig(
                "server_url must include a host",
            ));
        }
        match url.scheme() {
            "https" => {},
            "http" if allow_insecure_http => {},
            "http" => {
                return Err(CalDavConnectorError::AccountConfig(
                    "server_url must use HTTPS unless insecure HTTP is explicitly allowed",
                ));
            },
            _ => {
                return Err(CalDavConnectorError::AccountConfig(
                    "server_url must use HTTP or HTTPS",
                ));
            },
        }

        Ok(url)
    }

    pub async fn validate_connection_target(&self) -> Result<Url> {
        self.resolve_connection_target().await.map(|(url, _)| url)
    }

    pub(crate) async fn resolve_connection_target(&self) -> Result<(Url, Vec<SocketAddr>)> {
        let url = self.validate()?;
        let host = url.host_str().ok_or(CalDavConnectorError::AccountConfig(
            "server_url must include a host",
        ))?;
        let port = url.port_or_known_default().unwrap_or(443);
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| {
                CalDavConnectorError::NetworkSafety(moltis_common::Error::message(format!(
                    "DNS resolution failed for {host}: {error}"
                )))
            })?
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(CalDavConnectorError::NetworkSafety(
                moltis_common::Error::message(format!("DNS resolution failed for {host}")),
            ));
        }
        if !self.allow_private_network
            && let Some(address) = addresses
                .iter()
                .find(|address| moltis_common::ssrf::is_private_ip(&address.ip()))
        {
            return Err(CalDavConnectorError::NetworkSafety(
                moltis_common::Error::message(format!(
                    "SSRF blocked: {host} resolves to non-global IP {}",
                    address.ip()
                )),
            ));
        }
        Ok((url, addresses))
    }

    #[must_use]
    pub fn view(&self) -> CalDavAccountConfigView {
        self.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalDavAccountConfigView {
    pub server_url: String,
    pub username: String,
    pub timeout_seconds: u64,
    pub allow_insecure_http: bool,
    pub allow_private_network: bool,
    pub has_password: bool,
}

impl From<&CalDavAccountConfig> for CalDavAccountConfigView {
    fn from(config: &CalDavAccountConfig) -> Self {
        Self {
            server_url: config.server_url.clone(),
            username: config.username.clone(),
            timeout_seconds: config.timeout_seconds,
            allow_insecure_http: config.allow_insecure_http,
            allow_private_network: config.allow_private_network,
            has_password: !config.password.expose_secret().is_empty(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum CalendarSelection {
    #[default]
    All,
    Selected {
        #[serde(rename = "calendarHrefs", alias = "calendar_hrefs")]
        calendar_hrefs: Vec<String>,
    },
}

const fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalDavFilters {
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub accepted_by_account: bool,
}

impl CalDavFilters {
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.start_date.is_some() || self.end_date.is_some() || self.accepted_by_account
    }

    pub(crate) fn date_bounds(&self) -> Result<(Option<Date>, Option<Date>)> {
        let start = self
            .start_date
            .as_deref()
            .map(|value| parse_iso_date(value, "startDate must be an ISO date (YYYY-MM-DD)"))
            .transpose()?;
        let end = self
            .end_date
            .as_deref()
            .map(|value| parse_iso_date(value, "endDate must be an ISO date (YYYY-MM-DD)"))
            .transpose()?;
        if start.zip(end).is_some_and(|(start, end)| start >= end) {
            return Err(CalDavConnectorError::DatasetConfig(
                "startDate must be before endDate",
            ));
        }
        Ok((start, end))
    }
}

fn parse_iso_date(value: &str, message: &'static str) -> Result<Date> {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return Err(CalDavConnectorError::DatasetConfig(message));
    }
    Date::parse(value, &Iso8601::DEFAULT).map_err(|_| CalDavConnectorError::DatasetConfig(message))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalDavDatasetConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub selection: CalendarSelection,
    #[serde(default)]
    pub filters: CalDavFilters,
}

impl Default for CalDavDatasetConfig {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            selection: CalendarSelection::default(),
            filters: CalDavFilters::default(),
        }
    }
}

impl CalDavDatasetConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != default_schema_version() {
            return Err(CalDavConnectorError::DatasetConfig(
                "schemaVersion must be 1",
            ));
        }
        self.filters.date_bounds()?;
        let CalendarSelection::Selected { calendar_hrefs } = &self.selection else {
            return Ok(());
        };
        if calendar_hrefs.is_empty() {
            return Err(CalDavConnectorError::DatasetConfig(
                "selected calendars must not be empty",
            ));
        }
        if calendar_hrefs.iter().any(String::is_empty) {
            return Err(CalDavConnectorError::DatasetConfig(
                "calendar hrefs must not be empty",
            ));
        }
        let unique = calendar_hrefs.iter().collect::<HashSet<_>>();
        if unique.len() != calendar_hrefs.len() {
            return Err(CalDavConnectorError::DatasetConfig(
                "calendar hrefs must be exactly deduplicated",
            ));
        }
        Ok(())
    }
}
