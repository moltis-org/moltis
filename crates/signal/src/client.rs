//! HTTP JSON-RPC client for signal-cli daemon mode.

use {
    reqwest::{Client, Response},
    serde::{Deserialize, Serialize, de::DeserializeOwned},
    serde_json::{Value, json},
    std::time::Duration,
    uuid::Uuid,
};

#[derive(Clone, Debug)]
pub struct SignalClient {
    /// Short-lived RPC requests (connect 10s, overall 30s).
    http: Client,
    /// Long-lived SSE stream (connect 10s, no overall timeout).
    sse: Client,
}

#[derive(Debug, Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: Option<i64>,
    message: Option<String>,
}

#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    method: &'a str,
    params: Value,
    id: String,
}

impl Default for SignalClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalClient {
    pub fn new() -> Self {
        Self {
            http: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            sse: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn check(&self, base_url: &str) -> moltis_channels::Result<Value> {
        let url = format!("{}/api/v1/check", base_url.trim_end_matches('/'));
        let res = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| moltis_channels::Error::external("signal-cli check", e))?;
        let status = res.status();
        if !status.is_success() {
            return Err(moltis_channels::Error::unavailable(format!(
                "signal-cli check failed with HTTP {status}"
            )));
        }
        res.json::<Value>()
            .await
            .map_err(|e| moltis_channels::Error::external("signal-cli check json", e))
    }

    pub async fn stream_events(
        &self,
        base_url: &str,
        account: Option<&str>,
    ) -> moltis_channels::Result<Response> {
        let mut url = url::Url::parse(&format!("{}/api/v1/events", base_url.trim_end_matches('/')))
            .map_err(|e| {
                moltis_channels::Error::invalid_input(format!("invalid Signal daemon URL: {e}"))
            })?;
        if let Some(account) = account {
            url.query_pairs_mut().append_pair("account", account);
        }
        self.sse
            .get(url)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(|e| moltis_channels::Error::external("signal-cli events", e))
    }

    pub async fn rpc_value(
        &self,
        base_url: &str,
        method: &str,
        params: Value,
    ) -> moltis_channels::Result<Value> {
        self.rpc(base_url, method, params).await
    }

    pub async fn rpc<T>(
        &self,
        base_url: &str,
        method: &str,
        params: Value,
    ) -> moltis_channels::Result<T>
    where
        T: DeserializeOwned,
    {
        let url = format!("{}/api/v1/rpc", base_url.trim_end_matches('/'));
        let body = RpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id: Uuid::new_v4().to_string(),
        };

        let res = self
            .http
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|e| moltis_channels::Error::external("signal-cli rpc", e))?;
        let status = res.status();
        if !status.is_success() {
            return Err(moltis_channels::Error::unavailable(format!(
                "signal-cli RPC {method} failed with HTTP {status}"
            )));
        }

        let text = res
            .text()
            .await
            .map_err(|e| moltis_channels::Error::external("signal-cli rpc body", e))?;
        if text.trim().is_empty() {
            return serde_json::from_value(json!({}))
                .map_err(|e| moltis_channels::Error::external("signal-cli empty rpc response", e));
        }

        let parsed: RpcResponse<T> = serde_json::from_str(&text).map_err(|e| {
            moltis_channels::Error::external("signal-cli malformed rpc response", e)
        })?;
        if let Some(error) = parsed.error {
            let code = error
                .code
                .map_or_else(|| "unknown".to_string(), |v| v.to_string());
            let message = error
                .message
                .unwrap_or_else(|| "Signal RPC error".to_string());
            return Err(moltis_channels::Error::unavailable(format!(
                "signal-cli RPC {method} error {code}: {message}"
            )));
        }
        parsed.result.ok_or_else(|| {
            moltis_channels::Error::unavailable(format!(
                "signal-cli RPC {method} returned no result"
            ))
        })
    }
}
