//! A client that keeps itself connected.
//!
//! [`ManagedClient::spawn`] runs a supervisor task that connects, subscribes
//! to the requested events, forwards them, and reconnects with backoff when
//! the socket drops. Authentication failures stop the supervisor, because
//! retrying the same credentials cannot succeed. Consumers (the desktop UI)
//! read [`ClientUpdate`]s from one channel and refetch state on every
//! `Connected`, since events may have been missed while disconnected.

use std::{sync::Arc, time::Duration};

#[cfg(feature = "tracing")]
use tracing::{debug, info};
use {
    moltis_protocol::{
        EventFrame, HelloOk,
        client_api::{SubscribeParams, methods},
    },
    serde_json::Value,
    tokio::sync::{Notify, broadcast::error::RecvError, mpsc, watch},
};

use crate::{
    backoff::Backoff,
    connection::{ConnectOptions, Connection},
    error::{Error, FailureKind, Result},
};

/// Progress of a [`ManagedClient`].
#[derive(Debug, Clone)]
pub enum ClientUpdate {
    Connecting {
        attempt: u32,
    },
    Connected {
        hello: Box<HelloOk>,
    },
    /// The socket dropped or a connect attempt failed; retrying after `retry_in`.
    Disconnected {
        reason: String,
        kind: FailureKind,
        retry_in: Duration,
    },
    /// The daemon rejected the credentials; the supervisor stopped.
    AuthFailed {
        reason: String,
    },
    Event(Box<EventFrame>),
    /// `n` events were dropped because the consumer fell behind; refetch.
    Lagged(u64),
}

struct Shared {
    current: watch::Receiver<Option<Connection>>,
    retry_now: Notify,
    shutdown: watch::Sender<bool>,
}

/// Handle to a supervised connection. Clones share it; dropping the last
/// handle does not stop the supervisor, [`ManagedClient::shutdown`] does.
#[derive(Clone)]
pub struct ManagedClient {
    shared: Arc<Shared>,
}

impl ManagedClient {
    /// Start connecting. `subscriptions` are event names (or `"*"`).
    #[must_use]
    pub fn spawn(
        options: ConnectOptions,
        subscriptions: Vec<String>,
    ) -> (Self, mpsc::UnboundedReceiver<ClientUpdate>) {
        Self::spawn_with_backoff(options, subscriptions, Backoff::default())
    }

    #[must_use]
    pub fn spawn_with_backoff(
        options: ConnectOptions,
        subscriptions: Vec<String>,
        backoff: Backoff,
    ) -> (Self, mpsc::UnboundedReceiver<ClientUpdate>) {
        let (updates, updates_rx) = mpsc::unbounded_channel();
        let (current_tx, current) = watch::channel(None);
        let (shutdown, _) = watch::channel(false);
        let shared = Arc::new(Shared {
            current,
            retry_now: Notify::new(),
            shutdown,
        });
        tokio::spawn(supervise(
            options,
            SubscribeParams {
                events: subscriptions,
            },
            backoff,
            Arc::clone(&shared),
            current_tx,
            updates,
        ));
        (Self { shared }, updates_rx)
    }

    /// The live connection, if any.
    #[must_use]
    pub fn connection(&self) -> Option<Connection> {
        self.shared.current.borrow().clone()
    }

    /// Call through the live connection; `Disconnected` while reconnecting.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let connection = self.connection().ok_or(Error::Disconnected)?;
        connection.call(method, params).await
    }

    /// Skip the remaining backoff delay and retry immediately.
    pub fn reconnect_now(&self) {
        self.shared.retry_now.notify_one();
    }

    /// Close the connection and stop reconnecting.
    pub fn shutdown(&self) {
        let _ = self.shared.shutdown.send(true);
    }
}

impl crate::api::Rpc for ManagedClient {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        Self::call(self, method, params).await
    }
}

async fn supervise(
    options: ConnectOptions,
    subscriptions: SubscribeParams,
    mut backoff: Backoff,
    shared: Arc<Shared>,
    current: watch::Sender<Option<Connection>>,
    updates: mpsc::UnboundedSender<ClientUpdate>,
) {
    let mut shutdown = shared.shutdown.subscribe();
    let mut attempt = 0;
    loop {
        if *shutdown.borrow() || updates.is_closed() {
            break;
        }
        attempt += 1;
        let _ = updates.send(ClientUpdate::Connecting { attempt });
        let (reason, kind) = match connect_and_subscribe(&options, &subscriptions).await {
            Ok((connection, mut events)) => {
                #[cfg(feature = "tracing")]
                info!(endpoint = %options.endpoint, "connected to daemon");
                attempt = 0;
                backoff.reset();
                let _ = current.send(Some(connection.clone()));
                let _ = updates.send(ClientUpdate::Connected {
                    hello: Box::new(connection.hello().clone()),
                });
                let reason = loop {
                    tokio::select! {
                        event = events.recv() => match event {
                            Ok(event) => {
                                if updates.send(ClientUpdate::Event(Box::new(event))).is_err() {
                                    connection.close();
                                    return;
                                }
                            },
                            Err(RecvError::Lagged(n)) => {
                                let _ = updates.send(ClientUpdate::Lagged(n));
                            },
                            Err(RecvError::Closed) => break "event stream closed".to_string(),
                        },
                        reason = connection.closed() => break reason,
                        _ = shutdown.changed() => {
                            connection.close();
                            let _ = current.send(None);
                            return;
                        },
                    }
                };
                let _ = current.send(None);
                (reason, FailureKind::Closed)
            },
            Err(error) if error.is_auth() => {
                let _ = updates.send(ClientUpdate::AuthFailed {
                    reason: error.to_string(),
                });
                return;
            },
            Err(error) => (error.to_string(), error.failure_kind()),
        };
        let retry_in = backoff.next_delay();
        #[cfg(feature = "tracing")]
        debug!(%reason, ?retry_in, "daemon connection lost");
        let _ = updates.send(ClientUpdate::Disconnected {
            reason,
            kind,
            retry_in,
        });
        tokio::select! {
            () = tokio::time::sleep(retry_in) => {},
            () = shared.retry_now.notified() => {},
            _ = shutdown.changed() => break,
        }
    }
    let _ = current.send(None);
}

async fn connect_and_subscribe(
    options: &ConnectOptions,
    subscriptions: &SubscribeParams,
) -> Result<(Connection, tokio::sync::broadcast::Receiver<EventFrame>)> {
    let connection = Connection::connect(options).await?;
    // Receive before subscribing so no event between the two is lost.
    let events = connection.events();
    if !subscriptions.events.is_empty() {
        connection
            .call(methods::SUBSCRIBE, serde_json::to_value(subscriptions)?)
            .await?;
    }
    Ok((connection, events))
}

#[cfg(test)]
#[path = "managed_tests.rs"]
mod tests;
