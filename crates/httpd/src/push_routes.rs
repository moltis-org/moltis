//! Push notification API routes.

use {
    crate::server::AppState,
    axum::{
        Json, Router,
        extract::{ConnectInfo, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    },
    chrono::Utc,
    moltis_gateway::push::PushSubscription,
    serde::{Deserialize, Serialize},
    std::net::SocketAddr,
};

/// Response with the VAPID public key.
#[derive(Serialize)]
struct VapidKeyResponse {
    public_key: String,
}

/// Request to subscribe to push notifications.
#[derive(Deserialize)]
pub struct SubscribeRequest {
    pub endpoint: String,
    pub keys: SubscriptionKeys,
    /// Endpoint this subscription supersedes, sent when the browser rotates a
    /// subscription via `pushsubscriptionchange`.
    #[serde(default)]
    pub replaces: Option<String>,
}

/// Request reporting which session a subscribed device is currently viewing.
#[derive(Deserialize)]
pub struct PresenceRequest {
    pub endpoint: String,
    #[serde(default)]
    pub session_key: Option<String>,
    pub visible: bool,
}

#[derive(Deserialize)]
pub struct SubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

/// Request to unsubscribe from push notifications.
#[derive(Deserialize)]
pub struct UnsubscribeRequest {
    pub endpoint: String,
}

/// A subscription summary for display.
#[derive(Serialize)]
struct SubscriptionSummary {
    /// The full subscription endpoint (for deletion).
    endpoint: String,
    /// Parsed device name from user agent.
    device: String,
    /// Client IP address.
    ip: Option<String>,
    /// When the subscription was created (ISO 8601).
    created_at: String,
}

/// Status response.
#[derive(Serialize)]
struct PushStatusResponse {
    enabled: bool,
    subscription_count: usize,
    subscriptions: Vec<SubscriptionSummary>,
}

/// Get the VAPID public key for push subscription.
async fn vapid_key_handler(
    State(state): State<AppState>,
) -> Result<Json<VapidKeyResponse>, StatusCode> {
    let Some(ref push_service) = state.push_service else {
        return Err(StatusCode::NOT_IMPLEMENTED);
    };

    let public_key = push_service
        .vapid_public_key()
        .await
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(VapidKeyResponse { public_key }))
}

/// Extract the client IP from headers (for proxies) or connection info.
fn extract_client_ip(headers: &HeaderMap, conn_addr: SocketAddr) -> String {
    // Check X-Forwarded-For first (may contain multiple IPs, take the first/leftmost)
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first_ip) = xff.split(',').next()
    {
        let ip = first_ip.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }

    // Check X-Real-IP (common with nginx)
    if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = xri.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }

    // Check CF-Connecting-IP (Cloudflare)
    if let Some(cf_ip) = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
    {
        let ip = cf_ip.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }

    // Fall back to connection address
    conn_addr.ip().to_string()
}

/// Subscribe to push notifications.
async fn subscribe_handler(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<SubscribeRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let Some(ref push_service) = state.push_service else {
        return Err(StatusCode::NOT_IMPLEMENTED);
    };

    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let ip_address = Some(extract_client_ip(&headers, addr));

    let subscription = PushSubscription {
        endpoint: req.endpoint,
        p256dh: req.keys.p256dh,
        auth: req.keys.auth,
        user_agent,
        ip_address,
        created_at: Utc::now(),
    };

    push_service
        .add_subscription(subscription, req.replaces.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Broadcast subscription change
    moltis_gateway::broadcast::broadcast(
        &state.gateway,
        "push.subscriptions",
        serde_json::json!({"action": "added"}),
        moltis_gateway::broadcast::BroadcastOpts::default(),
    )
    .await;

    Ok(StatusCode::CREATED)
}

/// Unsubscribe from push notifications.
async fn unsubscribe_handler(
    State(state): State<AppState>,
    Json(req): Json<UnsubscribeRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let Some(ref push_service) = state.push_service else {
        return Err(StatusCode::NOT_IMPLEMENTED);
    };

    push_service
        .remove_subscription(&req.endpoint)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Broadcast subscription change
    moltis_gateway::broadcast::broadcast(
        &state.gateway,
        "push.subscriptions",
        serde_json::json!({"action": "removed"}),
        moltis_gateway::broadcast::BroadcastOpts::default(),
    )
    .await;

    Ok(StatusCode::OK)
}

/// Result of sending a test notification.
#[derive(Serialize)]
struct TestNotificationResponse {
    /// Number of devices the push was accepted for.
    sent: usize,
}

/// Send a test notification to every subscribed device.
///
/// Push failures are otherwise invisible — the browser, the push service, and
/// the network all fail silently — so this gives users a way to prove the path
/// works end to end.
async fn test_handler(
    State(state): State<AppState>,
) -> Result<Json<TestNotificationResponse>, StatusCode> {
    let Some(ref push_service) = state.push_service else {
        return Err(StatusCode::NOT_IMPLEMENTED);
    };

    // No session key: a test must reach every device, including the one the
    // user is holding, so it must not be suppressed by presence.
    let payload = moltis_gateway::push::PushPayload::new(
        "moltis",
        "Test notification — push is working.",
        Some("/settings/notifications".to_string()),
        None,
    );

    let sent = push_service
        .send_to_all(&payload)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(TestNotificationResponse { sent }))
}

/// Report which session this device is currently viewing.
///
/// Used to suppress push notifications on the device the user is already
/// reading the conversation on.
async fn presence_handler(
    State(state): State<AppState>,
    Json(req): Json<PresenceRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let Some(ref push_service) = state.push_service else {
        return Err(StatusCode::NOT_IMPLEMENTED);
    };

    let recorded = push_service
        .record_presence(&req.endpoint, req.session_key, req.visible)
        .await;

    // An unknown endpoint means the client's subscription is stale; telling it
    // so lets the page re-register instead of silently losing suppression.
    if recorded {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// Parse a user agent string into a friendly device name.
fn parse_device_name(user_agent: Option<&str>) -> String {
    let ua = match user_agent {
        Some(s) if !s.is_empty() => s,
        _ => return "Unknown device".to_string(),
    };

    // Check for mobile devices first
    if ua.contains("iPhone") {
        return "iPhone".to_string();
    }
    if ua.contains("iPad") {
        return "iPad".to_string();
    }
    if ua.contains("Android") {
        if ua.contains("Mobile") {
            return "Android Phone".to_string();
        }
        return "Android Tablet".to_string();
    }

    // Desktop browsers
    let os = if ua.contains("Macintosh") || ua.contains("Mac OS") {
        "macOS"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Linux") {
        "Linux"
    } else if ua.contains("CrOS") {
        "ChromeOS"
    } else {
        ""
    };

    let browser = if ua.contains("Safari") && !ua.contains("Chrome") && !ua.contains("Chromium") {
        "Safari"
    } else if ua.contains("Firefox") {
        "Firefox"
    } else if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome") {
        "Chrome"
    } else {
        ""
    };

    match (os, browser) {
        ("", "") => "Unknown device".to_string(),
        (os, "") => os.to_string(),
        ("", browser) => browser.to_string(),
        (os, browser) => format!("{browser} on {os}"),
    }
}

/// Get push notification status.
async fn status_handler(State(state): State<AppState>) -> Json<PushStatusResponse> {
    let (enabled, subscription_count, subscriptions) =
        if let Some(ref push_service) = state.push_service {
            let subs = push_service.list_subscriptions().await;
            let count = subs.len();
            let summaries: Vec<SubscriptionSummary> = subs
                .into_iter()
                .map(|s| SubscriptionSummary {
                    endpoint: s.endpoint,
                    device: parse_device_name(s.user_agent.as_deref()),
                    ip: s.ip_address,
                    created_at: s.created_at.to_rfc3339(),
                })
                .collect();
            (true, count, summaries)
        } else {
            (false, 0, Vec::new())
        };

    Json(PushStatusResponse {
        enabled,
        subscription_count,
        subscriptions,
    })
}

/// Create the push notification router.
pub fn push_router() -> Router<AppState> {
    Router::new()
        .route("/vapid-key", get(vapid_key_handler))
        .route("/subscribe", post(subscribe_handler))
        .route("/unsubscribe", post(unsubscribe_handler))
        .route("/presence", post(presence_handler))
        .route("/test", post(test_handler))
        .route("/status", get(status_handler))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        axum::body::to_bytes,
        moltis_gateway::{
            auth, methods::MethodRegistry, push::PushService, services::GatewayServices,
            state::GatewayState,
        },
        std::sync::Arc,
    };

    #[cfg(feature = "ngrok")]
    use std::sync::Weak;

    #[cfg(feature = "netbird")]
    use crate::server::NetbirdController;
    #[cfg(feature = "ngrok")]
    use crate::server::NgrokRuntimeStatus;
    use crate::server::{AppState, CloudflareTunnelController};

    use super::*;

    // ── Device name parsing ─────────────────────────────────────────────────

    #[test]
    fn device_name_recognises_mobile_devices() {
        assert_eq!(parse_device_name(Some("... iPhone ...")), "iPhone");
        assert_eq!(parse_device_name(Some("... iPad ...")), "iPad");
        assert_eq!(
            parse_device_name(Some("... Android ... Mobile ...")),
            "Android Phone"
        );
        assert_eq!(parse_device_name(Some("... Android ...")), "Android Tablet");
    }

    #[test]
    fn device_name_combines_browser_and_os() {
        assert_eq!(
            parse_device_name(Some("Mozilla/5.0 (Macintosh) Safari/605")),
            "Safari on macOS"
        );
        assert_eq!(
            parse_device_name(Some("Mozilla/5.0 (Windows NT 10.0) Chrome/120")),
            "Chrome on Windows"
        );
        assert_eq!(
            parse_device_name(Some("Mozilla/5.0 (X11; Linux) Firefox/121")),
            "Firefox on Linux"
        );
        assert_eq!(
            parse_device_name(Some("Mozilla/5.0 (Windows NT 10.0) Chrome/120 Edg/120")),
            "Edge on Windows"
        );
    }

    #[test]
    fn device_name_falls_back_when_unknown_or_missing() {
        assert_eq!(parse_device_name(None), "Unknown device");
        assert_eq!(parse_device_name(Some("")), "Unknown device");
        assert_eq!(parse_device_name(Some("curl/8.0")), "Unknown device");
    }

    // ── Client IP extraction ────────────────────────────────────────────────

    fn addr() -> SocketAddr {
        "10.0.0.1:5000".parse().expect("addr")
    }

    #[test]
    fn client_ip_prefers_leftmost_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.5, 70.41.3.18".parse().unwrap(),
        );
        assert_eq!(extract_client_ip(&headers, addr()), "203.0.113.5");
    }

    #[test]
    fn client_ip_falls_through_proxy_headers_in_order() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "198.51.100.7".parse().unwrap());
        assert_eq!(extract_client_ip(&headers, addr()), "198.51.100.7");

        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "198.51.100.9".parse().unwrap());
        assert_eq!(extract_client_ip(&headers, addr()), "198.51.100.9");
    }

    #[test]
    fn client_ip_ignores_blank_headers_and_uses_the_connection() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "   ".parse().unwrap());
        assert_eq!(extract_client_ip(&headers, addr()), "10.0.0.1");
        assert_eq!(extract_client_ip(&HeaderMap::new(), addr()), "10.0.0.1");
    }

    // ── Handlers ────────────────────────────────────────────────────────────

    fn test_state(push_service: Option<Arc<PushService>>) -> AppState {
        let gateway = GatewayState::new(auth::resolve_auth(None, None), GatewayServices::noop());
        let cloudflare_tunnel_runtime = Arc::new(tokio::sync::RwLock::new(None));
        #[cfg(feature = "netbird")]
        let netbird_runtime = Arc::new(tokio::sync::RwLock::new(None));

        AppState {
            gateway: Arc::clone(&gateway),
            methods: Arc::new(MethodRegistry::new()),
            request_throttle: Arc::new(crate::request_throttle::RequestThrottle::new()),
            webauthn_registry: None,
            #[cfg(feature = "ngrok")]
            ngrok_controller_owner: None,
            #[cfg(feature = "ngrok")]
            ngrok_controller: Weak::new(),
            #[cfg(feature = "ngrok")]
            ngrok_runtime: Arc::new(tokio::sync::RwLock::new(None::<NgrokRuntimeStatus>)),
            cloudflare_tunnel_controller: Arc::new(CloudflareTunnelController::new(
                Arc::clone(&gateway),
                None,
                Arc::clone(&cloudflare_tunnel_runtime),
            )),
            cloudflare_tunnel_runtime,
            #[cfg(feature = "netbird")]
            netbird_controller: Arc::new(NetbirdController::new(Arc::clone(&netbird_runtime))),
            #[cfg(feature = "netbird")]
            netbird_runtime,
            #[cfg(feature = "tailscale")]
            tailscale_manager: moltis_gateway::tailscale::CachedTailscaleManager::new_with_prefetch(
            ),
            push_service,
            #[cfg(feature = "graphql")]
            graphql_schema: crate::graphql_routes::build_graphql_schema(GatewayState::new(
                auth::resolve_auth(None, None),
                GatewayServices::noop(),
            )),
        }
    }

    async fn state_with_push() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = PushService::new(dir.path()).await.expect("push service");
        (test_state(Some(service)), dir)
    }

    fn subscribe_request(endpoint: &str, replaces: Option<&str>) -> SubscribeRequest {
        SubscribeRequest {
            endpoint: endpoint.to_string(),
            keys: SubscriptionKeys {
                p256dh: "p256dh".to_string(),
                auth: "auth".to_string(),
            },
            replaces: replaces.map(ToString::to_string),
        }
    }

    #[tokio::test]
    async fn routes_report_not_implemented_without_a_push_service() {
        let state = test_state(None);

        assert_eq!(
            vapid_key_handler(State(state.clone())).await.err(),
            Some(StatusCode::NOT_IMPLEMENTED)
        );
        assert_eq!(
            presence_handler(
                State(state.clone()),
                Json(PresenceRequest {
                    endpoint: "https://push.example/a".into(),
                    session_key: None,
                    visible: true,
                }),
            )
            .await
            .err(),
            Some(StatusCode::NOT_IMPLEMENTED)
        );
        assert_eq!(
            test_handler(State(state)).await.err(),
            Some(StatusCode::NOT_IMPLEMENTED)
        );
    }

    #[tokio::test]
    async fn status_reports_disabled_without_a_push_service() {
        let response = status_handler(State(test_state(None))).await;
        assert!(!response.0.enabled);
        assert_eq!(response.0.subscription_count, 0);
        assert!(response.0.subscriptions.is_empty());
    }

    #[tokio::test]
    async fn subscribe_then_status_lists_the_device() {
        let (state, _dir) = state_with_push().await;
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "Mozilla/5.0 (iPhone) Safari".parse().unwrap());
        headers.insert("x-forwarded-for", "203.0.113.5".parse().unwrap());

        let response = subscribe_handler(
            State(state.clone()),
            ConnectInfo(addr()),
            headers,
            Json(subscribe_request("https://push.example/a", None)),
        )
        .await
        .expect("subscribe")
        .into_response();
        assert_eq!(response.status(), StatusCode::CREATED);

        let status = status_handler(State(state)).await;
        assert!(status.0.enabled);
        assert_eq!(status.0.subscription_count, 1);
        let entry = &status.0.subscriptions[0];
        assert_eq!(entry.device, "iPhone");
        assert_eq!(entry.ip.as_deref(), Some("203.0.113.5"));
        assert_eq!(entry.endpoint, "https://push.example/a");
    }

    #[tokio::test]
    async fn subscribing_with_replaces_retires_the_rotated_endpoint() {
        let (state, _dir) = state_with_push().await;

        for endpoint in ["https://push.example/old", "https://push.example/other"] {
            subscribe_handler(
                State(state.clone()),
                ConnectInfo(addr()),
                HeaderMap::new(),
                Json(subscribe_request(endpoint, None)),
            )
            .await
            .expect("subscribe");
        }

        subscribe_handler(
            State(state.clone()),
            ConnectInfo(addr()),
            HeaderMap::new(),
            Json(subscribe_request(
                "https://push.example/new",
                Some("https://push.example/old"),
            )),
        )
        .await
        .expect("subscribe");

        let status = status_handler(State(state)).await;
        let endpoints: Vec<&str> = status
            .0
            .subscriptions
            .iter()
            .map(|s| s.endpoint.as_str())
            .collect();
        assert!(!endpoints.contains(&"https://push.example/old"));
        assert!(endpoints.contains(&"https://push.example/new"));
        assert!(
            endpoints.contains(&"https://push.example/other"),
            "replacing one endpoint must not disturb the others"
        );
    }

    #[tokio::test]
    async fn presence_is_accepted_for_known_endpoints_and_rejected_otherwise() {
        let (state, _dir) = state_with_push().await;

        // An endpoint the server never stored is how a client learns its
        // subscription is stale, so it must not be silently accepted.
        let unknown = presence_handler(
            State(state.clone()),
            Json(PresenceRequest {
                endpoint: "https://push.example/unknown".into(),
                session_key: Some("main".into()),
                visible: true,
            }),
        )
        .await;
        assert_eq!(unknown.err(), Some(StatusCode::NOT_FOUND));

        subscribe_handler(
            State(state.clone()),
            ConnectInfo(addr()),
            HeaderMap::new(),
            Json(subscribe_request("https://push.example/a", None)),
        )
        .await
        .expect("subscribe");

        let known = presence_handler(
            State(state),
            Json(PresenceRequest {
                endpoint: "https://push.example/a".into(),
                session_key: Some("main".into()),
                visible: true,
            }),
        )
        .await
        .expect("presence")
        .into_response();
        assert_eq!(known.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn unsubscribe_removes_the_device() {
        let (state, _dir) = state_with_push().await;
        subscribe_handler(
            State(state.clone()),
            ConnectInfo(addr()),
            HeaderMap::new(),
            Json(subscribe_request("https://push.example/a", None)),
        )
        .await
        .expect("subscribe");

        let response = unsubscribe_handler(
            State(state.clone()),
            Json(UnsubscribeRequest {
                endpoint: "https://push.example/a".into(),
            }),
        )
        .await
        .expect("unsubscribe")
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(status_handler(State(state)).await.0.subscription_count, 0);
    }

    #[tokio::test]
    async fn test_notification_reports_zero_without_subscribers() {
        let (state, _dir) = state_with_push().await;

        let response = test_handler(State(state))
            .await
            .expect("test")
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["sent"], 0);
    }
}
