//! Home Assistant REST API client.

use std::time::Duration;

use reqwest::Client;
use secrecy::ExposeSecret;

use crate::config::HomeAssistantAccountConfig;
use crate::error::{Error, Result};
use crate::types::{EntityState, HaConfigResponse, ServiceDescription, Target};

/// Extract the domain portion of an entity ID (part before the first `.`).
fn extract_domain(entity_id: &str) -> &str {
    entity_id.split('.').next().unwrap_or("homeassistant")
}

/// Check a response status and return an auth error for 401/403.
fn check_auth_status(status: reqwest::StatusCode) -> Result<()> {
    match status.as_u16() {
        401 => Err(Error::Auth("invalid or expired access token (401)".to_owned())),
        403 => Err(Error::Auth("insufficient permissions (403)".to_owned())),
        _ => Ok(()),
    }
}

/// REST API client for a single Home Assistant instance.
pub struct HomeAssistantClient {
    base_url: String,
    token: String,
    http: Client,
}

impl HomeAssistantClient {
    /// Build a client from account config.
    pub fn new(account: &HomeAssistantAccountConfig) -> Result<Self> {
        let url = account
            .url
            .as_deref()
            .ok_or_else(|| Error::Config("account has no url".to_owned()))?;

        let token = account
            .token
            .as_ref()
            .ok_or_else(|| Error::Config("account has no token".to_owned()))?;

        let timeout = Duration::from_secs(account.timeout_seconds);
        let http = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Error::Client(e.to_string()))?;

        Ok(Self {
            base_url: url.trim_end_matches('/').to_owned(),
            token: token.expose_secret().to_owned(),
            http,
        })
    }

    fn auth_header(&self) -> (&str, String) {
        ("Authorization", format!("Bearer {}", self.token))
    }

    /// Check if the HA instance is reachable and the token is valid.
    ///
    /// Hits the authenticated `GET /api/config` endpoint.
    pub async fn health_check(&self) -> Result<()> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/config", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            check_auth_status(status)?;
            Err(Error::Connection(format!("HA returned status {status}")))
        }
    }

    /// Fetch the HA instance configuration.
    pub async fn get_config(&self) -> Result<HaConfigResponse> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/config", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        resp.json().await.map_err(Error::from)
    }

    /// Get all entity states.
    pub async fn get_states(&self) -> Result<Vec<EntityState>> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/states", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        resp.json().await.map_err(Error::from)
    }

    /// Get a single entity state. Returns `None` if entity not found (404).
    pub async fn get_state(&self, entity_id: &str) -> Result<Option<EntityState>> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/states/{entity_id}", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        if resp.status().as_u16() == 404 {
            return Ok(None);
        }

        let status = resp.status();
        check_auth_status(status)?;
        resp.json().await.map_err(Error::from).map(Some)
    }

    /// Get all registered services.
    pub async fn get_services(&self) -> Result<Vec<ServiceDescription>> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/services", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        resp.json().await.map_err(Error::from)
    }

    /// Call a service.
    ///
    /// The `data` is passed directly as the JSON body — HA parses it as
    /// `service_data`. Use the `target` field to target by area, device,
    /// or label instead of listing individual entity IDs.
    pub async fn call_service(
        &self,
        domain: &str,
        service: &str,
        target: Option<&Target>,
        data: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let (_, auth) = self.auth_header();

        let mut body = serde_json::Map::new();
        if let Some(t) = target {
            body.insert("target".to_owned(), serde_json::to_value(t)?);
        }
        if let Some(d) = data {
            // HA expects service_data as a JSON object merged into the body.
            if let serde_json::Value::Object(map) = d {
                for (k, v) in map {
                    body.insert(k, v);
                }
            } else {
                return Err(Error::Client(
                    "service data must be a JSON object".to_owned(),
                ));
            }
        }

        let resp = self
            .http
            .post(format!(
                "{}/api/services/{domain}/{service}",
                self.base_url
            ))
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::ServiceCall(format!(
                "service {domain}.{service} returned {status}: {text}"
            )));
        }

        resp.json().await.map_err(Error::from)
    }

    /// Turn an entity on.
    pub async fn turn_on(&self, entity_id: &str) -> Result<()> {
        let domain = extract_domain(entity_id);
        let target = Target::entity(entity_id);
        self.call_service(domain, "turn_on", Some(&target), None)
            .await?;
        Ok(())
    }

    /// Turn an entity off.
    pub async fn turn_off(&self, entity_id: &str) -> Result<()> {
        let domain = extract_domain(entity_id);
        let target = Target::entity(entity_id);
        self.call_service(domain, "turn_off", Some(&target), None)
            .await?;
        Ok(())
    }

    /// Toggle an entity.
    pub async fn toggle(&self, entity_id: &str) -> Result<()> {
        let domain = extract_domain(entity_id);
        let target = Target::entity(entity_id);
        self.call_service(domain, "toggle", Some(&target), None)
            .await?;
        Ok(())
    }

    /// Fire a custom event on the HA event bus.
    pub async fn fire_event(
        &self,
        event_type: &str,
        event_data: Option<serde_json::Value>,
    ) -> Result<()> {
        let (_, auth) = self.auth_header();
        let mut body = serde_json::Map::new();
        if let Some(d) = event_data {
            body.insert("event_data".to_owned(), d);
        }

        let resp = self
            .http
            .post(format!(
                "{}/api/events/{event_type}",
                self.base_url
            ))
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Event(format!(
                "fire_event {event_type} returned {status}: {text}"
            )));
        }

        Ok(())
    }

    /// Fetch state history for entities within a time range.
    ///
    /// All URL parameters are percent-encoded to handle ISO 8601 timestamps
    /// and entity IDs with special characters.
    pub async fn get_history(
        &self,
        filter_entity_id: &str,
        start_time: &str,
        end_time: Option<&str>,
    ) -> Result<Vec<serde_json::Value>> {
        let (_, auth) = self.auth_header();
        let mut url = format!(
            "{}/api/history/period/{}?filter_entity_id={}",
            self.base_url,
            urlencoding::encode(start_time),
            urlencoding::encode(filter_entity_id),
        );
        if let Some(end) = end_time {
            url.push_str(&format!("&end_time={}", urlencoding::encode(end)));
        }

        let resp = self
            .http
            .get(&url)
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        resp.json().await.map_err(Error::from)
    }

    /// Fetch camera proxy image bytes for a camera entity.
    pub async fn camera_proxy(&self, entity_id: &str) -> Result<bytes::Bytes> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!(
                "{}/api/camera_proxy/{entity_id}",
                self.base_url
            ))
            .header("Authorization", &auth)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Camera(format!(
                "camera_proxy {entity_id} returned {status}: {text}"
            )));
        }

        resp.bytes().await.map_err(Error::from)
    }
}
