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
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
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
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
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
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
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
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
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
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
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
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
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
    ///
    /// Set `return_response` to `true` for services that return data
    /// (e.g. `weather.get_forecasts`). The response will include
    /// `changed_states` and `service_response` fields.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(domain, service)))]
    pub async fn call_service(
        &self,
        domain: &str,
        service: &str,
        target: Option<&Target>,
        data: Option<serde_json::Value>,
        return_response: bool,
    ) -> Result<serde_json::Value> {
        let (_, auth) = self.auth_header();

        let mut body = serde_json::Map::new();
        if let Some(t) = target {
            // HA REST API expects target fields flattened at top level, not nested
            // under a "target" key (that format is for WebSocket only).
            if !t.entity_id.is_empty() {
                body.insert(
                    "entity_id".to_owned(),
                    serde_json::to_value(&t.entity_id)?,
                );
            }
            if !t.device_id.is_empty() {
                body.insert(
                    "device_id".to_owned(),
                    serde_json::to_value(&t.device_id)?,
                );
            }
            if !t.area_id.is_empty() {
                body.insert(
                    "area_id".to_owned(),
                    serde_json::to_value(&t.area_id)?,
                );
            }
            if !t.label_id.is_empty() {
                body.insert(
                    "label_id".to_owned(),
                    serde_json::to_value(&t.label_id)?,
                );
            }
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

        let mut url = format!(
            "{}/api/services/{domain}/{service}",
            self.base_url
        );
        if return_response {
            url.push_str("?return_response");
        }

        let resp = self
            .http
            .post(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            #[cfg(feature = "tracing")]
            tracing::warn!(
                domain,
                service,
                status = %status,
                "service call failed"
            );
            return Err(Error::ServiceCall(format!(
                "service {domain}.{service} returned {status}: {text}"
            )));
        }

        resp.json().await.map_err(Error::from)
    }

    /// Turn an entity on.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(entity_id)))]
    pub async fn turn_on(&self, entity_id: &str) -> Result<()> {
        let domain = extract_domain(entity_id);
        let target = Target::entity(entity_id);
        self.call_service(domain, "turn_on", Some(&target), None, false)
            .await?;
        Ok(())
    }

    /// Turn an entity off.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(entity_id)))]
    pub async fn turn_off(&self, entity_id: &str) -> Result<()> {
        let domain = extract_domain(entity_id);
        let target = Target::entity(entity_id);
        self.call_service(domain, "turn_off", Some(&target), None, false)
            .await?;
        Ok(())
    }

    /// Toggle an entity.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(entity_id)))]
    pub async fn toggle(&self, entity_id: &str) -> Result<()> {
        let domain = extract_domain(entity_id);
        let target = Target::entity(entity_id);
        self.call_service(domain, "toggle", Some(&target), None, false)
            .await?;
        Ok(())
    }

    /// Fire a custom event on the HA event bus.
    ///
    /// The `event_data` value is sent as the raw JSON body per the HA REST API spec.
    /// Pass `None` to fire with no data payload.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(event_type)))]
    pub async fn fire_event(
        &self,
        event_type: &str,
        event_data: Option<serde_json::Value>,
    ) -> Result<()> {
        let (_, auth) = self.auth_header();
        let body = event_data.unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

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

    /// Set or create an entity state.
    ///
    /// Sends `POST /api/states/{entity_id}` with `state` and optional
    /// `attributes`. Returns the new [`EntityState`] from the server.
    ///
    /// Requires admin privileges in Home Assistant.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(entity_id)))]
    pub async fn set_state(
        &self,
        entity_id: &str,
        state: &str,
        attributes: Option<serde_json::Value>,
    ) -> Result<EntityState> {
        let (_, auth) = self.auth_header();

        let mut body = serde_json::Map::new();
        body.insert("state".to_owned(), serde_json::Value::String(state.to_owned()));
        if let Some(attrs) = attributes {
            body.insert("attributes".to_owned(), attrs);
        }

        let resp = self
            .http
            .post(format!(
                "{}/api/states/{entity_id}",
                self.base_url
            ))
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::ServiceCall(format!(
                "set_state {entity_id} returned {status}: {text}"
            )));
        }

        resp.json().await.map_err(Error::from)
    }

    /// Fetch logbook entries.
    ///
    /// Returns a chronological list of logbook entries (state changes,
    /// automations triggered, etc.) for the given time window.
    ///
    /// - `timestamp`: ISO 8601 datetime for the start of the period.
    ///   If `None`, defaults to the start of the current local day.
    /// - `end_time`: Optional ISO 8601 datetime for the end of the period.
    /// - `entity_id`: Optional entity to filter entries for.
    /// - `period`: Number of days to include (default 1). Ignored if
    ///   `end_time` is provided.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(entity_id)))]
    pub async fn get_logbook(
        &self,
        timestamp: Option<&str>,
        end_time: Option<&str>,
        entity_id: Option<&str>,
        period: Option<u32>,
    ) -> Result<Vec<serde_json::Value>> {
        let (_, auth) = self.auth_header();

        let url = if let Some(ts) = timestamp {
            format!(
                "{}/api/logbook/{}",
                self.base_url,
                urlencoding::encode(ts),
            )
        } else {
            format!("{}/api/logbook", self.base_url)
        };

        let mut query = Vec::new();
        if let Some(end) = end_time {
            query.push(format!("end_time={}", urlencoding::encode(end)));
        }
        if let Some(entity) = entity_id {
            query.push(format!("entity={}", urlencoding::encode(entity)));
        }
        if let Some(p) = period {
            query.push(format!("period={p}"));
        }

        let full_url = if query.is_empty() {
            url
        } else {
            format!("{url}?{}", query.join("&"))
        };

        let resp = self
            .http
            .get(&full_url)
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        resp.json().await.map_err(Error::from)
    }

    /// Fetch state history for entities within a time range.
    ///
    /// All URL parameters are percent-encoded to handle ISO 8601 timestamps
    /// and entity IDs with special characters.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(entity_id = filter_entity_id)))]
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
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(entity_id)))]
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

    // ── Discovery / introspection ────────────────────────────────────

    /// Lightweight ping: returns `Ok(())` if the API is running.
    ///
    /// `GET /api/` — unauthenticated, returns `"API running."`.
    pub async fn ping(&self) -> Result<()> {
        let resp = self
            .http
            .get(format!("{}/api/", self.base_url))
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(Error::Connection(format!(
                "HA API ping returned {}",
                resp.status()
            )))
        }
    }

    /// Get the list of loaded integration components.
    ///
    /// `GET /api/components` — returns `Vec<String>` of component names.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
    pub async fn get_components(&self) -> Result<Vec<String>> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/components", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        resp.json().await.map_err(Error::from)
    }

    /// Get the list of event listeners (event types and their listener counts).
    ///
    /// `GET /api/events` — returns `Vec<Value>` of `{event, listener_count}`.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
    pub async fn get_events(&self) -> Result<Vec<serde_json::Value>> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/events", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        resp.json().await.map_err(Error::from)
    }

    // ── Entity state mutation ────────────────────────────────────────

    /// Remove an entity from the state machine.
    ///
    /// `DELETE /api/states/{entity_id}` — requires admin privileges.
    /// Returns `Ok(true)` if the entity was removed, `Ok(false)` if not found.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(entity_id)))]
    pub async fn delete_state(&self, entity_id: &str) -> Result<bool> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .delete(format!(
                "{}/api/states/{entity_id}",
                self.base_url
            ))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        match status.as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            _ => {
                let text = resp.text().await.unwrap_or_default();
                Err(Error::ServiceCall(format!(
                    "delete_state {entity_id} returned {status}: {text}"
                )))
            }
        }
    }

    // ── History with query flags ─────────────────────────────────────

    /// Fetch state history with full query flags.
    ///
    /// Extends [`get_history`](Self::get_history) with additional HA query flags:
    /// - `significant_changes_only`: filter out insignificant state changes (default `true`).
    /// - `minimal_response`: return only state and last_updated (default `false`).
    /// - `no_attributes`: strip attributes from state data (default `false`).
    /// - `skip_initial_state`: omit the state at the start_time (default `false`).
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(entity_id = filter_entity_id)))]
    pub async fn get_history_with_flags(
        &self,
        filter_entity_id: &str,
        start_time: &str,
        end_time: Option<&str>,
        significant_changes_only: Option<bool>,
        minimal_response: Option<bool>,
        no_attributes: Option<bool>,
        skip_initial_state: Option<bool>,
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
        // HA defaults: significant_changes_only=1, others absent = false
        if let Some(v) = significant_changes_only {
            url.push_str(&format!("&significant_changes_only={v}"));
        }
        if minimal_response == Some(true) {
            url.push_str("&minimal_response");
        }
        if no_attributes == Some(true) {
            url.push_str("&no_attributes");
        }
        if skip_initial_state == Some(true) {
            url.push_str("&skip_initial_state");
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

    // ── Calendars ────────────────────────────────────────────────────

    /// List all calendar entities.
    ///
    /// `GET /api/calendars` — returns `Vec<Value>` of `{name, entity_id}`.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
    pub async fn get_calendars(&self) -> Result<Vec<serde_json::Value>> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/calendars", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        resp.json().await.map_err(Error::from)
    }

    /// Get calendar events for a specific calendar entity.
    ///
    /// `GET /api/calendars/{entity_id}?start=...&end=...`
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug", fields(entity_id)))]
    pub async fn get_calendar_events(
        &self,
        entity_id: &str,
        start: &str,
        end: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let (_, auth) = self.auth_header();
        let url = format!(
            "{}/api/calendars/{}?start={}&end={}",
            self.base_url,
            urlencoding::encode(entity_id),
            urlencoding::encode(start),
            urlencoding::encode(end),
        );

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

    // ── Template rendering ───────────────────────────────────────────

    /// Render a Jinja2 template on the HA server.
    ///
    /// `POST /api/template` — requires admin privileges.
    /// Sends `{"template": "...", "variables": {...}}` and returns
    /// the rendered result as a string.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
    pub async fn render_template(
        &self,
        template: &str,
        variables: Option<serde_json::Value>,
    ) -> Result<String> {
        let (_, auth) = self.auth_header();

        let mut body = serde_json::Map::new();
        body.insert("template".to_owned(), serde_json::Value::String(template.to_owned()));
        if let Some(vars) = variables {
            body.insert("variables".to_owned(), vars);
        }

        let resp = self
            .http
            .post(format!("{}/api/template", self.base_url))
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Client(format!(
                "render_template returned {status}: {text}"
            )));
        }

        resp.text().await.map_err(Error::from)
    }

    // ── Error log ────────────────────────────────────────────────────

    /// Fetch the HA error log as text.
    ///
    /// `GET /api/error_log` — requires admin privileges.
    /// Returns the raw log file contents (gzipped response may need decompression).
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
    pub async fn get_error_log(&self) -> Result<String> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/error_log", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        resp.text().await.map_err(Error::from)
    }

    // ── Configuration validation ─────────────────────────────────────

    /// Validate the HA configuration files.
    ///
    /// `POST /api/config/core/check_config` — requires admin privileges.
    /// Returns `{result: "valid"|"invalid", errors: Option<String>, warnings: Option<String>}`.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
    pub async fn check_config(&self) -> Result<serde_json::Value> {
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .post(format!(
                "{}/api/config/core/check_config",
                self.base_url
            ))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_auth_status(status)?;
        resp.json().await.map_err(Error::from)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    /// Custom wiremock matcher for query parameters that are flags (no value).
    struct QueryFlag {
        key: &'static str,
    }

    impl wiremock::Match for QueryFlag {
        fn matches(&self, request: &wiremock::Request) -> bool {
            request
                .url
                .query_pairs()
                .any(|(k, _)| k == self.key)
        }
    }

    fn test_account(url: &str) -> HomeAssistantAccountConfig {
        HomeAssistantAccountConfig {
            url: Some(url.to_owned()),
            token: Some(secrecy::Secret::new("test-token".to_owned())),
            timeout_seconds: 10,
        }
    }

    fn test_state_json() -> serde_json::Value {
        json!([{
            "entity_id": "light.living_room",
            "state": "on",
            "attributes": {
                "friendly_name": "Living Room",
                "area_id": "living_room"
            },
            "last_changed": "2026-01-01T00:00:00+00:00",
            "last_updated": "2026-01-01T00:00:00+00:00",
            "context": {"id": "abc", "parent_id": null, "user_id": null}
        }])
    }

    fn test_config_json() -> serde_json::Value {
        json!({
            "version": "2025.1.0",
            "unit_system": "metric",
            "location_name": "Home",
            "latitude": 45.0,
            "longitude": -63.0,
            "elevation": 30.0,
            "time_zone": "America/Halifax",
            "components": ["light", "switch", "sensor"],
            "config_dir": "/config"
        })
    }

    fn test_services_json() -> serde_json::Value {
        json!([{
            "domain": "light",
            "services": {
                "turn_on": {"name": "Turn on", "target": {}},
                "turn_off": {"name": "Turn off", "target": {}}
            }
        }])
    }

    // --- health_check ---

    #[tokio::test]
    async fn health_check_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/config"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(test_config_json()))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        assert!(client.health_check().await.is_ok());
    }

    #[tokio::test]
    async fn health_check_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/config"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client.health_check().await.unwrap_err();
        assert!(matches!(err, Error::Auth(_)));
    }

    #[tokio::test]
    async fn health_check_forbidden() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/config"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client.health_check().await.unwrap_err();
        assert!(matches!(err, Error::Auth(_)));
    }

    #[tokio::test]
    async fn health_check_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/config"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client.health_check().await.unwrap_err();
        assert!(matches!(err, Error::Connection(_)));
    }

    // --- get_config ---

    #[tokio::test]
    async fn get_config_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/config"))
            .respond_with(ResponseTemplate::new(200).set_body_json(test_config_json()))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let config = client.get_config().await.unwrap();
        assert_eq!(config.version, "2025.1.0");
        assert_eq!(config.location_name, "Home");
        assert_eq!(config.latitude, 45.0);
        assert_eq!(config.longitude, -63.0);
        assert_eq!(config.elevation, 30.0);
        assert_eq!(config.time_zone, "America/Halifax");
        assert_eq!(config.components, vec!["light", "switch", "sensor"]);
    }

    // --- get_states ---

    #[tokio::test]
    async fn get_states_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states"))
            .respond_with(ResponseTemplate::new(200).set_body_json(test_state_json()))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let states = client.get_states().await.unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].entity_id, "light.living_room");
        assert_eq!(states[0].state, "on");
    }

    #[tokio::test]
    async fn get_states_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let states = client.get_states().await.unwrap();
        assert!(states.is_empty());
    }

    // --- get_state ---

    #[tokio::test]
    async fn get_state_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states/light.living_room"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entity_id": "light.living_room",
                "state": "on",
                "attributes": {"friendly_name": "Living Room"},
                "last_changed": "2026-01-01T00:00:00+00:00",
                "last_updated": "2026-01-01T00:00:00+00:00",
                "context": {"id": "abc", "parent_id": null, "user_id": null}
            })))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let state = client.get_state("light.living_room").await.unwrap();
        assert!(state.is_some());
        assert_eq!(state.unwrap().entity_id, "light.living_room");
    }

    #[tokio::test]
    async fn get_state_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/states/light.nonexistent"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let state = client.get_state("light.nonexistent").await.unwrap();
        assert!(state.is_none());
    }

    // --- get_services ---

    #[tokio::test]
    async fn get_services_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(test_services_json()))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let services = client.get_services().await.unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].domain, "light");
    }

    // --- call_service ---

    #[tokio::test]
    async fn call_service_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/light/turn_on"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let result = client
            .call_service("light", "turn_on", None, None, false)
            .await
            .unwrap();
        assert_eq!(result, json!([]));
    }

    #[tokio::test]
    async fn call_service_with_target() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/light/turn_on"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let target = Target::entity("light.living_room");
        client
            .call_service("light", "turn_on", Some(&target), None, false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn call_service_with_data() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/light/turn_on"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let data = json!({"brightness": 255, "color_temp": 370});
        client
            .call_service("light", "turn_on", None, Some(data), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn call_service_rejects_non_object_data() {
        let client = HomeAssistantClient::new(&test_account("http://localhost:1")).unwrap();
        let err = client
            .call_service("light", "turn_on", None, Some(json!("not an object")), false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Client(_)));
    }

    #[tokio::test]
    async fn call_service_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/light/turn_on"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"error": "entity not found"})),
            )
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client
            .call_service("light", "turn_on", None, None, false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::ServiceCall(_)));
    }

    #[tokio::test]
    async fn call_service_return_response_appends_query_param() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/weather/get_forecasts"))
            .and(QueryFlag { key: "return_response" })
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "changed_states": [],
                "service_response": {"forecast": []}
            })))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let result = client
            .call_service("weather", "get_forecasts", None, None, true)
            .await
            .unwrap();
        assert!(result.get("service_response").is_some());
    }

    // --- turn_on / turn_off / toggle ---

    #[tokio::test]
    async fn turn_on_delegates_to_service() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/light/turn_on"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        assert!(client.turn_on("light.living_room").await.is_ok());
    }

    #[tokio::test]
    async fn turn_off_delegates_to_service() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/light/turn_off"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        assert!(client.turn_off("light.living_room").await.is_ok());
    }

    #[tokio::test]
    async fn toggle_delegates_to_service() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/services/light/toggle"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        assert!(client.toggle("light.living_room").await.is_ok());
    }

    // --- fire_event ---

    #[tokio::test]
    async fn fire_event_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/events/custom_event"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "ok"})))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        assert!(client
            .fire_event("custom_event", Some(json!({"key": "value"})))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn fire_event_no_data() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/events/custom_event"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        assert!(client.fire_event("custom_event", None).await.is_ok());
    }

    #[tokio::test]
    async fn fire_event_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/events/custom_event"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client.fire_event("custom_event", None).await.unwrap_err();
        assert!(matches!(err, Error::Event(_)));
    }

    // --- get_history ---

    #[tokio::test]
    async fn get_history_success() {
        let server = MockServer::start().await;
        // HA history API: GET /api/history/period/<start_time>?filter_entity_id=...
        Mock::given(method("GET"))
            .and(path("/api/history/period/2026-01-01T00%3A00%3A00%2B00%3A00"))
            .and(query_param("filter_entity_id", "sensor.temperature"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "entity_id": "sensor.temperature",
                "state": "22.5",
                "attributes": {},
                "last_changed": "2026-01-01T00:00:00+00:00",
                "last_updated": "2026-01-01T00:00:00+00:00",
                "context": {}
            }])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let history = client
            .get_history("sensor.temperature", "2026-01-01T00:00:00+00:00", None)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
    }

    #[tokio::test]
    async fn get_history_with_end_time() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/history/period/2026-01-01T00%3A00%3A00%2B00%3A00"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let history = client
            .get_history(
                "sensor.temperature",
                "2026-01-01T00:00:00+00:00",
                Some("2026-01-02T00:00:00+00:00"),
            )
            .await
            .unwrap();
        assert!(history.is_empty());
    }

    // --- camera_proxy ---

    #[tokio::test]
    async fn camera_proxy_success() {
        let server = MockServer::start().await;
        let fake_image = bytes::Bytes::from_static(b"\x89PNG\r\n\x1a\nfake");
        Mock::given(method("GET"))
            .and(path("/api/camera_proxy/camera.front_door"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(fake_image.clone()))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let bytes = client.camera_proxy("camera.front_door").await.unwrap();
        assert_eq!(bytes, fake_image);
    }

    #[tokio::test]
    async fn camera_proxy_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/camera_proxy/camera.nonexistent"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client.camera_proxy("camera.nonexistent").await.unwrap_err();
        assert!(matches!(err, Error::Camera(_)));
    }

    // --- set_state ---

    #[tokio::test]
    async fn set_state_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/states/input_boolean.test_mode"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entity_id": "input_boolean.test_mode",
                "state": "on",
                "attributes": {"friendly_name": "Test Mode"},
                "last_changed": "2026-01-01T00:00:00+00:00",
                "last_updated": "2026-01-01T00:00:00+00:00",
                "context": {}
            })))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let state = client
            .set_state("input_boolean.test_mode", "on", None)
            .await
            .unwrap();
        assert_eq!(state.entity_id, "input_boolean.test_mode");
        assert_eq!(state.state, "on");
    }

    #[tokio::test]
    async fn set_state_with_attributes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/states/sensor.custom"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "entity_id": "sensor.custom",
                "state": "42",
                "attributes": {"unit_of_measurement": "°C", "friendly_name": "Custom"},
                "last_changed": "2026-01-01T00:00:00+00:00",
                "last_updated": "2026-01-01T00:00:00+00:00",
                "context": {}
            })))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let state = client
            .set_state(
                "sensor.custom",
                "42",
                Some(json!({"unit_of_measurement": "°C", "friendly_name": "Custom"})),
            )
            .await
            .unwrap();
        assert_eq!(state.state, "42");
    }

    #[tokio::test]
    async fn set_state_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/states/light.test"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client.set_state("light.test", "on", None).await.unwrap_err();
        assert!(matches!(err, Error::ServiceCall(_)));
    }

    // --- get_logbook ---

    #[tokio::test]
    async fn get_logbook_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/logbook/2026-04-21T00%3A00%3A00"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {
                    "name": "Living Room",
                    "entity_id": "light.living_room",
                    "state": "on",
                    "last_changed": "2026-04-21T12:00:00+00:00"
                }
            ])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let entries = client
            .get_logbook(
                Some("2026-04-21T00:00:00"),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["entity_id"], "light.living_room");
    }

    #[tokio::test]
    async fn get_logbook_with_entity_and_period() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/logbook/2026-04-20T00%3A00%3A00"))
            .and(query_param("entity", "light.living_room"))
            .and(query_param("period", "3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let entries = client
            .get_logbook(
                Some("2026-04-20T00:00:00"),
                None,
                Some("light.living_room"),
                Some(3),
            )
            .await
            .unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn get_logbook_default_today() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/logbook"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let entries = client
            .get_logbook(None, None, None, None)
            .await
            .unwrap();
        assert!(entries.is_empty());
    }

    // --- construction ---

    #[test]
    fn new_rejects_missing_url() {
        let account = HomeAssistantAccountConfig {
            url: None,
            token: Some(secrecy::Secret::new("tok".to_owned())),
            timeout_seconds: 10,
        };
        assert!(HomeAssistantClient::new(&account).is_err());
    }

    #[test]
    fn new_rejects_missing_token() {
        let account = HomeAssistantAccountConfig {
            url: Some("http://localhost:8123".to_owned()),
            token: None,
            timeout_seconds: 10,
        };
        assert!(HomeAssistantClient::new(&account).is_err());
    }

    #[test]
    fn new_strips_trailing_slash() {
        let client = HomeAssistantClient::new(&test_account("http://localhost:8123/")).unwrap();
        assert_eq!(client.base_url, "http://localhost:8123");
    }

    // --- extract_domain helper ---

    #[test]
    fn extract_domain_standard() {
        assert_eq!(extract_domain("light.living_room"), "light");
        assert_eq!(extract_domain("sensor.temperature_2"), "sensor");
        assert_eq!(extract_domain("switch.kitchen_fan"), "switch");
    }

    #[test]
    fn extract_domain_edge_cases() {
        assert_eq!(extract_domain("noperiod"), "noperiod");
        assert_eq!(extract_domain(""), "");
        assert_eq!(extract_domain("homeassistant.hello"), "homeassistant");
    }

    // --- check_auth_status helper ---

    #[test]
    fn check_auth_status_ok() {
        assert!(check_auth_status(reqwest::StatusCode::OK).is_ok());
        assert!(check_auth_status(reqwest::StatusCode::NO_CONTENT).is_ok());
        assert!(check_auth_status(reqwest::StatusCode::INTERNAL_SERVER_ERROR).is_ok());
    }

    #[test]
    fn check_auth_status_401() {
        assert!(matches!(
            check_auth_status(reqwest::StatusCode::UNAUTHORIZED),
            Err(Error::Auth(_))
        ));
    }

    #[test]
    fn check_auth_status_403() {
        assert!(matches!(
            check_auth_status(reqwest::StatusCode::FORBIDDEN),
            Err(Error::Auth(_))
        ));
    }

    // --- ping (GET /api/) ---

    #[tokio::test]
    async fn ping_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "message": "API running."
            })))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        assert!(client.ping().await.is_ok());
    }

    #[tokio::test]
    async fn ping_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        assert!(client.ping().await.is_err());
    }

    // --- get_components ---

    #[tokio::test]
    async fn get_components_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/components"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!(["light", "switch", "sensor"])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let components = client.get_components().await.unwrap();
        assert_eq!(components, vec!["light", "switch", "sensor"]);
    }

    // --- get_events ---

    #[tokio::test]
    async fn get_events_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"event": "state_changed", "listener_count": 5}
            ])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let events = client.get_events().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "state_changed");
    }

    // --- delete_state ---

    #[tokio::test]
    async fn delete_state_success() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/states/sensor.stale"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        assert!(client.delete_state("sensor.stale").await.unwrap());
    }

    #[tokio::test]
    async fn delete_state_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/states/sensor.nonexistent"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        assert!(!client.delete_state("sensor.nonexistent").await.unwrap());
    }

    // --- get_history_with_flags ---

    #[tokio::test]
    async fn get_history_with_flags_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/history/period/2026-01-01T00%3A00%3A00%2B00%3A00"))
            .and(query_param("filter_entity_id", "sensor.temp"))
            .and(query_param("minimal_response", ""))
            .and(query_param("no_attributes", ""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let result = client
            .get_history_with_flags(
                "sensor.temp",
                "2026-01-01T00:00:00+00:00",
                None,
                None,
                Some(true),
                Some(true),
                None,
            )
            .await;
        assert!(result.is_ok());
    }

    // --- get_calendars ---

    #[tokio::test]
    async fn get_calendars_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/calendars"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"name": "Personal", "entity_id": "calendar.personal"}
            ])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let cals = client.get_calendars().await.unwrap();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0]["entity_id"], "calendar.personal");
    }

    // --- get_calendar_events ---

    #[tokio::test]
    async fn get_calendar_events_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/calendars/calendar.personal"))
            .and(query_param("start", "2026-04-21T00:00:00"))
            .and(query_param("end", "2026-04-22T00:00:00"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let events = client
            .get_calendar_events("calendar.personal", "2026-04-21T00:00:00", "2026-04-22T00:00:00")
            .await
            .unwrap();
        assert!(events.is_empty());
    }

    // --- render_template ---

    #[tokio::test]
    async fn render_template_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/template"))
            .respond_with(ResponseTemplate::new(200).set_body_string("on"))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let result = client
            .render_template("{{ states('light.living_room') }}", None)
            .await
            .unwrap();
        assert_eq!(result, "on");
    }

    #[tokio::test]
    async fn render_template_with_variables() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/template"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hello world"))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let result = client
            .render_template(
                "{{ name }}",
                Some(json!({"name": "hello world"})),
            )
            .await
            .unwrap();
        assert_eq!(result, "hello world");
    }

    // --- get_error_log ---

    #[tokio::test]
    async fn get_error_log_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/error_log"))
            .respond_with(ResponseTemplate::new(200).set_body_string("2026-04-21 error line\n"))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let log = client.get_error_log().await.unwrap();
        assert!(log.contains("error line"));
    }

    // --- check_config ---

    #[tokio::test]
    async fn check_config_valid() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/config/core/check_config"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": "valid",
                "errors": null,
                "warnings": null
            })))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let result = client.check_config().await.unwrap();
        assert_eq!(result["result"], "valid");
    }

    #[tokio::test]
    async fn check_config_invalid() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/config/core/check_config"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "result": "invalid",
                "errors": "Missing key 'platform' at line 5",
                "warnings": null
            })))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let result = client.check_config().await.unwrap();
        assert_eq!(result["result"], "invalid");
        assert!(result["errors"].is_string());
    }
}
