//! CDP screencast: stream browser page frames via `Page.startScreencast`.
//!
//! Each active screencast gets a background task that listens for
//! `Page.screencastFrame` CDP events, acknowledges them, and forwards
//! the base64 JPEG frame data to a broadcast channel that the gateway
//! can relay to WebSocket clients.

use std::{collections::HashMap, sync::Arc};

use {
    chromiumoxide::{
        Page,
        cdp::browser_protocol::page::{
            EventFrameNavigated, EventScreencastFrame, ScreencastFrameAckParams,
            StartScreencastFormat, StartScreencastParams, StopScreencastParams,
        },
    },
    futures::StreamExt,
    serde::Serialize,
    tokio::sync::{RwLock, broadcast},
    tracing::{debug, info, warn},
};

/// A single screencast frame delivered to subscribers.
#[derive(Debug, Clone, Serialize)]
pub struct ScreencastFrame {
    /// Browser session ID this frame belongs to.
    pub session_id: String,
    /// Base64-encoded JPEG image data.
    pub data: String,
    /// Frame metadata.
    pub metadata: FrameMetadata,
    /// Monotonically increasing frame sequence number.
    pub sequence: u64,
    /// Current page URL (included when it changes, None otherwise).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Metadata for a screencast frame.
#[derive(Debug, Clone, Serialize)]
pub struct FrameMetadata {
    pub offset_top: f64,
    pub page_scale_factor: f64,
    pub device_width: f64,
    pub device_height: f64,
    pub scroll_offset_x: f64,
    pub scroll_offset_y: f64,
    pub timestamp: f64,
}

/// Channel capacity for screencast frame broadcasts.
const FRAME_CHANNEL_CAPACITY: usize = 4;

/// Handle to a running screencast. Cloneable for multi-subscriber use.
pub struct ScreencastHandle {
    tx: broadcast::Sender<ScreencastFrame>,
    session_id: String,
}

impl ScreencastHandle {
    /// Subscribe a new receiver to this screencast's frame stream.
    pub fn subscribe(&self) -> broadcast::Receiver<ScreencastFrame> {
        self.tx.subscribe()
    }

    /// The browser session ID this screencast belongs to.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

/// Registry of active screencasts, keyed by browser session ID.
#[derive(Default)]
pub struct ScreencastRegistry {
    active: RwLock<HashMap<String, Arc<ActiveScreencast>>>,
}

struct ActiveScreencast {
    tx: broadcast::Sender<ScreencastFrame>,
    abort: tokio::task::AbortHandle,
}

impl Drop for ActiveScreencast {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

impl ScreencastRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a screencast for the given page and session ID.
    ///
    /// Returns a [`broadcast::Receiver`] for frame events. If a screencast is
    /// already active for this session, returns a new subscriber to the
    /// existing stream.
    pub async fn start(
        &self,
        session_id: &str,
        page: &Page,
        quality: u8,
        max_width: u32,
        max_height: u32,
    ) -> Result<broadcast::Receiver<ScreencastFrame>, crate::error::Error> {
        // If already active, return a new subscriber.
        {
            let active = self.active.read().await;
            if let Some(handle) = active.get(session_id) {
                debug!(session_id, "screencast already active, adding subscriber");
                return Ok(handle.tx.subscribe());
            }
        }

        // Start CDP screencast via the builder pattern.
        let params = StartScreencastParams {
            format: Some(StartScreencastFormat::Jpeg),
            quality: Some(i64::from(quality.min(100))),
            max_width: Some(i64::from(max_width)),
            max_height: Some(i64::from(max_height)),
            every_nth_frame: Some(1),
        };

        page.execute(params)
            .await
            .map_err(|e| crate::error::Error::Cdp(format!("failed to start screencast: {e}")))?;

        let (tx, rx) = broadcast::channel(FRAME_CHANNEL_CAPACITY);

        // Spawn background task to relay CDP screencast frame events.
        let tx_clone = tx.clone();
        let sid = session_id.to_string();
        let page_clone = page.clone();

        let task = tokio::spawn(async move {
            relay_screencast_frames(page_clone, tx_clone, sid).await;
        });

        let inner = Arc::new(ActiveScreencast {
            tx: tx.clone(),
            abort: task.abort_handle(),
        });

        self.active
            .write()
            .await
            .insert(session_id.to_string(), inner);

        debug!(session_id, "screencast started");
        Ok(rx)
    }

    /// Stop the screencast for the given session.
    pub async fn stop(
        &self,
        session_id: &str,
        page: Option<&Page>,
    ) -> Result<(), crate::error::Error> {
        let removed = self.active.write().await.remove(session_id);
        if removed.is_some() {
            // Try to send stop command to CDP (best effort).
            if let Some(page) = page {
                let _ = page.execute(StopScreencastParams::default()).await;
            }
            debug!(session_id, "screencast stopped");
        }
        Ok(())
    }

    /// Subscribe to an existing screencast for the given session.
    pub async fn subscribe(
        &self,
        session_id: &str,
    ) -> Option<broadcast::Receiver<ScreencastFrame>> {
        let active = self.active.read().await;
        active.get(session_id).map(|h| h.tx.subscribe())
    }

    /// Check if a screencast is active for the given session.
    pub async fn is_active(&self, session_id: &str) -> bool {
        self.active.read().await.contains_key(session_id)
    }

    /// List all active screencast session IDs.
    pub async fn active_sessions(&self) -> Vec<String> {
        self.active.read().await.keys().cloned().collect()
    }

    /// Stop all screencasts. Called during shutdown.
    pub async fn stop_all(&self) {
        let mut active = self.active.write().await;
        active.clear(); // Arc<Inner> drop aborts tasks
    }
}

/// Background task: listen for CDP screencast frame events and forward them.
async fn relay_screencast_frames(
    page: Page,
    tx: broadcast::Sender<ScreencastFrame>,
    session_id: String,
) {
    let mut frame_listener = match page.event_listener::<EventScreencastFrame>().await {
        Ok(l) => l,
        Err(e) => {
            warn!(session_id = %session_id, error = %e, "failed to subscribe to screencast frames");
            return;
        },
    };

    // Listen for navigation events to track URL changes without polling
    let mut nav_listener = page.event_listener::<EventFrameNavigated>().await.ok();

    let mut sequence: u64 = 0;
    let mut current_url: Option<String> = None;
    debug!(session_id = %session_id, "screencast frame listener ready, waiting for CDP frames");

    loop {
        // Process navigation events first (non-blocking drain)
        if let Some(ref mut nav) = nav_listener {
            while let Ok(Some(event)) =
                tokio::time::timeout(std::time::Duration::from_millis(0), nav.next()).await
            {
                if event.frame.parent_id.is_none() {
                    // Top-level frame navigation
                    current_url = Some(event.frame.url.clone());
                }
            }
        }

        // Wait for the next screencast frame
        let Some(event) = frame_listener.next().await else {
            break;
        };

        sequence += 1;
        if sequence == 1 {
            info!(session_id = %session_id, "first screencast frame received from CDP");
            // Get initial URL
            if current_url.is_none() {
                current_url = page.url().await.ok().flatten();
            }
        }

        // Acknowledge the frame so CDP sends the next one.
        let ack = ScreencastFrameAckParams::new(event.session_id);
        if let Err(e) = page.execute(ack).await {
            warn!(session_id = %session_id, error = %e, "failed to ack screencast frame");
        }

        let meta = &event.metadata;
        let frame = ScreencastFrame {
            session_id: session_id.clone(),
            data: event.data.clone().into(),
            metadata: FrameMetadata {
                offset_top: meta.offset_top,
                page_scale_factor: meta.page_scale_factor,
                device_width: meta.device_width,
                device_height: meta.device_height,
                scroll_offset_x: meta.scroll_offset_x,
                scroll_offset_y: meta.scroll_offset_y,
                timestamp: meta.timestamp.as_ref().map(|t| *t.inner()).unwrap_or(0.0),
            },
            sequence,
            url: current_url.take(),
        };

        let _ = tx.send(frame);
    }

    debug!(session_id = %session_id, "screencast frame relay ended");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_metadata_serializes() {
        let frame = ScreencastFrame {
            session_id: "test".into(),
            data: "AAAA".into(),
            metadata: FrameMetadata {
                offset_top: 0.0,
                page_scale_factor: 1.0,
                device_width: 1280.0,
                device_height: 800.0,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                timestamp: 123.456,
            },
            sequence: 1,
            url: None,
        };
        let json = serde_json::to_string(&frame);
        assert!(json.is_ok());
    }

    #[test]
    fn registry_is_default() {
        let _reg = ScreencastRegistry::new();
    }

    #[test]
    fn screencast_frame_url_serialized_only_when_some() {
        let with_url = ScreencastFrame {
            session_id: "s1".into(),
            data: "AAAA".into(),
            metadata: FrameMetadata {
                offset_top: 0.0,
                page_scale_factor: 1.0,
                device_width: 1280.0,
                device_height: 800.0,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                timestamp: 0.0,
            },
            sequence: 1,
            url: Some("https://example.com".into()),
        };
        let json = serde_json::to_string(&with_url).unwrap_or_else(|e| {
            panic!("serialize with url failed: {e}");
        });
        assert!(
            json.contains("\"url\""),
            "JSON should contain 'url' when Some, got: {json}"
        );

        let without_url = ScreencastFrame {
            session_id: "s2".into(),
            data: "BBBB".into(),
            metadata: FrameMetadata {
                offset_top: 0.0,
                page_scale_factor: 1.0,
                device_width: 1280.0,
                device_height: 800.0,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                timestamp: 0.0,
            },
            sequence: 2,
            url: None,
        };
        let json = serde_json::to_string(&without_url).unwrap_or_else(|e| {
            panic!("serialize without url failed: {e}");
        });
        assert!(
            !json.contains("\"url\""),
            "JSON should NOT contain 'url' when None, got: {json}"
        );
    }
}
