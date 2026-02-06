//! Push notification API routes.

use {
    crate::{
        push::{PushPayload, PushService, PushSubscription},
        server::AppState,
    },
    axum::{
        Json, Router,
        extract::{ConnectInfo, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    },
    chrono::Utc,
    serde::{Deserialize, Serialize},
    std::{net::SocketAddr, sync::Arc},
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
        .add_subscription(subscription)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Broadcast subscription change
    crate::broadcast::broadcast(
        &state.gateway,
        "push.subscriptions",
        serde_json::json!({"action": "added"}),
        crate::broadcast::BroadcastOpts::default(),
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
    crate::broadcast::broadcast(
        &state.gateway,
        "push.subscriptions",
        serde_json::json!({"action": "removed"}),
        crate::broadcast::BroadcastOpts::default(),
    )
    .await;

    Ok(StatusCode::OK)
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
        .route("/status", get(status_handler))
}

/// Send a push notification to all subscribers.
pub async fn send_push_notification(
    push_service: &Arc<PushService>,
    title: &str,
    body: &str,
    url: Option<&str>,
    session_key: Option<&str>,
) -> anyhow::Result<usize> {
    let payload = PushPayload {
        title: title.to_string(),
        body: body.to_string(),
        url: url.map(String::from),
        session_key: session_key.map(String::from),
    };

    push_service.send_to_all(&payload).await
}
