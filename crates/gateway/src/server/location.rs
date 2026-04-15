use std::sync::Arc;

use crate::state::GatewayState;

/// Gateway implementation of [`moltis_tools::location::LocationRequester`].
///
/// Uses the `PendingInvoke` + oneshot pattern to request the user's browser
/// geolocation and waits for `location.result` RPC to resolve it.
pub(crate) struct GatewayLocationRequester {
    pub(crate) state: Arc<GatewayState>,
}

#[async_trait::async_trait]
impl moltis_tools::location::LocationRequester for GatewayLocationRequester {
    async fn request_location(
        &self,
        conn_id: &str,
        precision: moltis_tools::location::LocationPrecision,
    ) -> moltis_tools::Result<moltis_tools::location::LocationResult> {
        use moltis_tools::location::{LocationError, LocationResult};

        let request_id = uuid::Uuid::new_v4().to_string();

        // Send a location.request event to the browser client, including
        // the requested precision so JS can adjust geolocation options.
        let event = moltis_protocol::EventFrame::new(
            "location.request",
            serde_json::json!({ "requestId": request_id, "precision": precision }),
            self.state.next_seq(),
        );
        let event_json = serde_json::to_string(&event)?;

        {
            let inner = self.state.inner.read().await;
            let clients = &inner.clients;
            let client = clients.get(conn_id).ok_or_else(|| {
                moltis_tools::Error::message(format!("no client connection for conn_id {conn_id}"))
            })?;
            if !client.send(&event_json) {
                return Err(moltis_tools::Error::message(format!(
                    "failed to send location request to client {conn_id}"
                )));
            }
        }

        // Set up a oneshot for the result with timeout.
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut inner_w = self.state.inner.write().await;
            let invokes = &mut inner_w.pending_invokes;
            invokes.insert(request_id.clone(), crate::state::PendingInvoke {
                request_id: request_id.clone(),
                sender: tx,
                created_at: std::time::Instant::now(),
            });
        }

        // Wait up to 30 seconds for the user to grant/deny permission.
        let result = match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                // Sender dropped — clean up.
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&request_id);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
            Err(_) => {
                // Timeout — clean up.
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&request_id);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
        };

        // Parse the result from the browser.
        if let Some(loc) = result.get("location") {
            let lat = loc.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let lon = loc.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let accuracy = loc.get("accuracy").and_then(|v| v.as_f64()).unwrap_or(0.0);
            Ok(LocationResult {
                location: Some(moltis_tools::location::BrowserLocation {
                    latitude: lat,
                    longitude: lon,
                    accuracy,
                }),
                error: None,
            })
        } else if let Some(err) = result.get("error") {
            let code = err.get("code").and_then(|v| v.as_u64()).unwrap_or(0);
            let error = match code {
                1 => LocationError::PermissionDenied,
                2 => LocationError::PositionUnavailable,
                3 => LocationError::Timeout,
                _ => LocationError::NotSupported,
            };
            Ok(LocationResult {
                location: None,
                error: Some(error),
            })
        } else {
            Ok(LocationResult {
                location: None,
                error: Some(LocationError::PositionUnavailable),
            })
        }
    }

    fn cached_location(&self) -> Option<moltis_config::GeoLocation> {
        self.state.inner.try_read().ok()?.cached_location.clone()
    }

    async fn request_channel_location(
        &self,
        session_key: &str,
    ) -> moltis_tools::Result<moltis_tools::location::LocationResult> {
        use moltis_tools::location::{LocationError, LocationResult};

        // Look up channel binding from session metadata.
        let session_meta = self
            .state
            .services
            .session_metadata
            .as_ref()
            .ok_or_else(|| moltis_tools::Error::message("session metadata not available"))?;
        let entry = session_meta.get(session_key).await.ok_or_else(|| {
            moltis_tools::Error::message(format!("no session metadata for key {session_key}"))
        })?;
        let binding_json = entry.channel_binding.ok_or_else(|| {
            moltis_tools::Error::message(format!("no channel binding for session {session_key}"))
        })?;
        let reply_target: moltis_channels::ChannelReplyTarget =
            serde_json::from_str(&binding_json)?;

        // Send a message asking the user to share their location.
        let outbound = self
            .state
            .services
            .channel_outbound_arc()
            .ok_or_else(|| moltis_tools::Error::message("no channel outbound available"))?;
        outbound
            .send_text(
                &reply_target.account_id,
                &reply_target.outbound_to(),
                "Please share your location in this chat, or paste a geo: link / map pin.",
                None,
            )
            .await
            .map_err(|e| moltis_tools::Error::external("send location request", e))?;

        // Create a pending invoke keyed by session.
        let pending_key = format!("channel_location:{session_key}");
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut inner = self.state.inner.write().await;
            inner
                .pending_invokes
                .insert(pending_key.clone(), crate::state::PendingInvoke {
                    request_id: pending_key.clone(),
                    sender: tx,
                    created_at: std::time::Instant::now(),
                });
        }

        // Wait up to 60 seconds — user needs to navigate Telegram's UI.
        let result = match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&pending_key);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
            Err(_) => {
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&pending_key);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
        };

        // Parse the result (same format as update_location sends).
        if let Some(loc) = result.get("location") {
            let lat = loc.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let lon = loc.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let accuracy = loc.get("accuracy").and_then(|v| v.as_f64()).unwrap_or(0.0);
            Ok(LocationResult {
                location: Some(moltis_tools::location::BrowserLocation {
                    latitude: lat,
                    longitude: lon,
                    accuracy,
                }),
                error: None,
            })
        } else {
            Ok(LocationResult {
                location: None,
                error: Some(LocationError::PositionUnavailable),
            })
        }
    }
}
