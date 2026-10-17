//! WebSocket RPC client for communicating with the Moltis gateway.
//!
//! A thin wrapper over [`moltis_client::Connection`], which implements the v4
//! handshake and request/response framing.

use std::time::Duration;

use {
    moltis_client::{ClientIdentity, ConnectOptions, Connection, Endpoint, SecretString, TlsTrust},
    serde_json::Value,
};

use crate::error::CtlError;

pub struct CtlClient {
    connection: Connection,
}

impl CtlClient {
    /// Connect to the Moltis gateway and perform the v4 handshake.
    pub async fn connect(gateway_url: &str, api_key: &str) -> Result<Self, CtlError> {
        let endpoint = Endpoint::parse(gateway_url, TlsTrust::System)?;
        let mut options = ConnectOptions::new(endpoint, ClientIdentity {
            platform: "cli".into(),
            ..ClientIdentity::new("moltis-ctl", env!("CARGO_PKG_VERSION"))
        })
        .with_api_key((!api_key.is_empty()).then(|| SecretString::new(api_key.into())));
        options.request_timeout = Duration::from_secs(120);
        let connection = Connection::connect(&options).await?;
        Ok(Self { connection })
    }

    /// Send an RPC request and wait for the response.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, CtlError> {
        Ok(self.connection.call(method, params).await?)
    }
}
