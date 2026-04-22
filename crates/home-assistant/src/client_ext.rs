//! Extended Home Assistant REST API client — discovery, admin, and utility endpoints.
//!
//! These methods live in a separate module to keep [`client`] under the 1500-line
//! CI limit. All methods are added directly to [`HomeAssistantClient`](super::client::HomeAssistantClient)
//! via an `impl` block in the same crate.

use {
    super::client::{HomeAssistantClient, check_status_metrics},
    crate::error::{Error, Result},
};

#[cfg(feature = "metrics")]
use {
    super::client::{record_rest_error, record_rest_request},
};

impl HomeAssistantClient {
    // ── Discovery / introspection ────────────────────────────────────

    /// Lightweight ping: returns `Ok(())` if the API is running.
    ///
    /// `GET /api/` — unauthenticated, returns `"API running."`.
    pub async fn ping(&self) -> Result<()> {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();
        let resp = self
            .http
            .get(format!("{}/api/", self.base_url))
            .send()
            .await?;

        let ok = resp.status().is_success();
        if ok {
            #[cfg(feature = "metrics")]
            record_rest_request("GET", "ping", start);
            Ok(())
        } else {
            #[cfg(feature = "metrics")]
            record_rest_error("GET", "ping");
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
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/components", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_status_metrics(status, "GET", "components")?;
        #[cfg(feature = "metrics")]
        record_rest_request("GET", "components", start);
        resp.json().await.map_err(Error::from)
    }

    /// Get the list of event listeners (event types and their listener counts).
    ///
    /// `GET /api/events` — returns `Vec<Value>` of `{event, listener_count}`.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
    pub async fn get_events(&self) -> Result<Vec<serde_json::Value>> {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/events", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_status_metrics(status, "GET", "events")?;
        #[cfg(feature = "metrics")]
        record_rest_request("GET", "events", start);
        resp.json().await.map_err(Error::from)
    }

    // ── Entity state mutation ────────────────────────────────────────

    /// Remove an entity from the state machine.
    ///
    /// `DELETE /api/states/{entity_id}` — requires admin privileges.
    /// Returns `Ok(true)` if the entity was removed, `Ok(false)` if not found.
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip_all, level = "debug", fields(entity_id))
    )]
    pub async fn delete_state(&self, entity_id: &str) -> Result<bool> {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .delete(format!("{}/api/states/{entity_id}", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        if status.as_u16() == 404 {
            #[cfg(feature = "metrics")]
            record_rest_request("DELETE", "delete_state", start);
            return Ok(false);
        }
        check_status_metrics(status, "DELETE", "delete_state")?;
        #[cfg(feature = "metrics")]
        record_rest_request("DELETE", "delete_state", start);
        Ok(true)
    }

    // ── History with query flags ─────────────────────────────────────

    /// Fetch state history with full query flags.
    ///
    /// Extends [`get_history`](Self::get_history) with additional HA query flags:
    /// - `significant_changes_only`: filter out insignificant state changes (default `true`).
    /// - `minimal_response`: return only state and last_updated (default `false`).
    /// - `no_attributes`: strip attributes from state data (default `false`).
    /// - `skip_initial_state`: omit the state at the start_time (default `false`).
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip_all, level = "debug", fields(entity_id = filter_entity_id))
    )]
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
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();
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
        // HA checks `query.get("significant_changes_only", "1") != "0"`
        // so we must send "0" or "1" (not "false"/"true").
        if let Some(v) = significant_changes_only {
            url.push_str(&format!(
                "&significant_changes_only={}",
                if v {
                    "1"
                } else {
                    "0"
                }
            ));
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
        check_status_metrics(status, "GET", "history_with_flags")?;
        #[cfg(feature = "metrics")]
        record_rest_request("GET", "history_with_flags", start);
        resp.json().await.map_err(Error::from)
    }

    // ── Calendars ────────────────────────────────────────────────────

    /// List all calendar entities.
    ///
    /// `GET /api/calendars` — returns `Vec<Value>` of `{name, entity_id}`.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
    pub async fn get_calendars(&self) -> Result<Vec<serde_json::Value>> {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/calendars", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_status_metrics(status, "GET", "calendars")?;
        #[cfg(feature = "metrics")]
        record_rest_request("GET", "calendars", start);
        resp.json().await.map_err(Error::from)
    }

    /// Get calendar events for a specific calendar entity.
    ///
    /// `GET /api/calendars/{entity_id}?start=...&end=...`
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip_all, level = "debug", fields(entity_id))
    )]
    pub async fn get_calendar_events(
        &self,
        entity_id: &str,
        start: &str,
        end: &str,
    ) -> Result<Vec<serde_json::Value>> {
        #[cfg(feature = "metrics")]
        let metrics_start = std::time::Instant::now();
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
        check_status_metrics(status, "GET", "calendar_events")?;
        #[cfg(feature = "metrics")]
        record_rest_request("GET", "calendar_events", metrics_start);
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
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();
        let (_, auth) = self.auth_header();

        let mut body = serde_json::Map::new();
        body.insert(
            "template".to_owned(),
            serde_json::Value::String(template.to_owned()),
        );
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
            #[cfg(feature = "metrics")]
            record_rest_error("POST", "render_template");
            return Err(Error::Client(format!(
                "render_template returned {status}: {text}"
            )));
        }

        #[cfg(feature = "metrics")]
        record_rest_request("POST", "render_template", start);
        resp.text().await.map_err(Error::from)
    }

    // ── Error log ────────────────────────────────────────────────────

    /// Fetch the HA error log as text.
    ///
    /// `GET /api/error_log` — requires admin privileges.
    /// Returns the raw log file contents (gzipped response may need decompression).
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
    pub async fn get_error_log(&self) -> Result<String> {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .get(format!("{}/api/error_log", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_status_metrics(status, "GET", "error_log")?;
        #[cfg(feature = "metrics")]
        record_rest_request("GET", "error_log", start);
        resp.text().await.map_err(Error::from)
    }

    // ── Configuration validation ─────────────────────────────────────

    /// Validate the HA configuration files.
    ///
    /// `POST /api/config/core/check_config` — requires admin privileges.
    /// Returns `{result: "valid"|"invalid", errors: Option<String>, warnings: Option<String>}`.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "debug"))]
    pub async fn check_config(&self) -> Result<serde_json::Value> {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();
        let (_, auth) = self.auth_header();
        let resp = self
            .http
            .post(format!("{}/api/config/core/check_config", self.base_url))
            .header("Authorization", &auth)
            .send()
            .await?;

        let status = resp.status();
        check_status_metrics(status, "POST", "check_config")?;
        #[cfg(feature = "metrics")]
        record_rest_request("POST", "check_config", start);
        resp.json().await.map_err(Error::from)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::*,
        crate::config::HomeAssistantAccountConfig,
        serde_json::json,
        wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path, query_param},
        },
    };

    fn test_account(url: &str) -> HomeAssistantAccountConfig {
        HomeAssistantAccountConfig {
            url: Some(url.to_owned()),
            token: Some(secrecy::Secret::new("test-token".to_owned())),
            timeout_seconds: 10,
        }
    }

    // --- ping ---

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
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!(["light", "switch", "sensor"])),
            )
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let components = client.get_components().await.unwrap();
        assert_eq!(components, vec!["light", "switch", "sensor"]);
    }

    #[tokio::test]
    async fn get_components_auth_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/components"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client.get_components().await.unwrap_err();
        assert!(format!("{err}").contains("401"));
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

    #[tokio::test]
    async fn get_events_auth_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/events"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client.get_events().await.unwrap_err();
        assert!(format!("{err}").contains("403"));
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
            .and(path(
                "/api/history/period/2026-01-01T00%3A00%3A00%2B00%3A00",
            ))
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

    #[tokio::test]
    async fn get_history_with_flags_significant_changes_sends_0_when_false() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/api/history/period/2026-01-01T00%3A00%3A00%2B00%3A00",
            ))
            .and(query_param("filter_entity_id", "sensor.temp"))
            .and(query_param("significant_changes_only", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let result = client
            .get_history_with_flags(
                "sensor.temp",
                "2026-01-01T00:00:00+00:00",
                None,
                Some(false), // must send "0", not "false"
                None,
                None,
                None,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_history_with_flags_significant_changes_sends_1_when_true() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/api/history/period/2026-01-01T00%3A00%3A00%2B00%3A00",
            ))
            .and(query_param("filter_entity_id", "sensor.temp"))
            .and(query_param("significant_changes_only", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let result = client
            .get_history_with_flags(
                "sensor.temp",
                "2026-01-01T00:00:00+00:00",
                None,
                Some(true), // must send "1", not "true"
                None,
                None,
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

    #[tokio::test]
    async fn get_calendars_auth_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/calendars"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client.get_calendars().await.unwrap_err();
        assert!(format!("{err}").contains("401"));
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
            .get_calendar_events(
                "calendar.personal",
                "2026-04-21T00:00:00",
                "2026-04-22T00:00:00",
            )
            .await
            .unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn get_calendar_events_auth_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/calendars/calendar.personal"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client
            .get_calendar_events(
                "calendar.personal",
                "2026-04-21T00:00:00",
                "2026-04-22T00:00:00",
            )
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("403"));
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
            .render_template("{{ name }}", Some(json!({"name": "hello world"})))
            .await
            .unwrap();
        assert_eq!(result, "hello world");
    }

    #[tokio::test]
    async fn render_template_error_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/template"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string("Error rendering template: ..."),
            )
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client
            .render_template("{{ invalid", None)
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("render_template"));
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

    #[tokio::test]
    async fn get_error_log_auth_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/error_log"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = HomeAssistantClient::new(&test_account(&server.uri())).unwrap();
        let err = client.get_error_log().await.unwrap_err();
        assert!(format!("{err}").contains("401"));
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
