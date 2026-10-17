//! An in-process mock daemon speaking the v4 WebSocket protocol, for tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use {
    futures::{SinkExt, StreamExt},
    moltis_protocol::{
        ErrorShape, EventFrame, Extensions, Features, HelloOk, Policy, RequestFrame, ResponseFrame,
        ServerInfo, error_codes,
    },
    serde_json::{Value, json},
    tokio::net::TcpListener,
    tokio_tungstenite::tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
        http::StatusCode,
    },
};

use crate::{
    connection::{ClientIdentity, ConnectOptions},
    endpoint::Endpoint,
    tls::TlsTrust,
};

/// Behaviour of the mock daemon.
#[derive(Clone, Default)]
pub struct MockConfig {
    /// Reject connects whose `auth.api_key` differs.
    pub required_key: Option<String>,
    /// Answer the HTTP upgrade with 401 instead of a WebSocket.
    pub reject_upgrade: bool,
}

/// What the mock saw, shared with the test.
#[derive(Default)]
pub struct MockLog {
    pub connections: AtomicUsize,
    pub methods: Mutex<Vec<(String, Value)>>,
    pub client_replies: Mutex<Vec<Value>>,
    pub authorization: Mutex<Vec<Option<String>>>,
}

pub struct MockServer {
    pub addr: SocketAddr,
    pub log: Arc<MockLog>,
    task: tokio::task::JoinHandle<()>,
    tls: bool,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockServer {
    pub async fn start(config: MockConfig) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        Self::serve(listener, config)
    }

    /// Serve on an already bound listener (to restart on the same port).
    pub fn serve(listener: TcpListener, config: MockConfig) -> Self {
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(MockLog::default());
        let task_log = Arc::clone(&log);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let log = Arc::clone(&task_log);
                let config = config.clone();
                tokio::spawn(handle(stream, config, log));
            }
        });
        Self {
            addr,
            log,
            task,
            tls: false,
        }
    }

    /// Serve over TLS with a leaf certificate for `localhost` signed by a
    /// fresh CA. Returns the server, the leaf DER and the CA PEM.
    pub async fn start_tls(config: MockConfig) -> (Self, Vec<u8>, String) {
        use {
            rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair},
            rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
        };

        let ca_key = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let leaf_key = KeyPair::generate().unwrap();
        let leaf = CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .signed_by(&leaf_key, &ca, &ca_key)
            .unwrap();
        let leaf_der = leaf.der().to_vec();

        let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(MockLog::default());
        let task_log = Arc::clone(&log);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let log = Arc::clone(&task_log);
                let config = config.clone();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Ok(tls) = acceptor.accept(stream).await {
                        handle(tls, config, log).await;
                    }
                });
            }
        });
        (
            Self {
                addr,
                log,
                task,
                tls: true,
            },
            leaf_der,
            ca.pem(),
        )
    }

    pub fn endpoint(&self) -> Endpoint {
        if self.tls {
            return Endpoint::parse(
                &format!("https://localhost:{}", self.addr.port()),
                TlsTrust::System,
            )
            .unwrap();
        }
        Endpoint::parse(&format!("http://{}", self.addr), TlsTrust::System).unwrap()
    }

    pub fn options(&self) -> ConnectOptions {
        let mut options =
            ConnectOptions::new(self.endpoint(), ClientIdentity::new("test-client", "0.0.0"));
        options.request_timeout = std::time::Duration::from_secs(2);
        options
    }

    pub fn methods(&self) -> Vec<String> {
        self.log
            .methods
            .lock()
            .unwrap()
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }
}

fn hello() -> HelloOk {
    HelloOk {
        r#type: "hello-ok".into(),
        protocol: 4,
        server: ServerInfo {
            version: "test".into(),
            commit: None,
            host: Some("mock".into()),
            conn_id: "conn-1".into(),
        },
        features: Features {
            methods: vec!["echo".into()],
            events: vec!["chat".into()],
        },
        snapshot: json!({}),
        canvas_host_url: None,
        auth: None,
        policy: Policy::default(),
        extensions: Extensions::new(),
    }
}

fn text(value: &impl serde::Serialize) -> Message {
    Message::Text(serde_json::to_string(value).unwrap().into())
}

async fn handle<S>(stream: S, config: MockConfig, log: Arc<MockLog>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let auth_log = Arc::clone(&log);
    let reject = config.reject_upgrade;
    #[allow(clippy::result_large_err)]
    let callback = move |request: &Request, response: Response| {
        auth_log.authorization.lock().unwrap().push(
            request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .map(String::from),
        );
        if reject {
            let mut error = ErrorResponse::new(Some("unauthorized".into()));
            *error.status_mut() = StatusCode::UNAUTHORIZED;
            return Err(error);
        }
        Ok(response)
    };
    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    log.connections.fetch_add(1, Ordering::SeqCst);
    let mut seq = 0;
    while let Some(Ok(message)) = ws.next().await {
        let Message::Text(raw) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&raw).unwrap();
        if value["type"] == "res" {
            log.client_replies.lock().unwrap().push(value);
            continue;
        }
        let request: RequestFrame = serde_json::from_value(value).unwrap();
        let params = request.params.clone().unwrap_or(Value::Null);
        log.methods
            .lock()
            .unwrap()
            .push((request.method.clone(), params.clone()));
        let reply = match request.method.as_str() {
            "connect" => {
                let key = params["auth"]["api_key"].as_str().map(String::from);
                match &config.required_key {
                    Some(required) if key.as_deref() != Some(required) => ResponseFrame::err(
                        &request.id,
                        ErrorShape::new(error_codes::UNAUTHORIZED, "bad key"),
                    ),
                    _ => ResponseFrame::ok(&request.id, serde_json::to_value(hello()).unwrap()),
                }
            },
            "echo" | "subscribe" => ResponseFrame::ok(&request.id, params),
            "fail" => ResponseFrame::err(
                &request.id,
                ErrorShape::new(error_codes::NOT_FOUND, "no such thing"),
            ),
            "emit" => {
                seq += 1;
                let event = EventFrame::new(
                    params["event"].as_str().unwrap_or("chat"),
                    params["payload"].clone(),
                    seq,
                );
                ws.send(text(&event)).await.unwrap();
                ResponseFrame::ok(&request.id, json!({"sent": true}))
            },
            "ask_client" => {
                let call = RequestFrame {
                    r#type: "req".into(),
                    id: "srv-1".into(),
                    method: "location.get".into(),
                    params: None,
                    channel: None,
                };
                ws.send(text(&call)).await.unwrap();
                ResponseFrame::ok(&request.id, json!({}))
            },
            "slow" => continue,
            "drop" => {
                let _ = ws.close(None).await;
                return;
            },
            _ => ResponseFrame::err(
                &request.id,
                ErrorShape::new(error_codes::UNKNOWN_METHOD, "unknown"),
            ),
        };
        if ws.send(text(&reply)).await.is_err() {
            return;
        }
        if request.method == "connect" && !reply.ok {
            let _ = ws.close(None).await;
            return;
        }
    }
}
