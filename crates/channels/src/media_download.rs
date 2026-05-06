use async_trait::async_trait;

use crate::Result;

/// Opaque source for downloading inbound media bytes from a channel.
///
/// The source shape is intentionally higher-level than "just a URL" so channel
/// implementations can extend this enum as new platforms need authenticated or
/// non-HTTP fetch paths.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InboundMediaSource {
    /// Download media bytes from a remote URL.
    RemoteUrl { url: String },
}

/// Fetches inbound media payloads for a channel-specific media source.
#[async_trait]
pub trait InboundMediaDownloader: Send + Sync {
    /// Download the source payload, enforcing a caller-provided size cap.
    async fn download_media(
        &self,
        source: &InboundMediaSource,
        max_bytes: usize,
    ) -> Result<Vec<u8>>;
}
