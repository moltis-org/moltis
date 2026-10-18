//! One authenticated WebSocket RPC connection to a daemon.
//!
//! [`Connection::connect`] performs the v4 `connect` handshake, then runs a
//! reader task that routes `res` frames to waiting callers by request id and
//! fans `event` frames out on a broadcast channel, and a writer task that owns
//! the socket sink and sends keepalive pings. The handle is cheap to clone.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

#[cfg(feature = "tracing")]
use tracing::{debug, warn};
use {
    futures::{SinkExt, StreamExt},
    moltis_protocol::{
        ClientInfo, ConnectAuth, ConnectParamsV4, ErrorShape, EventFrame, Extensions, HelloOk,
        PROTOCOL_VERSION, ProtocolRange, RequestFrame, ResponseFrame, error_codes, roles,
    },
    secrecy::{ExposeSecret, SecretString},
    serde::{Serialize, de::DeserializeOwned},
    serde_json::Value,
    tokio::sync::{broadcast, mpsc, oneshot, watch},
    tokio_tungstenite::{
        Connector, connect_async_tls_with_config,
        tungstenite::{
            self, Message,
            client::IntoClientRequest,
            http::{HeaderValue, StatusCode, header::AUTHORIZATION},
        },
    },
};

#[cfg(feature = "metrics")]
use moltis_metrics::{client as metric, counter, histogram};

use crate::{
    endpoint::Endpoint,
    error::{Error, Result},
    tls::client_config,
};

/// Events buffered per subscriber before the slowest one starts lagging.
const EVENT_BUFFER: usize = 1024;
/// Interval of client keepalive pings.
const PING_INTERVAL: Duration = Duration::from_secs(20);
/// A connection that received nothing for this long is considered dead.
const IDLE_TIMEOUT: Duration = Duration::from_secs(75);

/// What the client tells the daemon about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIdentity {
    pub id: String,
    pub display_name: Option<String>,
    pub version: String,
    pub platform: String,
}

impl ClientIdentity {
    #[must_use]
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: None,
            version: version.into(),
            platform: std::env::consts::OS.into(),
        }
    }
}

/// Everything needed to open a connection.
#[derive(Clone)]
pub struct ConnectOptions {
    pub endpoint: Endpoint,
    /// `mk_...` API key; `None` relies on the daemon allowing this client
    /// without credentials (loopback before setup, or auth disabled).
    pub api_key: Option<SecretString>,
    pub client: ClientIdentity,
    pub request_timeout: Duration,
    pub handshake_timeout: Duration,
}

impl ConnectOptions {
    #[must_use]
    pub fn new(endpoint: Endpoint, client: ClientIdentity) -> Self {
        Self {
            endpoint,
            api_key: None,
            client,
            request_timeout: Duration::from_secs(60),
            handshake_timeout: Duration::from_millis(moltis_protocol::HANDSHAKE_TIMEOUT_MS),
        }
    }

    #[must_use]
    pub fn with_api_key(mut self, api_key: Option<SecretString>) -> Self {
        self.api_key = api_key;
        self
    }
}

impl std::fmt::Debug for ConnectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectOptions")
            .field("endpoint", &self.endpoint)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

type Pending = Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>;

struct Inner {
    outgoing: mpsc::UnboundedSender<Message>,
    pending: Arc<Pending>,
    next_id: AtomicU64,
    events: broadcast::Sender<EventFrame>,
    closed: watch::Receiver<Option<String>>,
    hello: HelloOk,
    request_timeout: Duration,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Last handle gone: close the socket so both tasks stop.
        let _ = self.outgoing.send(Message::Close(None));
    }
}

/// A live connection. Clones share the socket.
#[derive(Clone)]
pub struct Connection {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("conn_id", &self.inner.hello.server.conn_id)
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Open the socket and complete the `connect` handshake.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, fields(endpoint = %options.endpoint)))]
    pub async fn connect(options: &ConnectOptions) -> Result<Self> {
        #[cfg(feature = "metrics")]
        counter!(metric::CONNECT_ATTEMPTS_TOTAL).increment(1);
        let result = Self::connect_inner(options).await;
        #[cfg(feature = "metrics")]
        if result.is_err() {
            counter!(metric::CONNECT_ERRORS_TOTAL).increment(1);
        }
        result
    }

    async fn connect_inner(options: &ConnectOptions) -> Result<Self> {
        let url = options.endpoint.ws_url();
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| Error::InvalidUrl(e.to_string()))?;
        if let Some(key) = &options.api_key {
            let value = HeaderValue::from_str(&format!("Bearer {}", key.expose_secret()))
                .map_err(|_| Error::Unauthorized("API key contains invalid characters".into()))?;
            request.headers_mut().insert(AUTHORIZATION, value);
        }
        let connector = if options.endpoint.is_tls() {
            Connector::Rustls(client_config(&options.endpoint.tls)?)
        } else {
            Connector::Plain
        };
        let open = connect_async_tls_with_config(request, None, false, Some(connector));
        let (mut ws, _) = tokio::time::timeout(options.handshake_timeout, open)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|e| map_ws_error(&options.endpoint, e))?;

        let connect_id = uuid::Uuid::new_v4().to_string();
        let params = ConnectParamsV4 {
            protocol: ProtocolRange {
                min: PROTOCOL_VERSION,
                max: PROTOCOL_VERSION,
            },
            client: ClientInfo {
                id: options.client.id.clone(),
                display_name: options.client.display_name.clone(),
                version: options.client.version.clone(),
                platform: options.client.platform.clone(),
                device_family: None,
                model_identifier: None,
                mode: roles::OPERATOR.into(),
                instance_id: None,
            },
            role: Some(roles::OPERATOR.into()),
            scopes: None,
            auth: options.api_key.as_ref().map(|key| ConnectAuth {
                token: None,
                password: None,
                api_key: Some(key.expose_secret().clone()),
                device_token: None,
                public_key: None,
            }),
            locale: None,
            timezone: None,
            extensions: Extensions::new(),
        };
        let frame = RequestFrame {
            r#type: "req".into(),
            id: connect_id.clone(),
            method: "connect".into(),
            params: Some(serde_json::to_value(params)?),
            channel: None,
        };
        ws.send(Message::Text(serde_json::to_string(&frame)?.into()))
            .await
            .map_err(|e| Error::Handshake(format!("send connect: {e}")))?;

        let hello =
            tokio::time::timeout(options.handshake_timeout, read_hello(&mut ws, &connect_id))
                .await
                .map_err(|_| Error::Timeout)??;
        #[cfg(feature = "tracing")]
        debug!(conn_id = %hello.server.conn_id, server = %hello.server.version, "connected");

        let (sink, stream) = ws.split();
        let (outgoing, outgoing_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let (closed_tx, closed) = watch::channel(None);
        let pending: Arc<Pending> = Arc::default();

        tokio::spawn(write_loop(sink, outgoing_rx));
        tokio::spawn(read_loop(
            stream,
            Arc::clone(&pending),
            events.clone(),
            outgoing.clone(),
            closed_tx,
        ));

        Ok(Self {
            inner: Arc::new(Inner {
                outgoing,
                pending,
                next_id: AtomicU64::new(1),
                events,
                closed,
                hello,
                request_timeout: options.request_timeout,
            }),
        })
    }

    /// The daemon's handshake reply.
    #[must_use]
    pub fn hello(&self) -> &HelloOk {
        &self.inner.hello
    }

    /// A new receiver of server events (only events subscribed to with
    /// `subscribe` are delivered).
    #[must_use]
    pub fn events(&self) -> broadcast::Receiver<EventFrame> {
        self.inner.events.subscribe()
    }

    /// Whether the socket has closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.closed.borrow().is_some()
    }

    /// Resolves with the reason once the socket closes.
    pub async fn closed(&self) -> String {
        let mut closed = self.inner.closed.clone();
        loop {
            if let Some(reason) = closed.borrow_and_update().clone() {
                return reason;
            }
            if closed.changed().await.is_err() {
                return "connection dropped".into();
            }
        }
    }

    /// Close the socket.
    pub fn close(&self) {
        let _ = self.inner.outgoing.send(Message::Close(None));
    }

    /// Send a request and wait for its response payload.
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip(self, params), fields(conn_id = %self.inner.hello.server.conn_id))
    )]
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        #[cfg(feature = "metrics")]
        let started = std::time::Instant::now();
        #[cfg(feature = "metrics")]
        counter!(metric::RPC_CALLS_TOTAL, "method" => method.to_string()).increment(1);
        let result = self.call_inner(method, params).await;
        #[cfg(feature = "metrics")]
        {
            histogram!(metric::RPC_DURATION_SECONDS, "method" => method.to_string())
                .record(started.elapsed().as_secs_f64());
            if result.is_err() {
                counter!(metric::RPC_ERRORS_TOTAL, "method" => method.to_string()).increment(1);
            }
        }
        result
    }

    async fn call_inner(&self, method: &str, params: Value) -> Result<Value> {
        if self.is_closed() {
            return Err(Error::Disconnected);
        }
        let id = format!("c{}", self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let frame = RequestFrame {
            r#type: "req".into(),
            id: id.clone(),
            method: method.into(),
            params: (!params.is_null()).then_some(params),
            channel: None,
        };
        let (tx, rx) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), tx);
        let sent = self
            .inner
            .outgoing
            .send(Message::Text(serde_json::to_string(&frame)?.into()));
        if sent.is_err() {
            self.forget(&id);
            return Err(Error::Disconnected);
        }
        match tokio::time::timeout(self.inner.request_timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(Error::Disconnected),
            Err(_) => {
                self.forget(&id);
                Err(Error::Timeout)
            },
        }
    }

    fn forget(&self, id: &str) {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// [`Self::call`] with typed params and response.
    pub async fn call_typed<P, R>(&self, method: &str, params: &P) -> Result<R>
    where
        P: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let payload = self.call(method, serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(payload)?)
    }
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn map_ws_error(endpoint: &Endpoint, error: tungstenite::Error) -> Error {
    match error {
        tungstenite::Error::Http(response)
            if matches!(
                response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ) =>
        {
            Error::Unauthorized(format!("{endpoint} answered {}", response.status()))
        },
        tungstenite::Error::Http(response) => Error::Http {
            status: response.status().as_u16(),
            body: String::from_utf8_lossy(response.body().as_deref().unwrap_or_default())
                .into_owned(),
        },
        tungstenite::Error::Tls(e) => Error::Tls(e.to_string()),
        tungstenite::Error::Io(e) if is_tls_failure(&e) => Error::Tls(e.to_string()),
        tungstenite::Error::Io(e) => Error::Connect(format!("{endpoint}: {e}")),
        other => Error::Connect(format!("{endpoint}: {other}")),
    }
}

/// rustls reports handshake failures (untrusted or changed certificates)
/// as I/O errors wrapping a `rustls::Error`.
fn is_tls_failure(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<rustls::Error>())
}

/// Read until the response to `connect` arrives.
async fn read_hello(ws: &mut WsStream, connect_id: &str) -> Result<HelloOk> {
    while let Some(message) = ws.next().await {
        let text = match message.map_err(|e| Error::Handshake(e.to_string()))? {
            Message::Text(text) => text,
            Message::Close(frame) => {
                let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                return Err(Error::Handshake(format!(
                    "closed during handshake: {reason}"
                )));
            },
            _ => continue,
        };
        let Ok(response) = serde_json::from_str::<ResponseFrame>(&text) else {
            continue;
        };
        if response.r#type != "res" || response.id != connect_id {
            continue;
        }
        if !response.ok {
            let error = response
                .error
                .unwrap_or_else(|| ErrorShape::new(error_codes::INTERNAL, "connect rejected"));
            return Err(
                if matches!(
                    error.code.as_str(),
                    error_codes::UNAUTHORIZED | error_codes::FORBIDDEN | error_codes::NOT_PAIRED
                ) {
                    Error::Unauthorized(error.message)
                } else {
                    Error::from(error)
                },
            );
        }
        let payload = response.payload.unwrap_or(Value::Null);
        return serde_json::from_value(payload)
            .map_err(|e| Error::Handshake(format!("invalid hello: {e}")));
    }
    Err(Error::Handshake(
        "connection closed during handshake".into(),
    ))
}

async fn write_loop(
    mut sink: futures::stream::SplitSink<WsStream, Message>,
    mut outgoing: mpsc::UnboundedReceiver<Message>,
) {
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.tick().await;
    loop {
        let message = tokio::select! {
            message = outgoing.recv() => match message {
                Some(message) => message,
                None => break,
            },
            _ = ping.tick() => Message::Ping(Vec::new().into()),
        };
        let closing = matches!(message, Message::Close(_));
        if let Err(_error) = sink.send(message).await {
            #[cfg(feature = "tracing")]
            debug!(error = %_error, "websocket write failed");
            break;
        }
        if closing {
            break;
        }
    }
    let _ = sink.close().await;
}

async fn read_loop(
    mut stream: futures::stream::SplitStream<WsStream>,
    pending: Arc<Pending>,
    events: broadcast::Sender<EventFrame>,
    outgoing: mpsc::UnboundedSender<Message>,
    closed: watch::Sender<Option<String>>,
) {
    let reason = loop {
        let next = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
            Ok(next) => next,
            Err(_) => break "no data from the daemon".to_string(),
        };
        let message = match next {
            Some(Ok(message)) => message,
            Some(Err(error)) => break error.to_string(),
            None => break "connection closed".to_string(),
        };
        let text = match message {
            Message::Text(text) => text,
            Message::Close(frame) => {
                break frame
                    .map(|f| f.reason.to_string())
                    .filter(|r| !r.is_empty())
                    .unwrap_or_else(|| "closed by the daemon".into());
            },
            _ => continue,
        };
        dispatch(&text, &pending, &events, &outgoing);
    };
    #[cfg(feature = "tracing")]
    debug!(%reason, "websocket closed");
    let waiters: Vec<_> = pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain()
        .collect();
    for (_, waiter) in waiters {
        let _ = waiter.send(Err(Error::Disconnected));
    }
    let _ = closed.send(Some(reason));
    let _ = outgoing.send(Message::Close(None));
}

/// Route one text frame.
fn dispatch(
    text: &str,
    pending: &Pending,
    events: &broadcast::Sender<EventFrame>,
    outgoing: &mpsc::UnboundedSender<Message>,
) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        #[cfg(feature = "tracing")]
        warn!("ignoring non-JSON frame");
        return;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("res") => {
            let Ok(response) = serde_json::from_value::<ResponseFrame>(value) else {
                #[cfg(feature = "tracing")]
                warn!("ignoring malformed response frame");
                return;
            };
            let waiter = pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&response.id);
            let Some(waiter) = waiter else {
                return;
            };
            let result = if response.ok {
                Ok(response.payload.unwrap_or(Value::Null))
            } else {
                Err(Error::from(response.error.unwrap_or_else(|| {
                    ErrorShape::new(error_codes::INTERNAL, "request failed")
                })))
            };
            let _ = waiter.send(result);
        },
        Some("event") => {
            let Ok(event) = serde_json::from_value::<EventFrame>(value) else {
                #[cfg(feature = "tracing")]
                warn!("ignoring malformed event frame");
                return;
            };
            #[cfg(feature = "metrics")]
            counter!(metric::EVENTS_RECEIVED_TOTAL).increment(1);
            // No subscribers is fine: nobody is listening yet.
            let _ = events.send(event);
        },
        Some("req") => {
            // v4 lets the daemon call clients; an operator client serves no
            // methods, so answer instead of leaving the daemon waiting.
            let Some(id) = value.get("id").and_then(Value::as_str) else {
                return;
            };
            let reply = ResponseFrame::err(
                id,
                ErrorShape::new(error_codes::UNKNOWN_METHOD, "client serves no methods"),
            );
            if let Ok(text) = serde_json::to_string(&reply) {
                let _ = outgoing.send(Message::Text(text.into()));
            }
        },
        _ => {},
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
