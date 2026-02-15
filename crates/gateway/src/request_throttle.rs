use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use {
    axum::{
        extract::{ConnectInfo, State},
        http::{HeaderMap, Method, StatusCode},
        middleware::Next,
        response::{IntoResponse, Json, Response},
    },
    dashmap::{DashMap, mapref::entry::Entry},
};

use crate::server::AppState;

const CLEANUP_EVERY_REQUESTS: u64 = 512;

#[derive(Clone)]
pub struct RequestThrottle {
    limits: ThrottleLimits,
    buckets: Arc<DashMap<ThrottleKey, WindowState>>,
    requests_seen: Arc<AtomicU64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ThrottleScope {
    Login,
    AuthApi,
    Api,
    Share,
    Ws,
}

impl ThrottleScope {
    fn from_request(method: &Method, path: &str) -> Option<Self> {
        if path == "/api/auth/login" && method == Method::POST {
            return Some(Self::Login);
        }
        if path.starts_with("/api/auth/") {
            return Some(Self::AuthApi);
        }
        if path.starts_with("/api/") {
            return Some(Self::Api);
        }
        if path.starts_with("/share/") {
            return Some(Self::Share);
        }
        if path == "/ws" {
            return Some(Self::Ws);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ThrottleKey {
    ip: IpAddr,
    scope: ThrottleScope,
}

#[derive(Debug, Clone, Copy)]
struct WindowState {
    started_at: Instant,
    count: usize,
}

#[derive(Debug, Clone, Copy)]
struct RateLimit {
    max_requests: usize,
    window: Duration,
}

#[derive(Debug, Clone, Copy)]
struct ThrottleLimits {
    login: RateLimit,
    auth_api: RateLimit,
    api: RateLimit,
    share: RateLimit,
    ws: RateLimit,
}

impl Default for ThrottleLimits {
    fn default() -> Self {
        Self {
            // Brute-force protection for password login attempts.
            login: RateLimit {
                max_requests: 5,
                window: Duration::from_secs(60),
            },
            // Login/setup status endpoints need a bit more headroom.
            auth_api: RateLimit {
                max_requests: 120,
                window: Duration::from_secs(60),
            },
            // Normal API usage: allow sustained usage while preventing abuse.
            api: RateLimit {
                max_requests: 180,
                window: Duration::from_secs(60),
            },
            // Public share links should be accessible but protected from abuse.
            share: RateLimit {
                max_requests: 90,
                window: Duration::from_secs(60),
            },
            // Limit reconnect storms for websocket upgrades.
            ws: RateLimit {
                max_requests: 30,
                window: Duration::from_secs(60),
            },
        }
    }
}

enum ThrottleDecision {
    Allowed,
    Denied { retry_after: Duration },
}

impl RequestThrottle {
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(ThrottleLimits::default())
    }

    fn with_limits(limits: ThrottleLimits) -> Self {
        Self {
            limits,
            buckets: Arc::new(DashMap::new()),
            requests_seen: Arc::new(AtomicU64::new(0)),
        }
    }

    fn limit_for(&self, scope: ThrottleScope) -> RateLimit {
        match scope {
            ThrottleScope::Login => self.limits.login,
            ThrottleScope::AuthApi => self.limits.auth_api,
            ThrottleScope::Api => self.limits.api,
            ThrottleScope::Share => self.limits.share,
            ThrottleScope::Ws => self.limits.ws,
        }
    }

    fn check(&self, ip: IpAddr, scope: ThrottleScope) -> ThrottleDecision {
        self.check_at(ip, scope, Instant::now())
    }

    fn check_at(&self, ip: IpAddr, scope: ThrottleScope, now: Instant) -> ThrottleDecision {
        let limit = self.limit_for(scope);
        if limit.max_requests == 0 {
            return ThrottleDecision::Denied {
                retry_after: limit.window.max(Duration::from_secs(1)),
            };
        }

        let key = ThrottleKey { ip, scope };
        let decision = match self.buckets.entry(key) {
            Entry::Occupied(mut occupied) => {
                let state = occupied.get_mut();
                let elapsed = now.duration_since(state.started_at);
                if elapsed >= limit.window {
                    state.started_at = now;
                    state.count = 1;
                    ThrottleDecision::Allowed
                } else if state.count < limit.max_requests {
                    state.count += 1;
                    ThrottleDecision::Allowed
                } else {
                    ThrottleDecision::Denied {
                        retry_after: limit.window.saturating_sub(elapsed),
                    }
                }
            },
            Entry::Vacant(vacant) => {
                vacant.insert(WindowState {
                    started_at: now,
                    count: 1,
                });
                ThrottleDecision::Allowed
            },
        };

        self.cleanup_if_needed(now);
        decision
    }

    fn cleanup_if_needed(&self, now: Instant) {
        let seen = self.requests_seen.fetch_add(1, Ordering::Relaxed) + 1;
        if !seen.is_multiple_of(CLEANUP_EVERY_REQUESTS) {
            return;
        }
        let stale_after = self.max_window().saturating_mul(3);
        self.buckets
            .retain(|_, state| now.duration_since(state.started_at) <= stale_after);
    }

    fn max_window(&self) -> Duration {
        [
            self.limits.login.window,
            self.limits.auth_api.window,
            self.limits.api.window,
            self.limits.share.window,
            self.limits.ws.window,
        ]
        .into_iter()
        .max()
        .unwrap_or(Duration::from_secs(60))
    }
}

impl Default for RequestThrottle {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn throttle_gate(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let Some(scope) = ThrottleScope::from_request(&method, &path) else {
        return next.run(request).await;
    };

    if should_bypass_throttling(&state, request.headers(), addr).await {
        return next.run(request).await;
    }

    let client_ip = resolve_client_ip(request.headers(), addr, state.gateway.behind_proxy);
    match state.request_throttle.check(client_ip, scope) {
        ThrottleDecision::Allowed => next.run(request).await,
        ThrottleDecision::Denied { retry_after } => rate_limited_response(path, retry_after),
    }
}

async fn should_bypass_throttling(state: &AppState, headers: &HeaderMap, addr: SocketAddr) -> bool {
    let Some(store) = state.gateway.credential_store.as_ref() else {
        // No credential store means auth is not enforced.
        return true;
    };

    let is_local = crate::server::is_local_connection(headers, addr, state.gateway.behind_proxy);
    matches!(
        crate::auth_middleware::check_auth(store, headers, is_local).await,
        crate::auth_middleware::AuthResult::Allowed(_)
    )
}

fn rate_limited_response(path: String, retry_after: Duration) -> Response {
    let retry_after_secs = retry_after.as_secs().max(1);
    let mut response = if path.starts_with("/api/") {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": "too many requests",
                "retry_after_seconds": retry_after_secs
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::TOO_MANY_REQUESTS,
            format!("too many requests, retry after {retry_after_secs}s"),
        )
            .into_response()
    };

    if let Ok(value) = retry_after_secs.to_string().parse() {
        response
            .headers_mut()
            .insert(axum::http::header::RETRY_AFTER, value);
    }
    response
}

fn resolve_client_ip(headers: &HeaderMap, addr: SocketAddr, behind_proxy: bool) -> IpAddr {
    if behind_proxy && let Some(ip) = extract_forwarded_ip(headers) {
        return ip;
    }
    addr.ip()
}

fn extract_forwarded_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let xff = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok());
    if let Some(xff) = xff
        && let Some(ip) = xff
            .split(',')
            .find_map(|candidate| parse_ip(candidate.trim()))
    {
        return Some(ip);
    }

    let xri = headers.get("x-real-ip").and_then(|v| v.to_str().ok());
    if let Some(xri) = xri
        && let Some(ip) = parse_ip(xri.trim())
    {
        return Some(ip);
    }

    let cf_ip = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok());
    if let Some(cf_ip) = cf_ip
        && let Some(ip) = parse_ip(cf_ip.trim())
    {
        return Some(ip);
    }

    None
}

fn parse_ip(value: &str) -> Option<IpAddr> {
    if value.is_empty() {
        return None;
    }
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Some(addr.ip());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_login_request() {
        assert_eq!(
            ThrottleScope::from_request(&Method::POST, "/api/auth/login"),
            Some(ThrottleScope::Login)
        );
    }

    #[test]
    fn classify_general_api_request() {
        assert_eq!(
            ThrottleScope::from_request(&Method::GET, "/api/bootstrap"),
            Some(ThrottleScope::Api)
        );
    }

    #[test]
    fn classify_ws_request() {
        assert_eq!(
            ThrottleScope::from_request(&Method::GET, "/ws"),
            Some(ThrottleScope::Ws)
        );
    }

    #[test]
    fn classify_share_request() {
        assert_eq!(
            ThrottleScope::from_request(&Method::GET, "/share/abc123"),
            Some(ThrottleScope::Share)
        );
    }

    #[test]
    fn login_window_limits_requests() {
        let throttle = RequestThrottle::with_limits(ThrottleLimits {
            login: RateLimit {
                max_requests: 2,
                window: Duration::from_secs(10),
            },
            auth_api: RateLimit {
                max_requests: 100,
                window: Duration::from_secs(10),
            },
            api: RateLimit {
                max_requests: 100,
                window: Duration::from_secs(10),
            },
            share: RateLimit {
                max_requests: 100,
                window: Duration::from_secs(10),
            },
            ws: RateLimit {
                max_requests: 100,
                window: Duration::from_secs(10),
            },
        });

        let ip = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
        let now = Instant::now();

        assert!(matches!(
            throttle.check_at(ip, ThrottleScope::Login, now),
            ThrottleDecision::Allowed
        ));
        assert!(matches!(
            throttle.check_at(ip, ThrottleScope::Login, now),
            ThrottleDecision::Allowed
        ));

        let decision = throttle.check_at(ip, ThrottleScope::Login, now);
        match decision {
            ThrottleDecision::Denied { retry_after } => {
                assert_eq!(retry_after, Duration::from_secs(10));
            },
            ThrottleDecision::Allowed => panic!("expected third request to be throttled"),
        }

        assert!(matches!(
            throttle.check_at(ip, ThrottleScope::Login, now + Duration::from_secs(11)),
            ThrottleDecision::Allowed
        ));
    }

    #[test]
    fn forwarded_ip_uses_first_xff_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            axum::http::HeaderValue::from_static("203.0.113.1, 198.51.100.9"),
        );
        assert_eq!(
            extract_forwarded_ip(&headers),
            Some(IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 1)))
        );
    }
}
