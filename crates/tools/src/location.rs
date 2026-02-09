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
};

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
    async fn request_location(&self, conn_id: &str) -> Result<LocationResult>;

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
        "Get the user's current geographic location (latitude/longitude). \
         Requires user permission via browser popup. Use when the user asks \
         about local weather, nearby places, directions, or anything \
         location-dependent. Returns cached location if already obtained."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        // Fast path: return cached location.
        if let Some(loc) = self.requester.cached_location() {
            return Ok(serde_json::json!({
                "latitude": loc.latitude,
                "longitude": loc.longitude,
                "source": "cached"
            }));
        }

        // Try browser geolocation if a connection ID is available.
        if let Some(conn_id) = params.get("_conn_id").and_then(|v| v.as_str()) {
            let result = self.requester.request_location(conn_id).await?;
            return match result.location {
                Some(loc) => Ok(serde_json::json!({
                    "latitude": loc.latitude,
                    "longitude": loc.longitude,
                    "accuracy_meters": loc.accuracy,
                    "source": "browser"
                })),
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
                Some(loc) => Ok(serde_json::json!({
                    "latitude": loc.latitude,
                    "longitude": loc.longitude,
                    "accuracy_meters": loc.accuracy,
                    "source": "channel"
                })),
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
        async fn request_location(&self, _conn_id: &str) -> Result<LocationResult> {
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
            async fn request_location(&self, _conn_id: &str) -> Result<LocationResult> {
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
}
