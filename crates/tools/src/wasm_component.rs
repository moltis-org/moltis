#[cfg(feature = "wasm")]
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    thread,
    time::Duration,
};

#[cfg(feature = "wasm")]
use anyhow::Context;

#[cfg(feature = "wasm")]
use crate::ssrf::ssrf_check_blocking;

#[cfg(feature = "wasm")]
pub mod pure_tool {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "pure-tool",
    });
}

#[cfg(feature = "wasm")]
pub mod http_tool {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "http-tool",
    });
}

#[cfg(feature = "wasm")]
pub type PureToolValue = pure_tool::moltis::tool::types::ToolValue;
#[cfg(feature = "wasm")]
pub type PureToolResult = pure_tool::moltis::tool::types::ToolResult;
#[cfg(feature = "wasm")]
pub type PureToolError = pure_tool::moltis::tool::types::ToolError;
#[cfg(feature = "wasm")]
pub type HttpRequest = http_tool::moltis::tool::outgoing_handler::HttpRequest;
#[cfg(feature = "wasm")]
pub type HttpResponse = http_tool::moltis::tool::outgoing_handler::HttpResponse;
#[cfg(feature = "wasm")]
pub type HttpError = http_tool::moltis::tool::outgoing_handler::HttpError;
#[cfg(feature = "wasm")]
pub type HttpToolValue = http_tool::moltis::tool::types::ToolValue;
#[cfg(feature = "wasm")]
pub type HttpToolResult = http_tool::moltis::tool::types::ToolResult;
#[cfg(feature = "wasm")]
pub type HttpToolError = http_tool::moltis::tool::types::ToolError;

/// Host-injected headers keyed by lowercase domain.
///
/// For each matching domain the host silently adds (or overrides) headers on
/// outgoing requests. This keeps secrets like API keys out of guest parameters.
#[cfg(feature = "wasm")]
pub type SecretHeaders = HashMap<String, Vec<(String, String)>>;

#[cfg(feature = "wasm")]
#[derive(Clone)]
pub struct HttpHostImpl {
    client: reqwest::blocking::Client,
    ssrf_allowlist: Vec<ipnet::IpNet>,
    max_response_bytes: u64,
    domain_allowlist: Option<HashSet<String>>,
    secret_headers: SecretHeaders,
}

#[cfg(feature = "wasm")]
impl HttpHostImpl {
    pub fn new(
        timeout: Duration,
        max_response_bytes: usize,
        ssrf_allowlist: Vec<ipnet::IpNet>,
        domain_allowlist: Option<Vec<String>>,
        secret_headers: SecretHeaders,
    ) -> anyhow::Result<Self> {
        fn build_blocking_client(timeout: Duration) -> anyhow::Result<reqwest::blocking::Client> {
            reqwest::blocking::Client::builder()
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .context("failed to build blocking HTTP client for wasm host")
        }

        let max_response_bytes = u64::try_from(max_response_bytes)
            .context("max_response_bytes does not fit into u64")?;
        let domain_allowlist = domain_allowlist.map(|domains| {
            domains
                .into_iter()
                .map(|domain| domain.trim().to_ascii_lowercase())
                .filter(|domain| !domain.is_empty())
                .collect::<HashSet<_>>()
        });
        let client = if tokio::runtime::Handle::try_current().is_ok() {
            thread::Builder::new()
                .name("moltis-wasm-http-client-init".to_string())
                .spawn(move || build_blocking_client(timeout))
                .context("failed to spawn blocking HTTP client init thread")?
                .join()
                .map_err(|_| anyhow::anyhow!("blocking HTTP client init thread panicked"))??
        } else {
            build_blocking_client(timeout)?
        };
        Ok(Self {
            client,
            ssrf_allowlist,
            max_response_bytes,
            domain_allowlist,
            secret_headers,
        })
    }

    fn domain_allowed(&self, host: &str) -> bool {
        let Some(allowlist) = &self.domain_allowlist else {
            return true;
        };
        let host = host.to_ascii_lowercase();
        allowlist
            .iter()
            .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
    }

    fn map_request_error(error: reqwest::Error) -> HttpError {
        if error.is_timeout() {
            HttpError::Timeout(error.to_string())
        } else {
            HttpError::Network(error.to_string())
        }
    }

    fn map_io_error(error: std::io::Error) -> HttpError {
        HttpError::Network(error.to_string())
    }

    fn resolve_max_response_bytes(&self, requested: Option<u64>) -> u64 {
        match requested {
            Some(value) => value.min(self.max_response_bytes),
            None => self.max_response_bytes,
        }
    }

    pub fn handle_request(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| HttpError::InvalidUrl(format!("invalid HTTP method: {error}")))?;
        let parsed_url = url::Url::parse(&request.url)
            .map_err(|error| HttpError::InvalidUrl(error.to_string()))?;
        if !matches!(parsed_url.scheme(), "http" | "https") {
            return Err(HttpError::InvalidUrl(format!(
                "unsupported URL scheme: {}",
                parsed_url.scheme()
            )));
        }
        let host = parsed_url
            .host_str()
            .ok_or_else(|| HttpError::InvalidUrl("URL has no host".to_string()))?;
        if !self.domain_allowed(host) {
            return Err(HttpError::BlockedUrl(format!(
                "host `{host}` is not in domain allowlist"
            )));
        }
        ssrf_check_blocking(&parsed_url, &self.ssrf_allowlist)
            .map_err(|error| HttpError::BlockedUrl(error.to_string()))?;

        // Collect host-injected header names so guest cannot override them.
        let host_injected = self
            .secret_headers
            .get(&host.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default();
        let host_header_names: HashSet<String> = host_injected
            .iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect();

        let mut req = self.client.request(method, parsed_url.as_str());
        // Apply guest headers, filtering out any that collide with host-injected ones.
        for (header_name, header_value) in &request.headers {
            if host_header_names.contains(&header_name.to_ascii_lowercase()) {
                continue;
            }
            let parsed_name = reqwest::header::HeaderName::from_bytes(header_name.as_bytes())
                .map_err(|error| {
                    HttpError::Other(format!(
                        "invalid request header name `{header_name}`: {error}"
                    ))
                })?;
            let parsed_value =
                reqwest::header::HeaderValue::from_str(header_value).map_err(|error| {
                    HttpError::Other(format!(
                        "invalid request header value for `{header_name}`: {error}"
                    ))
                })?;
            req = req.header(parsed_name, parsed_value);
        }
        // Inject host-side secret headers (always win over guest headers).
        for (header_name, header_value) in &host_injected {
            let parsed_name = reqwest::header::HeaderName::from_bytes(header_name.as_bytes())
                .map_err(|error| {
                    HttpError::Other(format!("invalid host header name `{header_name}`: {error}"))
                })?;
            let parsed_value =
                reqwest::header::HeaderValue::from_str(header_value).map_err(|error| {
                    HttpError::Other(format!(
                        "invalid host header value for `{header_name}`: {error}"
                    ))
                })?;
            req = req.header(parsed_name, parsed_value);
        }
        if let Some(body) = request.body {
            req = req.body(body);
        }
        if let Some(timeout_ms) = request.timeout_ms {
            req = req.timeout(Duration::from_millis(u64::from(timeout_ms)));
        }

        let mut response = req.send().map_err(Self::map_request_error)?;
        let status = response.status();

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();

        let max_response_bytes = self.resolve_max_response_bytes(request.max_response_bytes);
        let mut body = Vec::new();
        response
            .by_ref()
            .take(max_response_bytes.saturating_add(1))
            .read_to_end(&mut body)
            .map_err(Self::map_io_error)?;
        if u64::try_from(body.len()).unwrap_or(u64::MAX) > max_response_bytes {
            return Err(HttpError::TooLarge(max_response_bytes));
        }

        Ok(HttpResponse {
            status: status.as_u16(),
            headers,
            body,
            content_type,
        })
    }
}

#[cfg(feature = "wasm")]
impl http_tool::moltis::tool::outgoing_handler::Host for HttpHostImpl {
    fn handle(&mut self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        self.handle_request(request)
    }
}

#[cfg(feature = "wasm")]
pub fn add_http_outgoing_handler_to_linker<T>(
    linker: &mut wasmtime::component::Linker<T>,
    host_getter: impl Fn(&mut T) -> &mut HttpHostImpl + Copy + Send + Sync + 'static,
) -> anyhow::Result<()>
where
    T: 'static,
{
    http_tool::moltis::tool::outgoing_handler::add_to_linker_get_host(linker, host_getter)
        .context("failed to add outgoing-handler to linker")?;
    Ok(())
}

#[cfg(feature = "wasm")]
#[must_use]
pub fn marshal_tool_result(value: PureToolValue) -> serde_json::Value {
    match value {
        PureToolValue::Text(text) => serde_json::Value::String(text),
        PureToolValue::Number(number) => serde_json::Number::from_f64(number)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        PureToolValue::Integer(integer) => serde_json::Value::Number(integer.into()),
        PureToolValue::Boolean(boolean) => serde_json::Value::Bool(boolean),
        PureToolValue::Json(json) => match serde_json::from_str::<serde_json::Value>(&json) {
            Ok(parsed) => parsed,
            Err(_) => serde_json::Value::String(json),
        },
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(all(test, feature = "wasm"))]
mod tests {
    use {
        super::{HttpError, HttpHostImpl, HttpRequest, PureToolValue, marshal_tool_result},
        std::{
            collections::HashMap,
            io::{Read, Write},
            net::TcpListener,
            thread::{self, JoinHandle},
            time::Duration,
        },
    };

    #[test]
    fn marshal_tool_result_text() {
        let value = marshal_tool_result(PureToolValue::Text("hello".to_string()));
        assert_eq!(value, serde_json::json!("hello"));
    }

    #[test]
    fn marshal_tool_result_number() {
        let value = marshal_tool_result(PureToolValue::Number(12.5));
        assert_eq!(value, serde_json::json!(12.5));
    }

    #[test]
    fn marshal_tool_result_integer() {
        let value = marshal_tool_result(PureToolValue::Integer(-42));
        assert_eq!(value, serde_json::json!(-42));
    }

    #[test]
    fn marshal_tool_result_boolean() {
        let value = marshal_tool_result(PureToolValue::Boolean(true));
        assert_eq!(value, serde_json::json!(true));
    }

    #[test]
    fn marshal_tool_result_json() {
        let value = marshal_tool_result(PureToolValue::Json("{\"k\":\"v\"}".to_string()));
        assert_eq!(value, serde_json::json!({"k": "v"}));
    }

    fn spawn_http_server(body: &'static [u8], delay: Option<Duration>) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request_buffer = [0_u8; 2048];
                let _ = stream.read(&mut request_buffer);
                if let Some(wait) = delay {
                    thread::sleep(wait);
                }
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        (format!("http://{addr}/"), handle)
    }

    fn request_for_url(url: String) -> HttpRequest {
        HttpRequest {
            method: "GET".to_string(),
            url,
            headers: Vec::new(),
            body: None,
            timeout_ms: None,
            max_response_bytes: None,
        }
    }

    #[test]
    fn http_host_blocks_private_address_without_allowlist() {
        let host = HttpHostImpl::new(
            Duration::from_secs(2),
            64 * 1024,
            Vec::new(),
            None,
            HashMap::new(),
        )
        .unwrap();
        let request = request_for_url("http://127.0.0.1:80/secret".to_string());
        let result = host.handle_request(request);
        assert!(matches!(result, Err(HttpError::BlockedUrl(_))));
    }

    #[test]
    fn http_host_enforces_max_response_bytes() {
        let (url, handle) = spawn_http_server(b"0123456789", None);
        let allowlist = vec!["127.0.0.1/32".parse().unwrap()];
        let host =
            HttpHostImpl::new(Duration::from_secs(2), 8, allowlist, None, HashMap::new()).unwrap();
        let request = request_for_url(url);
        let result = host.handle_request(request);
        assert!(matches!(result, Err(HttpError::TooLarge(8))));
        handle.join().unwrap();
    }

    #[test]
    fn http_host_maps_timeout_errors() {
        let (url, handle) = spawn_http_server(b"slow", Some(Duration::from_millis(200)));
        let allowlist = vec!["127.0.0.1/32".parse().unwrap()];
        let host = HttpHostImpl::new(
            Duration::from_secs(2),
            64 * 1024,
            allowlist,
            None,
            HashMap::new(),
        )
        .unwrap();
        let mut request = request_for_url(url);
        request.timeout_ms = Some(20);
        let result = host.handle_request(request);
        assert!(matches!(result, Err(HttpError::Timeout(_))));
        handle.join().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_host_new_inside_tokio_runtime() {
        let host = HttpHostImpl::new(
            Duration::from_secs(2),
            64 * 1024,
            Vec::new(),
            None,
            HashMap::new(),
        );
        assert!(host.is_ok());
    }

    /// Spawn a tiny HTTP server that echoes back all request headers as the
    /// response body (one `name: value` per line).
    fn spawn_echo_headers_server() -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0_u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request_text = String::from_utf8_lossy(&buf[..n]);
                // Collect all header lines (skip request line).
                let body: String = request_text
                    .lines()
                    .skip(1)
                    .take_while(|line| !line.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://{addr}/echo"), handle)
    }

    fn host_with_allowlist_and_secrets(
        domain_allowlist: Option<Vec<String>>,
        secret_headers: HashMap<String, Vec<(String, String)>>,
    ) -> HttpHostImpl {
        let allowlist = vec!["127.0.0.1/32".parse().unwrap()];
        HttpHostImpl::new(
            Duration::from_secs(2),
            64 * 1024,
            allowlist,
            domain_allowlist,
            secret_headers,
        )
        .unwrap()
    }

    #[test]
    fn http_host_injects_secret_headers() {
        let (url, handle) = spawn_echo_headers_server();
        let mut secrets = HashMap::new();
        secrets.insert("127.0.0.1".to_string(), vec![(
            "X-Subscription-Token".to_string(),
            "my-secret-key".to_string(),
        )]);
        let host = host_with_allowlist_and_secrets(None, secrets);
        let request = request_for_url(url);
        let response = host.handle_request(request).unwrap();
        let body = String::from_utf8(response.body).unwrap();
        assert!(
            body.contains("my-secret-key"),
            "expected secret header in response body, got: {body}"
        );
        handle.join().unwrap();
    }

    #[test]
    fn http_host_secret_headers_override_guest_headers() {
        let (url, handle) = spawn_echo_headers_server();
        let mut secrets = HashMap::new();
        secrets.insert("127.0.0.1".to_string(), vec![(
            "X-Subscription-Token".to_string(),
            "host-value".to_string(),
        )]);
        let host = host_with_allowlist_and_secrets(None, secrets);
        let request = HttpRequest {
            method: "GET".to_string(),
            url,
            headers: vec![(
                "X-Subscription-Token".to_string(),
                "guest-value".to_string(),
            )],
            body: None,
            timeout_ms: None,
            max_response_bytes: None,
        };
        let response = host.handle_request(request).unwrap();
        let body = String::from_utf8(response.body).unwrap();
        assert!(
            body.contains("host-value"),
            "host secret should win, got: {body}"
        );
        assert!(
            !body.contains("guest-value"),
            "guest value should be filtered out, got: {body}"
        );
        handle.join().unwrap();
    }

    #[test]
    fn http_host_domain_allowlist_blocks_disallowed_host() {
        let host = host_with_allowlist_and_secrets(
            Some(vec!["allowed.example.com".to_string()]),
            HashMap::new(),
        );
        let request = request_for_url("http://127.0.0.1:9999/blocked".to_string());
        let result = host.handle_request(request);
        assert!(
            matches!(result, Err(HttpError::BlockedUrl(_))),
            "expected BlockedUrl, got: {result:?}"
        );
    }

    #[test]
    fn http_host_rejects_non_http_scheme() {
        let host = host_with_allowlist_and_secrets(None, HashMap::new());
        let request = request_for_url("ftp://127.0.0.1/file".to_string());
        let result = host.handle_request(request);
        assert!(
            matches!(result, Err(HttpError::InvalidUrl(ref msg)) if msg.contains("unsupported URL scheme")),
            "expected InvalidUrl for ftp scheme, got: {result:?}"
        );
    }

    #[test]
    fn http_host_rejects_invalid_method() {
        let host = host_with_allowlist_and_secrets(None, HashMap::new());
        let request = HttpRequest {
            method: "NOT A METHOD".to_string(),
            url: "http://127.0.0.1/test".to_string(),
            headers: Vec::new(),
            body: None,
            timeout_ms: None,
            max_response_bytes: None,
        };
        let result = host.handle_request(request);
        assert!(
            matches!(result, Err(HttpError::InvalidUrl(ref msg)) if msg.contains("invalid HTTP method")),
            "expected InvalidUrl for bad method, got: {result:?}"
        );
    }
}
