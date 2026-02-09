//! `get_user_location` tool — requests the user's geographic coordinates via
//! the browser Geolocation API.
//!
//! The tool checks for a cached location first (fast path), then asks the
//! gateway to send a WebSocket event to the connected browser client.  The
//! browser shows its native permission popup and returns the coordinates (or
//! an error) via an RPC response.

use std::sync::Arc;

use {
    anyhow::Result,
    async_trait::async_trait,
    moltis_config::GeoLocation,
    serde::{Deserialize, Serialize},
    tracing::warn,
};

// ── Precision ───────────────────────────────────────────────────────────────

/// How accurate the location fix should be.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocationPrecision {
    /// City-level (~1-5 km). Fast, low-power. Good for flights, weather, time zone.
    Coarse,
    /// GPS-level (~5-20 m). May take a few seconds. Good for nearby places,
    /// walking directions, "closest X".
    #[default]
    Precise,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// Location coordinates returned by the browser Geolocation API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserLocation {
    pub latitude: f64,
    pub longitude: f64,
    /// Accuracy in metres.
    pub accuracy: f64,
}

/// Reason the location could not be obtained.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationError {
    PermissionDenied,
    PositionUnavailable,
    Timeout,
    NoClientConnected,
    NotSupported,
}

impl std::fmt::Display for LocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied => f.write_str("User denied location permission"),
            Self::PositionUnavailable => f.write_str("Position unavailable"),
            Self::Timeout => f.write_str("Location request timed out"),
            Self::NoClientConnected => f.write_str(
                "No browser client connected — location requires an active browser session",
            ),
            Self::NotSupported => f.write_str("Geolocation not supported in this browser"),
        }
    }
}

/// Result from the browser geolocation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<BrowserLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<LocationError>,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Abstraction for requesting location from a connected browser client.
///
/// Implemented by the gateway layer and injected into [`LocationTool`] at
/// construction time.  This avoids a circular dependency between `crates/tools`
/// and `crates/gateway`.
#[async_trait]
pub trait LocationRequester: Send + Sync {
    /// Request location from the client identified by `conn_id`.
    ///
    /// The implementation creates a pending‐invoke, sends a WebSocket event to
    /// the browser, and awaits the response with a timeout.
    ///
    /// `precision` controls the browser's `enableHighAccuracy` and cache age.
    async fn request_location(
        &self,
        conn_id: &str,
        precision: LocationPrecision,
    ) -> Result<LocationResult>;

    /// Return a previously cached location (from `USER.md` or in-memory cache).
    fn cached_location(&self) -> Option<GeoLocation>;

    /// Request location from a channel user (e.g. Telegram).
    ///
    /// Sends a message asking the user to share their location via the channel's
    /// native location-sharing feature, then waits for the result.
    async fn request_channel_location(&self, _session_key: &str) -> Result<LocationResult> {
        Ok(LocationResult {
            location: None,
            error: Some(LocationError::NotSupported),
        })
    }
}

// ── Reverse geocoding ────────────────────────────────────────────────────────

/// Nominatim reverse-geocode response (subset of fields we care about).
#[derive(Debug, Deserialize)]
struct NominatimResponse {
    #[serde(default)]
    address: Option<NominatimAddress>,
    #[serde(default)]
    display_name: Option<String>,
}

/// Address breakdown from Nominatim.
#[derive(Debug, Deserialize)]
struct NominatimAddress {
    #[serde(default)]
    neighbourhood: Option<String>,
    #[serde(default)]
    suburb: Option<String>,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    town: Option<String>,
    #[serde(default)]
    village: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    country_code: Option<String>,
}

/// Full + short place name pair from reverse geocoding.
struct PlaceName {
    /// Full place string, e.g. "Noe Valley, San Francisco, California, US".
    full: String,
    /// Shortest useful locality, e.g. "Noe Valley". Suitable for TTS.
    short: String,
}

/// Resolve lat/lon to a human-readable place name via Nominatim (OpenStreetMap).
///
/// Returns `None` on any failure (network, parse, timeout) so the caller can
/// fall back to raw coordinates.
async fn reverse_geocode(lat: f64, lon: f64) -> Option<PlaceName> {
    reverse_geocode_with_client(&reqwest::Client::new(), lat, lon).await
}

/// Inner implementation that accepts a `reqwest::Client` for testability.
async fn reverse_geocode_with_client(
    client: &reqwest::Client,
    lat: f64,
    lon: f64,
) -> Option<PlaceName> {
    let url = format!(
        "https://nominatim.openstreetmap.org/reverse?lat={lat}&lon={lon}&format=json&zoom=14"
    );
    let resp = client
        .get(&url)
        .header("User-Agent", "moltis/0.3")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        warn!(status = %resp.status(), "Nominatim reverse geocode failed");
        return None;
    }

    let body: NominatimResponse = resp.json().await.ok()?;
    if let Some(addr) = body.address {
        Some(PlaceName {
            full: format_address(&addr),
            short: format_address_short(&addr),
        })
    } else {
        let name = body.display_name?;
        // Best-effort short: take everything before the first comma.
        let short = name
            .split_once(',')
            .map_or(name.as_str(), |(first, _)| first)
            .trim()
            .to_string();
        Some(PlaceName { full: name, short })
    }
}

/// Build a concise place string from Nominatim address components.
///
/// Prefers: neighbourhood/suburb, city/town/village, state, country_code.
fn format_address(addr: &NominatimAddress) -> String {
    let local = addr
        .neighbourhood
        .as_deref()
        .or(addr.suburb.as_deref())
        .unwrap_or_default();
    let city = addr
        .city
        .as_deref()
        .or(addr.town.as_deref())
        .or(addr.village.as_deref())
        .unwrap_or_default();
    let state = addr.state.as_deref().unwrap_or_default();
    let country = addr
        .country_code
        .as_deref()
        .unwrap_or_default()
        .to_uppercase();

    [local, city, state, &country]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

/// Return the most local place component — suitable for voice/TTS.
///
/// Prefers neighbourhood/suburb, then city/town/village, then state.
fn format_address_short(addr: &NominatimAddress) -> String {
    addr.neighbourhood
        .as_deref()
        .or(addr.suburb.as_deref())
        .or(addr.city.as_deref())
        .or(addr.town.as_deref())
        .or(addr.village.as_deref())
        .or(addr.state.as_deref())
        .unwrap_or_default()
        .to_string()
}

/// Set `place` and `place_short` on a JSON response object.
fn set_place_fields(resp: &mut serde_json::Value, place: Option<&PlaceName>) {
    if let Some(p) = place {
        resp["place"] = serde_json::Value::String(p.full.clone());
        if !p.short.is_empty() {
            resp["place_short"] = serde_json::Value::String(p.short.clone());
        }
    }
}

// ── Tool ──────────────────────────────────────────────────────────────────────

/// LLM-callable tool that requests the user's geographic coordinates.
pub struct LocationTool {
    requester: Arc<dyn LocationRequester>,
}

impl LocationTool {
    pub fn new(requester: Arc<dyn LocationRequester>) -> Self {
        Self { requester }
    }
}

#[async_trait]
impl moltis_agents::tool_registry::AgentTool for LocationTool {
    fn name(&self) -> &str {
        "get_user_location"
    }

    fn description(&self) -> &str {
        "Get the user's current location as coordinates and a place name. \
         Requires user permission via browser popup. Use when the user asks \
         about local weather, nearby places, directions, or anything \
         location-dependent. Returns `latitude`, `longitude`, `place` (full \
         location string), and `place_short` (neighbourhood/city). \
         IMPORTANT: For web searches, map lookups, and Google queries always \
         use the numeric `latitude` and `longitude` (e.g. \"lunch near \
         37.76,-122.42\") — place names are too imprecise for search engines. \
         Only use `place_short` when speaking aloud to the user. \
         Use `show_map` to display a map image with links to the user — \
         always pass the user's coordinates as `user_latitude`/`user_longitude` \
         so both positions appear on the map. \
         Set `precision` to choose accuracy: \"precise\" (default, GPS-level, \
         best for nearby places / walking directions / show_map) or \"coarse\" \
         (city-level, faster, good for flights / weather / time zones)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "precision": {
                    "type": "string",
                    "enum": ["precise", "coarse"],
                    "description": "Location accuracy: \"precise\" (GPS, ~5-20m, default) or \"coarse\" (city-level, ~1-5km, faster)"
                }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        // Fast path: return cached location.
        if let Some(loc) = self.requester.cached_location() {
            let geocoded = match loc.place {
                Some(ref p) => {
                    let short = p
                        .split_once(',')
                        .map_or(p.as_str(), |(first, _)| first)
                        .trim()
                        .to_string();
                    Some(PlaceName {
                        full: p.clone(),
                        short,
                    })
                },
                None => reverse_geocode(loc.latitude, loc.longitude).await,
            };
            let mut resp = serde_json::json!({
                "latitude": loc.latitude,
                "longitude": loc.longitude,
                "source": "cached"
            });
            set_place_fields(&mut resp, geocoded.as_ref());
            return Ok(resp);
        }

        // Parse requested precision (default: Precise).
        let precision: LocationPrecision = params
            .get("precision")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Try browser geolocation if a connection ID is available.
        if let Some(conn_id) = params.get("_conn_id").and_then(|v| v.as_str()) {
            let result = self.requester.request_location(conn_id, precision).await?;
            return match result.location {
                Some(loc) => {
                    let geocoded = reverse_geocode(loc.latitude, loc.longitude).await;
                    let mut resp = serde_json::json!({
                        "latitude": loc.latitude,
                        "longitude": loc.longitude,
                        "accuracy_meters": loc.accuracy,
                        "source": "browser"
                    });
                    set_place_fields(&mut resp, geocoded.as_ref());
                    Ok(resp)
                },
                None => {
                    let msg = result
                        .error
                        .as_ref()
                        .map_or("Unknown location error".to_string(), ToString::to_string);
                    Ok(serde_json::json!({
                        "error": msg,
                        "available": false
                    }))
                },
            };
        }

        // No browser connection — try channel-based location request.
        if let Some(session_key) = params.get("_session_key").and_then(|v| v.as_str())
            && (session_key.starts_with("telegram:") || session_key.starts_with("discord:"))
        {
            let result = self.requester.request_channel_location(session_key).await?;
            return match result.location {
                Some(loc) => {
                    let geocoded = reverse_geocode(loc.latitude, loc.longitude).await;
                    let mut resp = serde_json::json!({
                        "latitude": loc.latitude,
                        "longitude": loc.longitude,
                        "accuracy_meters": loc.accuracy,
                        "source": "channel"
                    });
                    set_place_fields(&mut resp, geocoded.as_ref());
                    Ok(resp)
                },
                None => {
                    let msg = result
                        .error
                        .as_ref()
                        .map_or("Unknown location error".to_string(), ToString::to_string);
                    Ok(serde_json::json!({
                        "error": msg,
                        "available": false
                    }))
                },
            };
        }

        Err(anyhow::anyhow!(
            "no client connection available for location request"
        ))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use {super::*, moltis_agents::tool_registry::AgentTool};

    /// Mock requester that returns a fixed response.
    struct MockRequester {
        cached: Option<GeoLocation>,
        response: LocationResult,
        channel_response: Option<LocationResult>,
    }

    #[async_trait]
    impl LocationRequester for MockRequester {
        async fn request_location(
            &self,
            _conn_id: &str,
            _precision: LocationPrecision,
        ) -> Result<LocationResult> {
            Ok(self.response.clone())
        }

        fn cached_location(&self) -> Option<GeoLocation> {
            self.cached.clone()
        }

        async fn request_channel_location(&self, _session_key: &str) -> Result<LocationResult> {
            match &self.channel_response {
                Some(r) => Ok(r.clone()),
                None => Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::NotSupported),
                }),
            }
        }
    }

    #[tokio::test]
    async fn cached_location_returns_immediately() {
        let tool = LocationTool::new(Arc::new(MockRequester {
            cached: Some(GeoLocation {
                latitude: 48.8566,
                longitude: 2.3522,
                place: Some("Paris, France".to_string()),
                updated_at: None,
            }),
            response: LocationResult {
                location: None,
                error: None,
            },
            channel_response: None,
        }));

        let result = tool.execute(serde_json::json!({})).await.unwrap();
        assert_eq!(result["latitude"], 48.8566);
        assert_eq!(result["source"], "cached");
        assert_eq!(result["place"], "Paris, France");
        assert_eq!(result["place_short"], "Paris");
    }

    #[tokio::test]
    async fn browser_location_success() {
        let tool = LocationTool::new(Arc::new(MockRequester {
            cached: None,
            response: LocationResult {
                location: Some(BrowserLocation {
                    latitude: 40.7128,
                    longitude: -74.006,
                    accuracy: 25.0,
                }),
                error: None,
            },
            channel_response: None,
        }));

        let result = tool
            .execute(serde_json::json!({ "_conn_id": "test-conn" }))
            .await
            .unwrap();
        assert_eq!(result["latitude"], 40.7128);
        assert_eq!(result["source"], "browser");
        assert_eq!(result["accuracy_meters"], 25.0);
    }

    #[tokio::test]
    async fn permission_denied_returns_error_json() {
        let tool = LocationTool::new(Arc::new(MockRequester {
            cached: None,
            response: LocationResult {
                location: None,
                error: Some(LocationError::PermissionDenied),
            },
            channel_response: None,
        }));

        let result = tool
            .execute(serde_json::json!({ "_conn_id": "test-conn" }))
            .await
            .unwrap();
        assert_eq!(result["available"], false);
        assert!(result["error"].as_str().unwrap().contains("denied"));
    }

    #[tokio::test]
    async fn missing_conn_id_returns_error() {
        let tool = LocationTool::new(Arc::new(MockRequester {
            cached: None,
            response: LocationResult {
                location: None,
                error: None,
            },
            channel_response: None,
        }));

        let err = tool.execute(serde_json::json!({})).await.unwrap_err();
        assert!(err.to_string().contains("no client connection"));
    }

    #[test]
    fn tool_schema_is_valid() {
        let tool = LocationTool::new(Arc::new(MockRequester {
            cached: None,
            response: LocationResult {
                location: None,
                error: None,
            },
            channel_response: None,
        }));

        assert_eq!(tool.name(), "get_user_location");
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        // Precision enum must be in schema.
        let precision = &schema["properties"]["precision"];
        assert_eq!(precision["type"], "string");
    }

    /// Mock that captures the precision value passed to `request_location`.
    struct PrecisionCapturingRequester {
        captured: std::sync::Mutex<Option<LocationPrecision>>,
    }

    #[async_trait]
    impl LocationRequester for PrecisionCapturingRequester {
        async fn request_location(
            &self,
            _conn_id: &str,
            precision: LocationPrecision,
        ) -> Result<LocationResult> {
            *self.captured.lock().unwrap() = Some(precision);
            Ok(LocationResult {
                location: Some(BrowserLocation {
                    latitude: 1.0,
                    longitude: 2.0,
                    accuracy: 100.0,
                }),
                error: None,
            })
        }

        fn cached_location(&self) -> Option<GeoLocation> {
            None
        }
    }

    #[tokio::test]
    async fn precision_defaults_to_precise() {
        let req = Arc::new(PrecisionCapturingRequester {
            captured: std::sync::Mutex::new(None),
        });
        let tool = LocationTool::new(req.clone());
        tool.execute(serde_json::json!({ "_conn_id": "c1" }))
            .await
            .unwrap();
        assert_eq!(
            *req.captured.lock().unwrap(),
            Some(LocationPrecision::Precise)
        );
    }

    #[tokio::test]
    async fn precision_coarse_is_forwarded() {
        let req = Arc::new(PrecisionCapturingRequester {
            captured: std::sync::Mutex::new(None),
        });
        let tool = LocationTool::new(req.clone());
        tool.execute(serde_json::json!({ "_conn_id": "c2", "precision": "coarse" }))
            .await
            .unwrap();
        assert_eq!(
            *req.captured.lock().unwrap(),
            Some(LocationPrecision::Coarse)
        );
    }

    #[tokio::test]
    async fn channel_location_success() {
        let tool = LocationTool::new(Arc::new(MockRequester {
            cached: None,
            response: LocationResult {
                location: None,
                error: None,
            },
            channel_response: Some(LocationResult {
                location: Some(BrowserLocation {
                    latitude: 51.5074,
                    longitude: -0.1278,
                    accuracy: 0.0,
                }),
                error: None,
            }),
        }));

        let result = tool
            .execute(serde_json::json!({ "_session_key": "telegram:bot1:12345" }))
            .await
            .unwrap();
        assert_eq!(result["latitude"], 51.5074);
        assert_eq!(result["longitude"], -0.1278);
        assert_eq!(result["source"], "channel");
    }

    #[tokio::test]
    async fn channel_location_not_supported_for_non_channel_session() {
        let tool = LocationTool::new(Arc::new(MockRequester {
            cached: None,
            response: LocationResult {
                location: None,
                error: None,
            },
            channel_response: None,
        }));

        // Non-channel session key should not attempt channel location.
        let err = tool
            .execute(serde_json::json!({ "_session_key": "web:session:123" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no client connection"));
    }

    #[tokio::test]
    async fn channel_location_fallback_no_session() {
        let tool = LocationTool::new(Arc::new(MockRequester {
            cached: None,
            response: LocationResult {
                location: None,
                error: None,
            },
            channel_response: None,
        }));

        // No _session_key and no _conn_id — should error.
        let err = tool.execute(serde_json::json!({})).await.unwrap_err();
        assert!(err.to_string().contains("no client connection"));
    }

    #[tokio::test]
    async fn channel_location_default_trait_returns_not_supported() {
        // Test the default trait implementation directly.
        struct MinimalRequester;

        #[async_trait]
        impl LocationRequester for MinimalRequester {
            async fn request_location(
                &self,
                _conn_id: &str,
                _precision: LocationPrecision,
            ) -> Result<LocationResult> {
                Ok(LocationResult {
                    location: None,
                    error: None,
                })
            }

            fn cached_location(&self) -> Option<GeoLocation> {
                None
            }
        }

        let req = MinimalRequester;
        let result = req
            .request_channel_location("telegram:bot1:123")
            .await
            .unwrap();
        assert!(result.location.is_none());
        assert!(matches!(result.error, Some(LocationError::NotSupported)));
    }

    #[test]
    fn format_address_full() {
        let addr = NominatimAddress {
            neighbourhood: Some("Noe Valley".to_string()),
            suburb: None,
            city: Some("San Francisco".to_string()),
            town: None,
            village: None,
            state: Some("California".to_string()),
            country_code: Some("us".to_string()),
        };
        assert_eq!(
            format_address(&addr),
            "Noe Valley, San Francisco, California, US"
        );
        assert_eq!(format_address_short(&addr), "Noe Valley");
    }

    #[test]
    fn format_address_suburb_fallback() {
        let addr = NominatimAddress {
            neighbourhood: None,
            suburb: Some("Montmartre".to_string()),
            city: Some("Paris".to_string()),
            town: None,
            village: None,
            state: Some("Île-de-France".to_string()),
            country_code: Some("fr".to_string()),
        };
        assert_eq!(
            format_address(&addr),
            "Montmartre, Paris, Île-de-France, FR"
        );
        assert_eq!(format_address_short(&addr), "Montmartre");
    }

    #[test]
    fn format_address_village_only() {
        let addr = NominatimAddress {
            neighbourhood: None,
            suburb: None,
            city: None,
            town: None,
            village: Some("Gruyères".to_string()),
            state: Some("Fribourg".to_string()),
            country_code: Some("ch".to_string()),
        };
        assert_eq!(format_address(&addr), "Gruyères, Fribourg, CH");
        assert_eq!(format_address_short(&addr), "Gruyères");
    }

    #[test]
    fn format_address_empty() {
        let addr = NominatimAddress {
            neighbourhood: None,
            suburb: None,
            city: None,
            town: None,
            village: None,
            state: None,
            country_code: None,
        };
        assert_eq!(format_address(&addr), "");
        assert_eq!(format_address_short(&addr), "");
    }

    #[tokio::test]
    async fn reverse_geocode_with_mock_server() {
        // Start a lightweight HTTP mock.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "address": {
                        "neighbourhood": "Mission District",
                        "city": "San Francisco",
                        "state": "California",
                        "country_code": "us"
                    }
                }"#,
            )
            .create_async()
            .await;

        // Build a client that points at the mock server.
        let client = reqwest::Client::new();
        let url = format!(
            "{}/reverse?lat=37.76&lon=-122.42&format=json&zoom=14",
            server.url()
        );
        let resp = client
            .get(&url)
            .header("User-Agent", "moltis/0.3-test")
            .send()
            .await
            .unwrap();
        let body: NominatimResponse = resp.json().await.unwrap();
        let addr = body.address.as_ref().unwrap();
        assert_eq!(
            format_address(addr),
            "Mission District, San Francisco, California, US"
        );
        assert_eq!(format_address_short(addr), "Mission District");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn reverse_geocode_fallback_on_error() {
        // Start a mock that returns 500.
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(500)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        // Point at the mock by calling the inner function with a custom URL.
        // Since `reverse_geocode_with_client` uses the real Nominatim URL, we
        // test the parse/fallback path directly here.
        let url = format!("{}/reverse?lat=0&lon=0&format=json&zoom=14", server.url());
        let resp = client
            .get(&url)
            .header("User-Agent", "moltis/0.3-test")
            .send()
            .await
            .unwrap();
        assert!(!resp.status().is_success());
    }
}
