use super::*;

// ── Handlers ─────────────────────────────────────────────────────────────────

#[cfg(test)]
fn should_prebuild_sandbox_image(
    mode: &moltis_tools::sandbox::SandboxMode,
    packages: &[String],
) -> bool {
    !matches!(mode, moltis_tools::sandbox::SandboxMode::Off) && !packages.is_empty()
}

pub(super) async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let count = state.gateway.client_count().await;
    Json(serde_json::json!({
        "status": "ok",
        "version": state.gateway.version,
        "protocol": moltis_protocol::PROTOCOL_VERSION,
        "connections": count,
    }))
}

pub(super) async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // ── CSWSH protection ────────────────────────────────────────────────
    // Reject cross-origin WebSocket upgrades.  Browsers always send an
    // Origin header on cross-origin requests; non-browser clients (CLI,
    // SDKs) typically omit it — those are allowed through.
    if let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        let host = websocket_origin_host(&headers, state.gateway.behind_proxy).unwrap_or_default();
        if !is_same_origin(origin, &host) {
            tracing::warn!(
                origin,
                host = %host,
                remote = %addr,
                "rejected cross-origin WebSocket upgrade"
            );
            return (
                StatusCode::FORBIDDEN,
                "cross-origin WebSocket connections are not allowed",
            )
                .into_response();
        }
    }

    let accept_language = headers
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // Extract the real client IP (respecting proxy headers) and only keep it
    // when it resolves to a public address — private/loopback IPs are not useful
    // for the LLM to reason about locale or location.
    let remote_ip = extract_ws_client_ip(&headers, addr).filter(|ip| is_public_ip(ip));

    let is_local = is_local_connection(&headers, addr, state.gateway.behind_proxy);
    let header_identity =
        websocket_header_authenticate(&headers, state.gateway.credential_store.as_ref(), is_local)
            .await;
    ws.on_upgrade(move |socket| {
        handle_connection(
            socket,
            state.gateway,
            state.methods,
            addr,
            accept_language,
            remote_ip,
            header_identity,
            is_local,
        )
    })
    .into_response()
}

pub(super) fn websocket_origin_host(
    headers: &axum::http::HeaderMap,
    behind_proxy: bool,
) -> Option<String> {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    if !behind_proxy {
        return host;
    }
    headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or(host)
}

/// Dedicated host terminal WebSocket stream (`Settings > Terminal`).
/// Extract the client IP from proxy headers, falling back to the direct connection address.
pub(super) fn extract_ws_client_ip(
    headers: &axum::http::HeaderMap,
    conn_addr: SocketAddr,
) -> Option<String> {
    // X-Forwarded-For (may contain multiple IPs — take the leftmost/client IP)
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first_ip) = xff.split(',').next()
    {
        let ip = first_ip.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }

    // X-Real-IP (common with nginx)
    if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = xri.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }

    // CF-Connecting-IP (Cloudflare)
    if let Some(cf_ip) = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
    {
        let ip = cf_ip.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }

    Some(conn_addr.ip().to_string())
}

/// Returns `true` if the IP string parses to a public (non-private, non-loopback) address.
pub(super) fn is_public_ip(ip: &str) -> bool {
    use std::net::IpAddr;
    let Ok(addr) = ip.parse::<IpAddr>() else {
        return false;
    };
    match addr {
        IpAddr::V4(v4) => {
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                // 100.64.0.0/10 (CGNAT)
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
                // 192.0.0.0/24
                || (v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 0))
        },
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 (unique local)
                || (v6.segments()[0] & 0xFE00) == 0xFC00
                // fe80::/10 (link-local)
                || (v6.segments()[0] & 0xFFC0) == 0xFE80)
        },
    }
}

pub(crate) use moltis_auth::locality::is_local_connection;

pub(super) async fn websocket_header_authenticate(
    headers: &axum::http::HeaderMap,
    credential_store: Option<&Arc<auth::CredentialStore>>,
    is_local: bool,
) -> Option<auth::AuthIdentity> {
    let store = credential_store?;

    match crate::auth_middleware::check_auth(store, headers, is_local).await {
        crate::auth_middleware::AuthResult::Allowed(identity) => Some(identity),
        _ => None,
    }
}

/// Resolve the machine's primary outbound IP address.
///
/// Connects a UDP socket to a public DNS address (no traffic is sent) and
/// reads back the local address the OS chose.  Returns `None` when no
/// routable interface is available.
pub(super) fn resolve_outbound_ip(ipv6: bool) -> Option<std::net::IpAddr> {
    use std::net::UdpSocket;
    let (bind, target) = if ipv6 {
        (":::0", "[2001:4860:4860::8888]:80")
    } else {
        ("0.0.0.0:0", "8.8.8.8:80")
    };
    let socket = UdpSocket::bind(bind).ok()?;
    socket.connect(target).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

#[cfg(feature = "tls")]
pub(super) fn tls_runtime_sans(bind: &str) -> Vec<moltis_tls::ServerSan> {
    let normalized = bind.trim().trim_end_matches('.');
    if normalized.is_empty() {
        return Vec::new();
    }

    if let Ok(ip) = normalized.parse::<std::net::IpAddr>() {
        if ip.is_unspecified() {
            // For wildcard binds we can only infer one "best" reachable IP
            // from the current routing table, which fixes the common single-LAN
            // case but still cannot cover every interface on multi-homed hosts.
            return resolve_outbound_ip(ip.is_ipv6())
                .filter(|resolved| !resolved.is_loopback() && !resolved.is_unspecified())
                .map(moltis_tls::ServerSan::Ip)
                .into_iter()
                .collect();
        }

        if !ip.is_loopback() {
            return vec![moltis_tls::ServerSan::Ip(ip)];
        }

        return Vec::new();
    }

    if matches!(normalized, "localhost") || normalized.ends_with(".localhost") {
        Vec::new()
    } else {
        vec![moltis_tls::ServerSan::Dns(normalized.to_ascii_lowercase())]
    }
}

pub(super) fn startup_bind_line(addr: SocketAddr) -> String {
    format!("bind (--bind): {addr}")
}

pub(super) fn startup_passkey_origin_lines(origins: &[String]) -> Vec<String> {
    origins
        .iter()
        .map(|origin| format!("passkey origin: {origin}"))
        .collect()
}

pub(super) fn startup_setup_code_lines(code: &str) -> Vec<String> {
    vec![
        String::new(),
        format!("setup code: {code}"),
        "enter this code to set your password or register a passkey".to_string(),
        String::new(),
    ]
}

/// Check whether a WebSocket `Origin` header matches the request `Host`.
///
/// Extracts the host portion of the origin URL and compares it to the Host
/// header.  Accepts `localhost`, `127.0.0.1`, and `[::1]` interchangeably
/// so that `http://localhost:8080` matches a Host of `127.0.0.1:8080`.
pub fn is_same_origin(origin: &str, host: &str) -> bool {
    fn default_port_for_scheme(scheme: &str) -> Option<&'static str> {
        match scheme {
            "http" | "ws" => Some("80"),
            "https" | "wss" => Some("443"),
            _ => None,
        }
    }

    let origin_scheme = origin
        .split("://")
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    // Origin is a full URL (e.g. "https://localhost:8080"), Host is just
    // "host:port" or "host".
    let origin_host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or("");

    fn strip_port(h: &str) -> &str {
        if h.starts_with('[') {
            // IPv6: [::1]:port
            h.rsplit_once("]:")
                .map_or(h, |(addr, _)| addr)
                .trim_start_matches('[')
                .trim_end_matches(']')
        } else {
            h.rsplit_once(':').map_or(h, |(addr, _)| addr)
        }
    }
    fn get_port(h: &str) -> Option<&str> {
        if h.starts_with('[') {
            h.rsplit_once("]:").map(|(_, p)| p)
        } else {
            h.rsplit_once(':').map(|(_, p)| p)
        }
    }

    let origin_port = get_port(origin_host).or_else(|| default_port_for_scheme(&origin_scheme));
    let host_port = get_port(host).or_else(|| default_port_for_scheme(&origin_scheme));

    let oh = strip_port(origin_host);
    let hh = strip_port(host);

    // Normalise loopback variants so 127.0.0.1 == localhost == ::1.
    // Subdomains of .localhost (e.g. moltis.localhost) are also loopback per RFC 6761.
    let is_loopback =
        |h: &str| matches!(h, "localhost" | "127.0.0.1" | "::1") || h.ends_with(".localhost");

    (oh == hh || (is_loopback(oh) && is_loopback(hh))) && origin_port == host_port
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_origin_exact_match() {
        assert!(is_same_origin(
            "https://example.com:8080",
            "example.com:8080"
        ));
        assert!(is_same_origin(
            "http://example.com:3000",
            "example.com:3000"
        ));
    }

    #[test]
    fn same_origin_treats_default_ports_as_equivalent() {
        assert!(is_same_origin("https://example.com", "example.com:443"));
        assert!(is_same_origin("https://example.com:443", "example.com"));
        assert!(is_same_origin("http://example.com", "example.com:80"));
        assert!(is_same_origin("http://example.com:80", "example.com"));
    }

    #[test]
    fn same_origin_localhost_variants() {
        // localhost ↔ 127.0.0.1
        assert!(is_same_origin("http://localhost:8080", "127.0.0.1:8080"));
        assert!(is_same_origin("https://127.0.0.1:8080", "localhost:8080"));
        // localhost ↔ ::1
        assert!(is_same_origin("http://localhost:8080", "[::1]:8080"));
        assert!(is_same_origin("http://[::1]:8080", "localhost:8080"));
        // 127.0.0.1 ↔ ::1
        assert!(is_same_origin("http://127.0.0.1:8080", "[::1]:8080"));
    }

    #[test]
    fn cross_origin_rejected() {
        // Different host
        assert!(!is_same_origin("https://attacker.com", "localhost:8080"));
        assert!(!is_same_origin("https://evil.com:8080", "localhost:8080"));
        // Different port
        assert!(!is_same_origin("http://localhost:9999", "localhost:8080"));
    }

    #[test]
    fn same_origin_no_port() {
        assert!(is_same_origin("https://example.com", "example.com"));
        assert!(is_same_origin("http://localhost", "localhost"));
        assert!(is_same_origin("http://localhost", "127.0.0.1"));
    }

    #[test]
    fn cross_origin_port_mismatch() {
        // One has port, other doesn't — different origins.
        assert!(!is_same_origin("http://localhost:8080", "localhost"));
        assert!(!is_same_origin("http://localhost", "localhost:8080"));
    }

    // share_labels and share_social_image tests moved to share_render::tests

    // share_template, map_share_message_views tests moved to share_render::tests

    #[test]
    fn same_origin_moltis_localhost() {
        // moltis.localhost ↔ localhost loopback variants
        assert!(is_same_origin(
            "https://moltis.localhost:8080",
            "localhost:8080"
        ));
        assert!(is_same_origin(
            "https://moltis.localhost:8080",
            "127.0.0.1:8080"
        ));
        assert!(is_same_origin(
            "http://localhost:8080",
            "moltis.localhost:8080"
        ));
        // Any .localhost subdomain is treated as loopback (RFC 6761).
        assert!(is_same_origin(
            "https://app.moltis.localhost:8080",
            "localhost:8080"
        ));
    }

    #[test]
    fn websocket_origin_host_prefers_forwarded_host_when_behind_proxy() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "127.0.0.1:13131".parse().unwrap());
        headers.insert("x-forwarded-host", "chat.example.com".parse().unwrap());
        assert_eq!(
            websocket_origin_host(&headers, true).as_deref(),
            Some("chat.example.com")
        );
    }

    #[test]
    fn websocket_origin_host_uses_host_without_proxy_mode() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "gateway.example.com:8443".parse().unwrap(),
        );
        headers.insert("x-forwarded-host", "chat.example.com".parse().unwrap());
        assert_eq!(
            websocket_origin_host(&headers, false).as_deref(),
            Some("gateway.example.com:8443")
        );
    }

    #[test]
    fn prebuild_runs_only_when_mode_enabled_and_packages_present() {
        let packages = vec!["curl".to_string()];
        assert!(should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::All,
            &packages
        ));
        assert!(should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::NonMain,
            &packages
        ));
        assert!(!should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::Off,
            &packages
        ));
        assert!(!should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::All,
            &[]
        ));
    }

    #[test]
    fn resolve_outbound_ip_returns_non_loopback() {
        // This test requires network connectivity; skip gracefully otherwise.
        if let Some(ip) = resolve_outbound_ip(false) {
            assert!(!ip.is_loopback(), "expected a non-loopback IP, got {ip}");
            assert!(!ip.is_unspecified(), "expected a routable IP, got {ip}");
        }
    }

    #[test]
    fn display_host_uses_real_ip_for_unspecified_bind() {
        let addr: SocketAddr = "0.0.0.0:9999".parse().unwrap();
        assert!(addr.ip().is_unspecified());

        if let Some(ip) = resolve_outbound_ip(false) {
            let display = SocketAddr::new(ip, addr.port());
            assert!(!display.ip().is_unspecified());
            assert_eq!(display.port(), 9999);
        }
    }

    #[cfg(feature = "tls")]
    #[test]
    fn tls_runtime_sans_uses_dns_for_non_localhost_names() {
        assert_eq!(tls_runtime_sans("gateway.local"), vec![
            moltis_tls::ServerSan::Dns("gateway.local".to_string())
        ]);
    }

    #[cfg(feature = "tls")]
    #[test]
    fn tls_runtime_sans_uses_ip_for_concrete_non_loopback_bind() {
        assert_eq!(tls_runtime_sans("192.168.1.9"), vec![
            moltis_tls::ServerSan::Ip("192.168.1.9".parse().unwrap())
        ]);
    }

    #[cfg(feature = "tls")]
    #[test]
    fn tls_runtime_sans_uses_ip_for_concrete_non_loopback_ipv6_bind() {
        assert_eq!(tls_runtime_sans("2001:db8::42"), vec![
            moltis_tls::ServerSan::Ip("2001:db8::42".parse().unwrap())
        ]);
    }

    #[cfg(feature = "tls")]
    #[test]
    fn tls_runtime_sans_skips_loopback_hosts() {
        assert!(tls_runtime_sans("127.0.0.1").is_empty());
        assert!(tls_runtime_sans("::1").is_empty());
        assert!(tls_runtime_sans("localhost").is_empty());
        assert!(tls_runtime_sans("moltis.localhost").is_empty());
    }

    #[cfg(feature = "tls")]
    #[test]
    fn tls_runtime_sans_wildcard_bind_uses_resolved_outbound_ip_when_available() {
        let sans = tls_runtime_sans("0.0.0.0");
        if let Some(ip) =
            resolve_outbound_ip(false).filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        {
            assert_eq!(sans, vec![moltis_tls::ServerSan::Ip(ip)]);
        } else {
            assert!(sans.is_empty());
        }
    }

    #[cfg(feature = "tls")]
    #[test]
    fn tls_runtime_sans_ipv6_wildcard_bind_uses_resolved_outbound_ip_when_available() {
        let sans = tls_runtime_sans("::");
        if let Some(ip) =
            resolve_outbound_ip(true).filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        {
            assert_eq!(sans, vec![moltis_tls::ServerSan::Ip(ip)]);
        } else {
            assert!(sans.is_empty());
        }
    }

    #[cfg(feature = "ngrok")]
    #[test]
    fn ngrok_loopback_guard_rejects_requests_without_proxy_headers() {
        let headers = axum::http::HeaderMap::new();
        assert!(!ngrok_loopback_has_proxy_headers(&headers));
    }

    #[cfg(feature = "ngrok")]
    #[test]
    fn ngrok_loopback_guard_allows_requests_with_proxy_headers() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.50".parse().unwrap());
        assert!(ngrok_loopback_has_proxy_headers(&headers));
    }

    #[test]
    fn ipv6_bind_addresses_parse_correctly() {
        // Regression test for GitHub issue #447 — binding to "::" crashed
        // because `format!("{bind}:{port}")` produced the unparseable ":::8080".

        // Demonstrate the old `format!("{bind}:{port}")` approach is broken for IPv6.
        assert!(":::8080".parse::<SocketAddr>().is_err());
        assert!("::1:8080".parse::<SocketAddr>().is_err());

        let cases: &[(&str, u16)] = &[
            ("::", 8080),
            ("::1", 8080),
            ("0.0.0.0", 9090),
            ("127.0.0.1", 3000),
            // Parses OK; actual bind requires a zone ID (e.g. fe80::1%eth0) on most OSes.
            ("fe80::1", 443),
        ];
        for &(bind, port) in cases {
            let ip: std::net::IpAddr = bind.parse().unwrap_or_else(|e| {
                panic!("failed to parse bind address '{bind}': {e}");
            });
            let addr = SocketAddr::new(ip, port);
            if bind.contains(':') {
                assert!(addr.is_ipv6(), "expected IPv6 SocketAddr for bind={bind}");
            } else {
                assert!(addr.is_ipv4(), "expected IPv4 SocketAddr for bind={bind}");
            }
        }
    }

    #[test]
    fn startup_bind_line_includes_bind_flag_and_address() {
        let addr: SocketAddr = "0.0.0.0:49494".parse().unwrap();
        assert_eq!(startup_bind_line(addr), "bind (--bind): 0.0.0.0:49494");
    }

    #[test]
    fn startup_passkey_origin_lines_emits_clickable_urls() {
        let lines = startup_passkey_origin_lines(&[
            "https://localhost:49494".to_string(),
            "https://m4max.local:49494".to_string(),
        ]);
        assert_eq!(lines, vec![
            "passkey origin: https://localhost:49494",
            "passkey origin: https://m4max.local:49494",
        ]);
    }

    #[test]
    fn startup_setup_code_lines_adds_spacers() {
        let lines = startup_setup_code_lines("493413");
        assert_eq!(lines, vec![
            "",
            "setup code: 493413",
            "enter this code to set your password or register a passkey",
            "",
        ]);
    }

    #[cfg(feature = "ngrok")]
    #[test]
    fn public_build_gateway_base_keeps_ngrok_controller_alive() {
        let state = GatewayState::new(
            auth::resolve_auth(None, None),
            moltis_gateway::services::GatewayServices::noop(),
        );
        let methods = Arc::new(MethodRegistry::new());
        #[cfg(feature = "push-notifications")]
        let (_router, app_state) = build_gateway_base(state, methods, None, None);
        #[cfg(not(feature = "push-notifications"))]
        let (_router, app_state) = build_gateway_base(state, methods, None);

        assert!(app_state.ngrok_controller_owner.is_some());
        assert!(app_state.ngrok_controller.upgrade().is_some());
    }

    #[cfg(feature = "ngrok")]
    #[test]
    fn attaching_owner_keeps_internal_ngrok_controller_alive_after_local_arc_drop() {
        let state = GatewayState::new(
            auth::resolve_auth(None, None),
            moltis_gateway::services::GatewayServices::noop(),
        );
        let methods = Arc::new(MethodRegistry::new());
        #[cfg(feature = "push-notifications")]
        let (_router, app_state, ngrok_controller) =
            build_gateway_base_internal(state, methods, None, None);
        #[cfg(not(feature = "push-notifications"))]
        let (_router, mut app_state, ngrok_controller) =
            build_gateway_base_internal(state, methods, None);

        assert!(app_state.ngrok_controller.upgrade().is_some());

        let weak = app_state.ngrok_controller.clone();
        drop(ngrok_controller);
        assert!(weak.upgrade().is_none());

        #[cfg(feature = "push-notifications")]
        let (_router, mut app_state, ngrok_controller) = build_gateway_base_internal(
            GatewayState::new(
                auth::resolve_auth(None, None),
                moltis_gateway::services::GatewayServices::noop(),
            ),
            Arc::new(MethodRegistry::new()),
            None,
            None,
        );
        #[cfg(not(feature = "push-notifications"))]
        let (_router, mut app_state, ngrok_controller) = build_gateway_base_internal(
            GatewayState::new(
                auth::resolve_auth(None, None),
                moltis_gateway::services::GatewayServices::noop(),
            ),
            Arc::new(MethodRegistry::new()),
            None,
        );

        attach_ngrok_controller_owner(&mut app_state, &ngrok_controller);
        let weak = app_state.ngrok_controller.clone();
        drop(ngrok_controller);
        assert!(weak.upgrade().is_some());
        assert!(app_state.ngrok_controller_owner.is_some());
    }
}
