use std::{collections::HashMap, sync::Arc};

use {
    anyhow::{Result, bail},
    axum::{Router, extract::Query, response::Html, routing::get},
    tokio::sync::oneshot,
};

/// Starts a local HTTP server to receive the OAuth callback, then shuts down.
pub struct CallbackServer;

impl CallbackServer {
    /// Listen on `127.0.0.1:{port}` for a GET `/auth/callback` with `code` and `state` params.
    /// Validates state matches `expected_state`, returns the authorization code.
    /// Times out after 60 seconds.
    pub async fn wait_for_code(port: u16, expected_state: String) -> Result<String> {
        let (tx, rx) = oneshot::channel::<Result<String>>();
        let tx = Arc::new(std::sync::Mutex::new(Some(tx)));

        let app = Router::new().route(
            "/auth/callback",
            get(move |Query(params): Query<HashMap<String, String>>| {
                let tx = tx.lock().unwrap_or_else(|e| e.into_inner()).take();
                async move {
                    let result = (|| {
                        let state = params.get("state").ok_or("missing state")?;
                        if *state != expected_state {
                            return Err("state mismatch");
                        }
                        let code = params.get("code").ok_or("missing code")?;
                        Ok(code.clone())
                    })();

                    match result {
                        Ok(code) => {
                            if let Some(tx) = tx {
                                let _ = tx.send(Ok(code));
                            }
                            Html("<h1>Authentication successful!</h1><p>You can close this window.</p>".to_string())
                        }
                        Err(e) => {
                            if let Some(tx) = tx {
                                let _ = tx.send(Err(anyhow::anyhow!("{e}")));
                            }
                            Html(format!("<h1>Authentication failed</h1><p>{e}</p>"))
                        }
                    }
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
        let server = axum::serve(listener, app);

        tokio::select! {
            result = rx => {
                result?
            }
            _ = server.into_future() => {
                bail!("server exited unexpectedly")
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {
                bail!("OAuth callback timed out after 60 seconds")
            }
        }
    }
}
