//! HTTP CONNECT proxy server with domain filtering.
//!
//! Handles both `CONNECT host:port` (HTTPS) and plain HTTP forward requests.
//! Connections from non-private IPs are rejected for security.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
};

use {
    tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::{TcpListener, TcpStream},
    },
    tracing::{debug, info, instrument, warn},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, gauge, histogram};

use crate::{
    ApprovalSource, AuditSender, DomainDecision, FilterAction, FilterOutcome, NetworkAuditEntry,
    NetworkProtocol, Result, domain_approval::DomainApprovalManager,
};

// Re-export for convenience; canonical definition is in `types.rs`.
pub use crate::DEFAULT_PROXY_PORT;

/// HTTP CONNECT proxy server that filters outbound connections by domain.
///
/// The proxy handles:
/// - `CONNECT host:port` requests (used by HTTPS clients via `HTTPS_PROXY`)
/// - Plain HTTP requests forwarded via `HTTP_PROXY`
///
/// For each connection, the domain is checked against the `DomainApprovalManager`.
/// If the domain needs approval, the proxy holds the connection until a decision
/// is made (or timeout).
pub struct NetworkProxyServer {
    listener_addr: SocketAddr,
    filter: Arc<DomainApprovalManager>,
    audit_tx: Option<AuditSender>,
}

impl NetworkProxyServer {
    pub fn new(
        listener_addr: SocketAddr,
        filter: Arc<DomainApprovalManager>,
        audit_tx: Option<AuditSender>,
    ) -> Self {
        Self {
            listener_addr,
            filter,
            audit_tx,
        }
    }

    /// Start the proxy server. This runs until the `shutdown` future completes.
    pub async fn run(&self, shutdown: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        let listener = TcpListener::bind(self.listener_addr).await?;
        info!(addr = %self.listener_addr, "network proxy listening");

        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, peer)) => {
                            if !is_private_or_loopback(&peer.ip()) {
                                debug!(peer = %peer, "rejected proxy connection from non-private IP");
                                drop(stream);
                                continue;
                            }
                            let filter = Arc::clone(&self.filter);
                            let audit_tx = self.audit_tx.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_client(stream, peer, filter, audit_tx).await {
                                    debug!(peer = %peer, error = %e, "proxy client error");
                                }
                            });
                        },
                        Err(e) => {
                            warn!(error = %e, "proxy accept error");
                        },
                    }
                },
                _ = shutdown_signal(&shutdown) => {
                    info!("network proxy shutting down");
                    break;
                },
            }
        }
        Ok(())
    }

    /// The address the proxy is listening on.
    pub fn addr(&self) -> SocketAddr {
        self.listener_addr
    }
}

/// Returns `true` if the IP is loopback or belongs to a private/link-local
/// range.  Used to reject connections from public IPs when the proxy binds
/// to `0.0.0.0` so container VMs on bridge/vmnet networks can reach it.
fn is_private_or_loopback(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()            // 127.0.0.0/8
                || v4.is_private()       // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local()    // 169.254/16
                || is_cgnat(*v4) // 100.64/10 (Tailscale, CGNAT)
        },
        IpAddr::V6(v6) => {
            v6.is_loopback()             // ::1
                || is_ula(*v6) // fc00::/7
        },
    }
}

/// CGNAT / shared address space (100.64.0.0/10), also used by Tailscale.
fn is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0xC0) == 64
}

/// IPv6 Unique Local Address (fc00::/7).
fn is_ula(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xFE00) == 0xFC00
}

async fn shutdown_signal(rx: &tokio::sync::watch::Receiver<bool>) {
    let mut rx = rx.clone();
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            return;
        }
    }
}

/// Handle a single client connection.
///
/// Reads the first line to determine if it's a CONNECT request or a plain HTTP request.
#[instrument(skip(stream, filter, audit_tx), fields(peer = %peer))]
async fn handle_client(
    stream: TcpStream,
    peer: SocketAddr,
    filter: Arc<DomainApprovalManager>,
    audit_tx: Option<AuditSender>,
) -> Result<()> {
    #[cfg(feature = "metrics")]
    {
        counter!("proxy_connections_total").increment(1);
        gauge!("proxy_connections_active").increment(1.0);
    }

    let result = handle_client_inner(stream, peer, filter, audit_tx).await;

    #[cfg(feature = "metrics")]
    gauge!("proxy_connections_active").decrement(1.0);

    result
}

async fn handle_client_inner(
    stream: TcpStream,
    peer: SocketAddr,
    filter: Arc<DomainApprovalManager>,
    audit_tx: Option<AuditSender>,
) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await?;
    let request_line = request_line.trim_end();

    if request_line.is_empty() {
        return Err(crate::Error::message("empty request"));
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(crate::Error::message(format!(
            "malformed request line: {request_line}"
        )));
    }

    let method = parts[0];
    let target = parts[1];

    if method.eq_ignore_ascii_case("CONNECT") {
        handle_connect(reader, peer, target, filter, audit_tx).await
    } else {
        handle_http_forward(reader, peer, method, target, filter, audit_tx).await
    }
}

/// Send an audit entry on a best-effort basis (non-blocking, drop if channel full).
fn emit_audit(tx: &Option<AuditSender>, entry: NetworkAuditEntry) {
    if let Some(ref sender) = *tx {
        let _ = sender.try_send(entry);
    }
}

/// Handle an HTTP CONNECT tunnel request.
#[instrument(skip(reader, filter, audit_tx), fields(peer = %peer, target = %target))]
async fn handle_connect(
    mut reader: BufReader<TcpStream>,
    peer: SocketAddr,
    target: &str,
    filter: Arc<DomainApprovalManager>,
    audit_tx: Option<AuditSender>,
) -> Result<()> {
    let start = std::time::Instant::now();

    // Parse host:port from CONNECT target.
    let (domain, port) = parse_host_port(target);

    // Consume remaining request headers.
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        if line.trim().is_empty() {
            break;
        }
    }

    // Use peer address as session identifier for now.
    let session = peer.to_string();
    let (action, approval_source) = filter.check_domain_with_source(&session, &domain).await;

    match action {
        FilterAction::Allow => {
            #[cfg(feature = "metrics")]
            counter!("proxy_requests_total", "method" => "CONNECT", "result" => "allowed")
                .increment(1);
        },
        FilterAction::Deny => {
            #[cfg(feature = "metrics")]
            counter!("proxy_requests_total", "method" => "CONNECT", "result" => "denied")
                .increment(1);
            let resp = "HTTP/1.1 403 Forbidden\r\n\r\n";
            reader.get_mut().write_all(resp.as_bytes()).await?;
            emit_audit(&audit_tx, NetworkAuditEntry {
                timestamp: time::OffsetDateTime::now_utc(),
                session: session.clone(),
                domain: domain.clone(),
                port,
                protocol: NetworkProtocol::HttpConnect,
                action: FilterOutcome::Denied,
                method: None,
                url: None,
                status: None,
                bytes_sent: 0,
                bytes_received: 0,
                duration_ms: start.elapsed().as_millis() as u64,
                error: None,
                approval_source: None,
            });
            return Ok(());
        },
        FilterAction::NeedsApproval => {
            let (id, rx) = filter.create_request(&session, &domain).await;
            debug!(id = %id, domain = %domain, "waiting for domain approval");
            let decision = filter.wait_for_decision(rx).await;
            match decision {
                DomainDecision::Approved => {
                    #[cfg(feature = "metrics")]
                    counter!("proxy_requests_total", "method" => "CONNECT", "result" => "approved")
                        .increment(1);
                    // approval_source is overridden: user prompt approved it.
                },
                DomainDecision::Denied => {
                    #[cfg(feature = "metrics")]
                    counter!("proxy_requests_total", "method" => "CONNECT", "result" => "denied")
                        .increment(1);
                    let resp = "HTTP/1.1 403 Forbidden\r\n\r\n";
                    reader.get_mut().write_all(resp.as_bytes()).await?;
                    emit_audit(&audit_tx, NetworkAuditEntry {
                        timestamp: time::OffsetDateTime::now_utc(),
                        session: session.clone(),
                        domain: domain.clone(),
                        port,
                        protocol: NetworkProtocol::HttpConnect,
                        action: FilterOutcome::Denied,
                        method: None,
                        url: None,
                        status: None,
                        bytes_sent: 0,
                        bytes_received: 0,
                        duration_ms: start.elapsed().as_millis() as u64,
                        error: None,
                        approval_source: None,
                    });
                    return Ok(());
                },
                DomainDecision::Timeout => {
                    #[cfg(feature = "metrics")]
                    counter!("proxy_requests_total", "method" => "CONNECT", "result" => "denied")
                        .increment(1);
                    let resp = "HTTP/1.1 403 Forbidden\r\n\r\n";
                    reader.get_mut().write_all(resp.as_bytes()).await?;
                    emit_audit(&audit_tx, NetworkAuditEntry {
                        timestamp: time::OffsetDateTime::now_utc(),
                        session: session.clone(),
                        domain: domain.clone(),
                        port,
                        protocol: NetworkProtocol::HttpConnect,
                        action: FilterOutcome::Timeout,
                        method: None,
                        url: None,
                        status: None,
                        bytes_sent: 0,
                        bytes_received: 0,
                        duration_ms: start.elapsed().as_millis() as u64,
                        error: None,
                        approval_source: None,
                    });
                    return Ok(());
                },
            }
        },
    }

    // Determine the effective approval source for allowed/approved connections.
    let effective_source = if action == FilterAction::NeedsApproval {
        Some(ApprovalSource::UserPrompt)
    } else {
        approval_source
    };

    // Connect to upstream.
    let upstream_addr = format!("{domain}:{port}");
    let upstream = match TcpStream::connect(&upstream_addr).await {
        Ok(s) => s,
        Err(e) => {
            #[cfg(feature = "metrics")]
            counter!("proxy_upstream_errors_total", "error" => "connect_failed").increment(1);
            let resp = format!("HTTP/1.1 502 Bad Gateway\r\n\r\n{e}");
            reader.get_mut().write_all(resp.as_bytes()).await?;
            let action = if action == FilterAction::NeedsApproval {
                FilterOutcome::ApprovedByUser
            } else {
                FilterOutcome::Allowed
            };
            emit_audit(&audit_tx, NetworkAuditEntry {
                timestamp: time::OffsetDateTime::now_utc(),
                session: session.clone(),
                domain: domain.clone(),
                port,
                protocol: NetworkProtocol::HttpConnect,
                action,
                method: None,
                url: None,
                status: None,
                bytes_sent: 0,
                bytes_received: 0,
                duration_ms: start.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
                approval_source: effective_source.clone(),
            });
            return Ok(());
        },
    };

    // Send 200 Connection Established.
    let resp = "HTTP/1.1 200 Connection Established\r\n\r\n";
    reader.get_mut().write_all(resp.as_bytes()).await?;

    // Bidirectional copy.
    let mut client_stream = reader.into_inner();
    let (mut client_read, mut client_write) = client_stream.split();
    let (mut upstream_read, mut upstream_write) = upstream.into_split();

    let c2u = tokio::io::copy(&mut client_read, &mut upstream_write);
    let u2c = tokio::io::copy(&mut upstream_read, &mut client_write);

    let (c2u_result, u2c_result) = tokio::join!(c2u, u2c);

    let (bytes_sent, c2u_err) = match c2u_result {
        Ok(n) => (n, None),
        Err(e) => (0, Some(e.to_string())),
    };
    let (bytes_received, u2c_err) = match u2c_result {
        Ok(n) => (n, None),
        Err(e) => (0, Some(e.to_string())),
    };
    let error = c2u_err.or(u2c_err);

    #[cfg(feature = "metrics")]
    {
        counter!("proxy_bytes_transferred_total", "direction" => "client_to_upstream")
            .increment(bytes_sent);
        counter!("proxy_bytes_transferred_total", "direction" => "upstream_to_client")
            .increment(bytes_received);
        histogram!("proxy_tunnel_duration_seconds").record(start.elapsed().as_secs_f64());
    }

    let audit_action = if action == FilterAction::NeedsApproval {
        FilterOutcome::ApprovedByUser
    } else {
        FilterOutcome::Allowed
    };
    emit_audit(&audit_tx, NetworkAuditEntry {
        timestamp: time::OffsetDateTime::now_utc(),
        session,
        domain,
        port,
        protocol: NetworkProtocol::HttpConnect,
        action: audit_action,
        method: None,
        url: None,
        status: None,
        bytes_sent,
        bytes_received,
        duration_ms: start.elapsed().as_millis() as u64,
        error,
        approval_source: effective_source,
    });

    Ok(())
}

/// Handle a plain HTTP forward request (non-CONNECT).
///
/// Used when `HTTP_PROXY` is set and the client sends a full URL request.
#[instrument(skip(reader, filter, audit_tx), fields(peer = %peer, method = %method, target = %target))]
async fn handle_http_forward(
    mut reader: BufReader<TcpStream>,
    peer: SocketAddr,
    method: &str,
    target: &str,
    filter: Arc<DomainApprovalManager>,
    audit_tx: Option<AuditSender>,
) -> Result<()> {
    let start = std::time::Instant::now();

    // Extract host from the URL.
    let domain = extract_host_from_url(target)?;
    let port = extract_port_from_url(target);

    let session = peer.to_string();
    let (action, approval_source) = filter.check_domain_with_source(&session, &domain).await;

    match action {
        FilterAction::Allow => {
            #[cfg(feature = "metrics")]
            counter!("proxy_requests_total", "method" => "HTTP", "result" => "allowed")
                .increment(1);
        },
        FilterAction::Deny => {
            #[cfg(feature = "metrics")]
            counter!("proxy_requests_total", "method" => "HTTP", "result" => "denied").increment(1);
            let resp = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n";
            reader.get_mut().write_all(resp.as_bytes()).await?;
            emit_audit(&audit_tx, NetworkAuditEntry {
                timestamp: time::OffsetDateTime::now_utc(),
                session: session.clone(),
                domain: domain.clone(),
                port,
                protocol: NetworkProtocol::HttpForward,
                action: FilterOutcome::Denied,
                method: Some(method.to_string()),
                url: Some(target.to_string()),
                status: None,
                bytes_sent: 0,
                bytes_received: 0,
                duration_ms: start.elapsed().as_millis() as u64,
                error: None,
                approval_source: None,
            });
            return Ok(());
        },
        FilterAction::NeedsApproval => {
            let (id, rx) = filter.create_request(&session, &domain).await;
            debug!(id = %id, domain = %domain, "waiting for domain approval (HTTP)");
            let decision = filter.wait_for_decision(rx).await;
            match decision {
                DomainDecision::Approved => {
                    #[cfg(feature = "metrics")]
                    counter!("proxy_requests_total", "method" => "HTTP", "result" => "approved")
                        .increment(1);
                },
                DomainDecision::Denied => {
                    #[cfg(feature = "metrics")]
                    counter!("proxy_requests_total", "method" => "HTTP", "result" => "denied")
                        .increment(1);
                    let resp = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n";
                    reader.get_mut().write_all(resp.as_bytes()).await?;
                    emit_audit(&audit_tx, NetworkAuditEntry {
                        timestamp: time::OffsetDateTime::now_utc(),
                        session: session.clone(),
                        domain: domain.clone(),
                        port,
                        protocol: NetworkProtocol::HttpForward,
                        action: FilterOutcome::Denied,
                        method: Some(method.to_string()),
                        url: Some(target.to_string()),
                        status: None,
                        bytes_sent: 0,
                        bytes_received: 0,
                        duration_ms: start.elapsed().as_millis() as u64,
                        error: None,
                        approval_source: None,
                    });
                    return Ok(());
                },
                DomainDecision::Timeout => {
                    #[cfg(feature = "metrics")]
                    counter!("proxy_requests_total", "method" => "HTTP", "result" => "denied")
                        .increment(1);
                    let resp = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n";
                    reader.get_mut().write_all(resp.as_bytes()).await?;
                    emit_audit(&audit_tx, NetworkAuditEntry {
                        timestamp: time::OffsetDateTime::now_utc(),
                        session: session.clone(),
                        domain: domain.clone(),
                        port,
                        protocol: NetworkProtocol::HttpForward,
                        action: FilterOutcome::Timeout,
                        method: Some(method.to_string()),
                        url: Some(target.to_string()),
                        status: None,
                        bytes_sent: 0,
                        bytes_received: 0,
                        duration_ms: start.elapsed().as_millis() as u64,
                        error: None,
                        approval_source: None,
                    });
                    return Ok(());
                },
            }
        },
    }

    // Determine the effective approval source for allowed/approved connections.
    let effective_source = if action == FilterAction::NeedsApproval {
        Some(ApprovalSource::UserPrompt)
    } else {
        approval_source
    };

    // Read remaining headers.
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        if line.trim().is_empty() {
            break;
        }
        headers.push_str(&line);
    }

    // Connect to upstream and forward the request.
    let upstream_addr = format!("{domain}:{port}");
    let mut upstream = match TcpStream::connect(&upstream_addr).await {
        Ok(s) => s,
        Err(e) => {
            #[cfg(feature = "metrics")]
            counter!("proxy_upstream_errors_total", "error" => "connect_failed").increment(1);
            let resp = format!("HTTP/1.1 502 Bad Gateway\r\n\r\n{e}");
            reader.get_mut().write_all(resp.as_bytes()).await?;
            let audit_action = if action == FilterAction::NeedsApproval {
                FilterOutcome::ApprovedByUser
            } else {
                FilterOutcome::Allowed
            };
            emit_audit(&audit_tx, NetworkAuditEntry {
                timestamp: time::OffsetDateTime::now_utc(),
                session: session.clone(),
                domain: domain.clone(),
                port,
                protocol: NetworkProtocol::HttpForward,
                action: audit_action,
                method: Some(method.to_string()),
                url: Some(target.to_string()),
                status: None,
                bytes_sent: 0,
                bytes_received: 0,
                duration_ms: start.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
                approval_source: effective_source.clone(),
            });
            return Ok(());
        },
    };

    // Convert absolute URL to relative path for upstream.
    let path = url_to_path(target);
    let request_line = format!("{method} {path} HTTP/1.1\r\n");
    upstream.write_all(request_line.as_bytes()).await?;
    upstream.write_all(headers.as_bytes()).await?;
    upstream.write_all(b"\r\n").await?;

    // Bidirectional copy for the rest.
    let mut client_stream = reader.into_inner();
    let (mut client_read, mut client_write) = client_stream.split();
    let (mut upstream_read, mut upstream_write) = upstream.into_split();

    let c2u = tokio::io::copy(&mut client_read, &mut upstream_write);
    let u2c = tokio::io::copy(&mut upstream_read, &mut client_write);

    let (c2u_result, u2c_result) = tokio::join!(c2u, u2c);

    let (bytes_sent, c2u_err) = match c2u_result {
        Ok(n) => (n, None),
        Err(e) => (0, Some(e.to_string())),
    };
    let (bytes_received, u2c_err) = match u2c_result {
        Ok(n) => (n, None),
        Err(e) => (0, Some(e.to_string())),
    };
    let error = c2u_err.or(u2c_err);

    #[cfg(feature = "metrics")]
    {
        counter!("proxy_bytes_transferred_total", "direction" => "client_to_upstream")
            .increment(bytes_sent);
        counter!("proxy_bytes_transferred_total", "direction" => "upstream_to_client")
            .increment(bytes_received);
        histogram!("proxy_request_duration_seconds").record(start.elapsed().as_secs_f64());
    }

    let audit_action = if action == FilterAction::NeedsApproval {
        FilterOutcome::ApprovedByUser
    } else {
        FilterOutcome::Allowed
    };
    emit_audit(&audit_tx, NetworkAuditEntry {
        timestamp: time::OffsetDateTime::now_utc(),
        session,
        domain,
        port,
        protocol: NetworkProtocol::HttpForward,
        action: audit_action,
        method: Some(method.to_string()),
        url: Some(target.to_string()),
        status: None,
        bytes_sent,
        bytes_received,
        duration_ms: start.elapsed().as_millis() as u64,
        error,
        approval_source: effective_source,
    });

    Ok(())
}

/// Parse `host:port` from a CONNECT target. Defaults port to 443 if not specified.
fn parse_host_port(target: &str) -> (String, u16) {
    if let Some((host, port_str)) = target.rsplit_once(':') {
        let port: u16 = port_str.parse().unwrap_or(443);
        (host.to_string(), port)
    } else {
        (target.to_string(), 443)
    }
}

/// Extract the hostname from an absolute HTTP URL.
fn extract_host_from_url(url: &str) -> Result<String> {
    // Strip scheme.
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);

    // Take up to the first '/' or end.
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);

    // Strip port if present.
    if let Some((host, _)) = host_port.rsplit_once(':') {
        Ok(host.to_string())
    } else {
        Ok(host_port.to_string())
    }
}

/// Extract the port from an absolute HTTP URL. Defaults to 80 for http, 443 for https.
fn extract_port_from_url(url: &str) -> u16 {
    let is_https = url.starts_with("https://");
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);

    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    if let Some((_, port_str)) = host_port.rsplit_once(':') {
        port_str.parse().unwrap_or(if is_https {
            443
        } else {
            80
        })
    } else if is_https {
        443
    } else {
        80
    }
}

/// Convert an absolute URL to a relative path.
fn url_to_path(url: &str) -> String {
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);

    if let Some(slash_pos) = after_scheme.find('/') {
        after_scheme[slash_pos..].to_string()
    } else {
        "/".to_string()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_port() {
        let (host, port) = parse_host_port("github.com:443");
        assert_eq!(host, "github.com");
        assert_eq!(port, 443);

        let (host, port) = parse_host_port("example.com");
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);

        let (host, port) = parse_host_port("api.example.com:8080");
        assert_eq!(host, "api.example.com");
        assert_eq!(port, 8080);
    }

    #[test]
    fn test_extract_host_from_url() {
        assert_eq!(
            extract_host_from_url("http://example.com/path").unwrap(),
            "example.com"
        );
        assert_eq!(
            extract_host_from_url("https://api.github.com:443/v1").unwrap(),
            "api.github.com"
        );
        assert_eq!(
            extract_host_from_url("http://localhost:8080/").unwrap(),
            "localhost"
        );
    }

    #[test]
    fn test_extract_port_from_url() {
        assert_eq!(extract_port_from_url("http://example.com/path"), 80);
        assert_eq!(extract_port_from_url("https://example.com/path"), 443);
        assert_eq!(extract_port_from_url("http://example.com:8080/path"), 8080);
    }

    #[test]
    fn test_url_to_path() {
        assert_eq!(
            url_to_path("http://example.com/path/to/resource"),
            "/path/to/resource"
        );
        assert_eq!(url_to_path("http://example.com"), "/");
        assert_eq!(url_to_path("https://api.github.com/v1/repos"), "/v1/repos");
    }

    #[test]
    fn test_is_private_or_loopback() {
        // Loopback
        assert!(is_private_or_loopback(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_private_or_loopback(&IpAddr::V6(Ipv6Addr::LOCALHOST)));

        // Private ranges (RFC 1918)
        assert!(is_private_or_loopback(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_or_loopback(&"172.17.0.1".parse().unwrap())); // Docker bridge
        assert!(is_private_or_loopback(&"192.168.64.1".parse().unwrap())); // macOS vmnet

        // CGNAT / Tailscale
        assert!(is_private_or_loopback(&"100.64.0.1".parse().unwrap()));
        assert!(is_private_or_loopback(&"100.127.255.254".parse().unwrap()));

        // Link-local
        assert!(is_private_or_loopback(&"169.254.1.1".parse().unwrap()));

        // IPv6 ULA
        assert!(is_private_or_loopback(&"fd00::1".parse().unwrap()));

        // Public IPs must be rejected
        assert!(!is_private_or_loopback(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_or_loopback(&"1.1.1.1".parse().unwrap()));
        assert!(!is_private_or_loopback(&"2001:db8::1".parse().unwrap()));
    }
}
