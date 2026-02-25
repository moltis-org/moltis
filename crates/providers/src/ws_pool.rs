//! Lightweight WebSocket connection pool for the OpenAI Responses API.
//!
//! Reuses idle connections across consecutive `stream_with_tools_websocket`
//! calls.  Connections are bucketed by `(ws_url, key_hash)` so different
//! providers/keys never share a socket.  Stale entries are lazily evicted
//! on checkout — no background reaper task.

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::LazyLock,
};

use {
    tokio::{net::TcpStream, sync::Mutex},
    tokio_tungstenite::{MaybeTlsStream, WebSocketStream},
};

use tracing::debug;

// ── Pool constants (compile-time, not user-facing) ───────────────────

const MAX_IDLE_PER_KEY: usize = 4;
const MAX_IDLE_TOTAL: usize = 8;
/// Must sit below OpenAI's 60 s server-side idle timeout.
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(55);
/// Rotate connections periodically regardless of activity.
const MAX_LIFETIME: std::time::Duration = std::time::Duration::from_secs(300);

// ── Types ────────────────────────────────────────────────────────────

/// Stream type returned by `tokio_tungstenite::connect_async`.
pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Key used to bucket pooled connections.
#[derive(Clone, Hash, Eq, PartialEq)]
pub struct PoolKey {
    ws_url: String,
    key_hash: u64,
}

impl PoolKey {
    /// Build a pool key from a WebSocket URL and an API key.
    ///
    /// The API key is hashed immediately — we never store the secret.
    pub fn new(ws_url: &str, api_key: &secrecy::Secret<String>) -> Self {
        use secrecy::ExposeSecret;
        let mut hasher = DefaultHasher::new();
        api_key.expose_secret().hash(&mut hasher);
        Self {
            ws_url: ws_url.to_string(),
            key_hash: hasher.finish(),
        }
    }
}

struct IdleConnection {
    stream: WsStream,
    returned_at: std::time::Instant,
    created_at: std::time::Instant,
}

/// A bounded pool of idle WebSocket connections.
pub struct WsPool {
    connections: Mutex<HashMap<PoolKey, Vec<IdleConnection>>>,
}

// ── Singleton ────────────────────────────────────────────────────────

/// Global shared pool (mirrors `shared_http_client()` pattern).
pub fn shared_ws_pool() -> &'static WsPool {
    static POOL: LazyLock<WsPool> = LazyLock::new(WsPool::new);
    &POOL
}

// ── Implementation ───────────────────────────────────────────────────

impl WsPool {
    fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
        }
    }

    /// Try to reclaim an idle connection for the given key.
    ///
    /// Returns `None` when the pool has nothing valid.  Stale entries
    /// (idle-timeout or max-lifetime) are dropped automatically.
    pub async fn checkout(&self, key: &PoolKey) -> Option<(WsStream, std::time::Instant)> {
        let mut map = self.connections.lock().await;
        let bucket = map.get_mut(key)?;
        let now = std::time::Instant::now();

        while let Some(entry) = bucket.pop() {
            if now.duration_since(entry.returned_at) > IDLE_TIMEOUT {
                debug!("ws_pool: dropping idle-timeout connection");
                continue;
            }
            if now.duration_since(entry.created_at) > MAX_LIFETIME {
                debug!("ws_pool: dropping max-lifetime connection");
                continue;
            }
            if bucket.is_empty() {
                map.remove(key);
            }
            debug!("ws_pool: reusing pooled connection");
            return Some((entry.stream, entry.created_at));
        }

        // Bucket drained without finding a valid connection.
        map.remove(key);
        None
    }

    /// Return a still-healthy connection for future reuse.
    ///
    /// The connection is silently dropped if it exceeds `MAX_LIFETIME`
    /// or the pool is already at capacity.
    pub async fn return_conn(
        &self,
        key: PoolKey,
        stream: WsStream,
        created_at: std::time::Instant,
    ) {
        let now = std::time::Instant::now();
        if now.duration_since(created_at) > MAX_LIFETIME {
            debug!("ws_pool: not returning max-lifetime connection");
            return;
        }

        let mut map = self.connections.lock().await;

        // Per-key cap — drop oldest if at limit.
        let bucket = map.entry(key.clone()).or_default();
        if bucket.len() >= MAX_IDLE_PER_KEY {
            debug!("ws_pool: per-key cap reached, dropping oldest");
            bucket.remove(0);
        }
        bucket.push(IdleConnection {
            stream,
            returned_at: now,
            created_at,
        });

        // Global cap — evict oldest entry across all buckets.
        let total: usize = map.values().map(Vec::len).sum();
        if total > MAX_IDLE_TOTAL {
            debug!("ws_pool: global cap reached, evicting oldest");
            Self::evict_oldest(&mut map);
        }
    }

    /// Remove the single oldest (by `returned_at`) entry across all buckets.
    fn evict_oldest(map: &mut HashMap<PoolKey, Vec<IdleConnection>>) {
        let mut oldest_key: Option<PoolKey> = None;
        let mut oldest_idx: usize = 0;
        let mut oldest_time: Option<std::time::Instant> = None;

        for (key, bucket) in map.iter() {
            for (idx, entry) in bucket.iter().enumerate() {
                if oldest_time.is_none_or(|t| entry.returned_at < t) {
                    oldest_key = Some(key.clone());
                    oldest_idx = idx;
                    oldest_time = Some(entry.returned_at);
                }
            }
        }

        if let Some(key) = oldest_key
            && let Some(bucket) = map.get_mut(&key)
        {
            bucket.remove(oldest_idx);
            if bucket.is_empty() {
                map.remove(&key);
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use {
        super::*,
        futures::{SinkExt, StreamExt},
        secrecy::Secret,
        tokio_tungstenite::tungstenite::Message,
    };

    /// Server-side WS stream (plain TCP, no TLS wrapper).
    type ServerWsStream = WebSocketStream<TcpStream>;

    /// Spin up a local WS server and return a real `WsStream` client
    /// plus the server half for verification.
    async fn make_ws_pair() -> (WsStream, ServerWsStream) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server_handle = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept");
            tokio_tungstenite::accept_async(tcp)
                .await
                .expect("ws accept")
        });

        let url = format!("ws://127.0.0.1:{}", addr.port());
        let (client, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("connect");
        let server = server_handle.await.expect("join server");
        (client, server)
    }

    fn test_key(url: &str, secret: &str) -> PoolKey {
        PoolKey::new(url, &Secret::new(secret.to_string()))
    }

    #[tokio::test]
    async fn checkout_empty_returns_none() {
        let pool = WsPool::new();
        let key = test_key("wss://api.openai.com/v1/responses", "sk-test");
        assert!(pool.checkout(&key).await.is_none());
    }

    #[tokio::test]
    async fn return_then_checkout() {
        let pool = WsPool::new();
        let key = test_key("wss://api.openai.com/v1/responses", "sk-test");
        let (client, mut server) = make_ws_pair().await;

        let created = std::time::Instant::now();
        pool.return_conn(key.clone(), client, created).await;

        let (mut stream, checkout_created) =
            pool.checkout(&key).await.expect("should get connection");
        assert_eq!(
            checkout_created, created,
            "created_at should be preserved through pool"
        );

        // Verify the connection still works by sending a message.
        stream
            .send(Message::Text("hello".into()))
            .await
            .expect("send");
        let msg = server.next().await.expect("recv").expect("frame");
        assert_eq!(msg, Message::Text("hello".into()));

        // Pool should now be empty.
        assert!(pool.checkout(&key).await.is_none());
    }

    #[tokio::test]
    async fn idle_timeout_eviction() {
        let pool = WsPool::new();
        let key = test_key("wss://api.openai.com/v1/responses", "sk-test");
        let (client, _server) = make_ws_pair().await;

        // Simulate a connection returned long ago by using a very old returned_at.
        let created = std::time::Instant::now();
        {
            let mut map = pool.connections.lock().await;
            map.entry(key.clone()).or_default().push(IdleConnection {
                stream: client,
                returned_at: created - (IDLE_TIMEOUT + std::time::Duration::from_secs(1)),
                created_at: created,
            });
        }

        assert!(
            pool.checkout(&key).await.is_none(),
            "stale connection should be evicted"
        );
    }

    #[tokio::test]
    async fn max_lifetime_eviction() {
        let pool = WsPool::new();
        let key = test_key("wss://api.openai.com/v1/responses", "sk-test");
        let (client, _server) = make_ws_pair().await;

        let old_created =
            std::time::Instant::now() - (MAX_LIFETIME + std::time::Duration::from_secs(1));
        {
            let mut map = pool.connections.lock().await;
            map.entry(key.clone()).or_default().push(IdleConnection {
                stream: client,
                returned_at: std::time::Instant::now(),
                created_at: old_created,
            });
        }

        assert!(
            pool.checkout(&key).await.is_none(),
            "max-lifetime connection should be evicted"
        );
    }

    #[tokio::test]
    async fn max_idle_per_key() {
        let pool = WsPool::new();
        let key = test_key("wss://api.openai.com/v1/responses", "sk-test");

        let mut _servers = Vec::new();
        for _ in 0..(MAX_IDLE_PER_KEY + 2) {
            let (client, server) = make_ws_pair().await;
            _servers.push(server);
            pool.return_conn(key.clone(), client, std::time::Instant::now())
                .await;
        }

        let map = pool.connections.lock().await;
        let count = map.get(&key).map(Vec::len).unwrap_or(0);
        assert!(
            count <= MAX_IDLE_PER_KEY,
            "bucket should not exceed MAX_IDLE_PER_KEY ({MAX_IDLE_PER_KEY}), got {count}"
        );
    }

    #[tokio::test]
    async fn max_idle_total() {
        let pool = WsPool::new();

        let mut _servers = Vec::new();
        for i in 0..(MAX_IDLE_TOTAL + 4) {
            let key = test_key(&format!("wss://api.openai.com/v1/responses/{i}"), "sk-test");
            let (client, server) = make_ws_pair().await;
            _servers.push(server);
            pool.return_conn(key, client, std::time::Instant::now())
                .await;
        }

        let map = pool.connections.lock().await;
        let total: usize = map.values().map(Vec::len).sum();
        assert!(
            total <= MAX_IDLE_TOTAL,
            "global pool should not exceed MAX_IDLE_TOTAL ({MAX_IDLE_TOTAL}), got {total}"
        );
    }

    #[tokio::test]
    async fn different_keys_isolated() {
        let pool = WsPool::new();
        let key_a = test_key("wss://api.openai.com/v1/responses", "sk-aaa");
        let key_b = test_key("wss://api.openai.com/v1/responses", "sk-bbb");

        let (client_a, _server_a) = make_ws_pair().await;
        pool.return_conn(key_a.clone(), client_a, std::time::Instant::now())
            .await;

        assert!(
            pool.checkout(&key_b).await.is_none(),
            "key_b should not receive key_a's connection"
        );
        assert!(
            pool.checkout(&key_a).await.is_some(),
            "key_a should still have its connection"
        );
    }

    #[tokio::test]
    async fn max_lifetime_rejected_on_return() {
        let pool = WsPool::new();
        let key = test_key("wss://api.openai.com/v1/responses", "sk-test");
        let (client, _server) = make_ws_pair().await;

        let old_created =
            std::time::Instant::now() - (MAX_LIFETIME + std::time::Duration::from_secs(1));
        pool.return_conn(key.clone(), client, old_created).await;

        let map = pool.connections.lock().await;
        assert!(
            map.get(&key).is_none(),
            "expired connection should not be stored"
        );
    }
}
