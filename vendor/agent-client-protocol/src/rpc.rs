use std::{
    any::Any,
    borrow::Cow,
    collections::{HashMap, VecDeque},
    io,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use {
    agent_client_protocol_schema::{
        Error, JsonRpcMessage, Notification, OutgoingMessage, Request, RequestId, Response, Result,
        Side,
    },
    futures::{
        AsyncBufRead, AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, Future,
        FutureExt as _, StreamExt as _,
        channel::{
            mpsc::{self, UnboundedReceiver, UnboundedSender},
            oneshot,
        },
        future::LocalBoxFuture,
        io::BufReader,
        pin_mut, select_biased,
    },
    serde::{Deserialize, de::DeserializeOwned},
    serde_json::value::RawValue,
};

use super::stream_broadcast::{StreamBroadcast, StreamReceiver, StreamSender};

#[derive(Debug)]
pub(crate) struct RpcConnection<Local: Side, Remote: Side> {
    outgoing: OutgoingSender<Local, Remote>,
    pending_responses: Arc<Mutex<HashMap<RequestId, PendingResponse>>>,
    next_id: AtomicI64,
    broadcast: StreamBroadcast,
}

/// Resource limits for an ACP connection.
///
/// The default leaves limits disabled to preserve the behavior of the upstream
/// constructors. Use [`ConnectionLimits::bounded`] for untrusted peers.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConnectionLimits {
    max_buffered_outgoing_bytes: Option<usize>,
    max_incoming_frame_bytes: Option<usize>,
    max_inflight_messages: Option<usize>,
}

impl ConnectionLimits {
    /// Creates limits for queued output, incoming frames, and concurrent
    /// incoming message handlers. Frame byte limits include the trailing newline.
    #[must_use]
    pub const fn bounded(
        max_buffered_outgoing_bytes: usize,
        max_incoming_frame_bytes: usize,
        max_inflight_messages: usize,
    ) -> Self {
        Self {
            max_buffered_outgoing_bytes: Some(max_buffered_outgoing_bytes),
            max_incoming_frame_bytes: Some(max_incoming_frame_bytes),
            max_inflight_messages: Some(max_inflight_messages),
        }
    }
}

struct OutgoingSender<Local: Side, Remote: Side> {
    tx: UnboundedSender<QueuedOutgoingMessage<Local, Remote>>,
    fatal_tx: UnboundedSender<Error>,
    budget: Option<ByteBudget>,
    max_frame_bytes: Option<usize>,
}

impl<Local: Side, Remote: Side> std::fmt::Debug for OutgoingSender<Local, Remote> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OutgoingSender")
            .finish_non_exhaustive()
    }
}

impl<Local: Side, Remote: Side> Clone for OutgoingSender<Local, Remote> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            fatal_tx: self.fatal_tx.clone(),
            budget: self.budget.clone(),
            max_frame_bytes: self.max_frame_bytes,
        }
    }
}

struct PreparedOutgoingMessage<Local: Side, Remote: Side> {
    message: OutgoingMessage<Local, Remote>,
    line: Vec<u8>,
}

struct QueuedOutgoingMessage<Local: Side, Remote: Side> {
    prepared: PreparedOutgoingMessage<Local, Remote>,
    _permit: Option<BytePermit>,
}

impl<Local: Side, Remote: Side> OutgoingSender<Local, Remote> {
    fn prepare(
        message: OutgoingMessage<Local, Remote>,
        max_frame_bytes: Option<usize>,
    ) -> Result<PreparedOutgoingMessage<Local, Remote>> {
        let mut line = if let Some(max_frame_bytes) = max_frame_bytes {
            let mut writer = LimitedVec::new(max_frame_bytes.saturating_sub(1));
            serde_json::to_writer(&mut writer, &JsonRpcMessage::wrap(&message))
                .map_err(Error::into_internal_error)?;
            writer.into_inner()
        } else {
            serde_json::to_vec(&JsonRpcMessage::wrap(&message))
                .map_err(Error::into_internal_error)?
        };
        line.push(b'\n');
        Ok(PreparedOutgoingMessage { message, line })
    }

    async fn prepare_and_reserve(
        &self,
        message: OutgoingMessage<Local, Remote>,
    ) -> Result<QueuedOutgoingMessage<Local, Remote>> {
        let mut permit = if let Some(budget) = &self.budget {
            Some(
                budget
                    .acquire(self.max_frame_bytes.unwrap_or_default())
                    .await?,
            )
        } else {
            None
        };
        let prepared = Self::prepare(message, self.max_frame_bytes)?;
        if let Some(permit) = &mut permit {
            permit.shrink_to(prepared.line.len());
        }
        Ok(QueuedOutgoingMessage {
            prepared,
            _permit: permit,
        })
    }

    fn enqueue(&self, message: QueuedOutgoingMessage<Local, Remote>) -> Result<()> {
        self.tx.unbounded_send(message).map_err(|_| {
            Error::internal_error().data("connection closed before message could be sent")
        })
    }

    async fn send(&self, message: OutgoingMessage<Local, Remote>) -> Result<()> {
        let queued = self.prepare_and_reserve(message).await?;
        self.enqueue(queued)
    }

    async fn send_response(&self, message: OutgoingMessage<Local, Remote>) -> Result<()> {
        let queued = match self.prepare_and_reserve(message).await {
            Ok(queued) => queued,
            Err(error) => {
                if let Some(budget) = &self.budget {
                    budget.close(error.clone());
                }
                self.fatal_tx.unbounded_send(error.clone()).ok();
                return Err(error);
            },
        };
        self.enqueue(queued)
    }
}

#[derive(Clone, Debug)]
struct ByteBudget {
    state: Arc<Mutex<ByteBudgetState>>,
}

#[derive(Debug)]
struct ByteBudgetState {
    capacity: usize,
    used: usize,
    closed: Option<Error>,
    next_waiter_id: u64,
    waiters: VecDeque<ByteWaiter>,
}

#[derive(Debug)]
struct ByteWaiter {
    id: u64,
    waker: Waker,
}

impl ByteBudget {
    fn new(capacity: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(ByteBudgetState {
                capacity,
                used: 0,
                closed: None,
                next_waiter_id: 0,
                waiters: VecDeque::new(),
            })),
        }
    }

    fn acquire(&self, bytes: usize) -> AcquireBytes {
        AcquireBytes {
            budget: self.clone(),
            bytes,
            waiter_id: None,
        }
    }

    fn close(&self, error: Error) {
        let waiters = {
            let mut state = self.state.lock().unwrap();
            if state.closed.is_none() {
                state.closed = Some(error);
            }
            state
                .waiters
                .drain(..)
                .map(|waiter| waiter.waker)
                .collect::<Vec<_>>()
        };
        for waker in waiters {
            waker.wake();
        }
    }
}

#[derive(Debug)]
struct AcquireBytes {
    budget: ByteBudget,
    bytes: usize,
    waiter_id: Option<u64>,
}

impl Future for AcquireBytes {
    type Output = Result<BytePermit>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.budget.state.lock().unwrap();
        if let Some(error) = state.closed.clone() {
            return Poll::Ready(Err(error));
        }
        if self.bytes > state.capacity {
            return Poll::Ready(Err(
                Error::internal_error().data("outgoing ACP frame exceeds byte budget")
            ));
        }

        if let Some(waiter_id) = self.waiter_id {
            let Some(position) = state
                .waiters
                .iter()
                .position(|waiter| waiter.id == waiter_id)
            else {
                return Poll::Ready(Err(Error::internal_error().data("connection closed")));
            };
            if position == 0 && state.used + self.bytes <= state.capacity {
                state.waiters.pop_front();
                state.used += self.bytes;
                drop(state);
                self.waiter_id = None;
                return Poll::Ready(Ok(BytePermit {
                    budget: self.budget.clone(),
                    bytes: self.bytes,
                }));
            }
            state.waiters[position].waker.clone_from(cx.waker());
            return Poll::Pending;
        }

        if state.waiters.is_empty() && state.used + self.bytes <= state.capacity {
            state.used += self.bytes;
            return Poll::Ready(Ok(BytePermit {
                budget: self.budget.clone(),
                bytes: self.bytes,
            }));
        }

        let waiter_id = state.next_waiter_id;
        state.next_waiter_id = state.next_waiter_id.wrapping_add(1);
        state.waiters.push_back(ByteWaiter {
            id: waiter_id,
            waker: cx.waker().clone(),
        });
        drop(state);
        self.waiter_id = Some(waiter_id);
        Poll::Pending
    }
}

impl Drop for AcquireBytes {
    fn drop(&mut self) {
        let Some(waiter_id) = self.waiter_id else {
            return;
        };
        let wake = {
            let mut state = self.budget.state.lock().unwrap();
            let was_front = state
                .waiters
                .front()
                .is_some_and(|waiter| waiter.id == waiter_id);
            if let Some(position) = state
                .waiters
                .iter()
                .position(|waiter| waiter.id == waiter_id)
            {
                state.waiters.remove(position);
            }
            was_front
                .then(|| state.waiters.front().map(|waiter| waiter.waker.clone()))
                .flatten()
        };
        if let Some(waker) = wake {
            waker.wake();
        }
    }
}

#[derive(Debug)]
struct BytePermit {
    budget: ByteBudget,
    bytes: usize,
}

impl Drop for BytePermit {
    fn drop(&mut self) {
        let wake = {
            let mut state = self.budget.state.lock().unwrap();
            state.used = state.used.saturating_sub(self.bytes);
            state.waiters.front().map(|waiter| waiter.waker.clone())
        };
        if let Some(waker) = wake {
            waker.wake();
        }
    }
}

impl BytePermit {
    fn shrink_to(&mut self, bytes: usize) {
        let released = self.bytes.saturating_sub(bytes);
        self.bytes = bytes;
        let wake = {
            let mut state = self.budget.state.lock().unwrap();
            state.used = state.used.saturating_sub(released);
            state.waiters.front().map(|waiter| waiter.waker.clone())
        };
        if let Some(waker) = wake {
            waker.wake();
        }
    }
}

#[derive(Clone)]
struct InflightBudget {
    active: Arc<AtomicUsize>,
    limit: Option<usize>,
}

impl InflightBudget {
    fn new(limit: Option<usize>) -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(0)),
            limit,
        }
    }

    fn try_acquire(&self) -> Result<InflightPermit> {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (self.limit.is_none_or(|limit| active < limit)).then_some(active + 1)
            })
            .map_err(|_| Error::internal_error().data("too many in-flight ACP messages"))?;
        Ok(InflightPermit(self.active.clone()))
    }
}

struct InflightPermit(Arc<AtomicUsize>);

impl Drop for InflightPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct LimitedVec {
    bytes: Vec<u8>,
    limit: usize,
}

impl LimitedVec {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl io::Write for LimitedVec {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "outgoing ACP frame exceeds byte budget",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct PendingResponse {
    deserialize: fn(&serde_json::value::RawValue) -> Result<Box<dyn Any + Send>>,
    respond: oneshot::Sender<Result<Box<dyn Any + Send>>>,
}

struct PendingRequestGuard {
    id: RequestId,
    pending_responses: Arc<Mutex<HashMap<RequestId, PendingResponse>>>,
}

impl Drop for PendingRequestGuard {
    fn drop(&mut self) {
        self.pending_responses.lock().unwrap().remove(&self.id);
    }
}

impl<Local, Remote> RpcConnection<Local, Remote>
where
    Local: Side + 'static,
    Remote: Side + 'static,
{
    pub(crate) fn new<Handler>(
        handler: Handler,
        outgoing_bytes: impl Unpin + AsyncWrite,
        incoming_bytes: impl Unpin + AsyncRead,
        spawn: impl Fn(LocalBoxFuture<'static, ()>) + 'static,
    ) -> (Self, impl futures::Future<Output = Result<()>>)
    where
        Handler: MessageHandler<Local> + 'static,
    {
        Self::new_with_limits(
            handler,
            outgoing_bytes,
            incoming_bytes,
            ConnectionLimits::default(),
            spawn,
        )
    }

    pub(crate) fn new_with_limits<Handler>(
        handler: Handler,
        outgoing_bytes: impl Unpin + AsyncWrite,
        incoming_bytes: impl Unpin + AsyncRead,
        limits: ConnectionLimits,
        spawn: impl Fn(LocalBoxFuture<'static, ()>) + 'static,
    ) -> (Self, impl futures::Future<Output = Result<()>>)
    where
        Handler: MessageHandler<Local> + 'static,
    {
        let (incoming_tx, incoming_rx) = mpsc::unbounded();
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded();
        let (fatal_tx, fatal_rx) = mpsc::unbounded();
        let budget = limits.max_buffered_outgoing_bytes.map(ByteBudget::new);
        let inflight = InflightBudget::new(limits.max_inflight_messages);
        let outgoing = OutgoingSender {
            tx: outgoing_tx,
            fatal_tx,
            budget: budget.clone(),
            max_frame_bytes: limits.max_buffered_outgoing_bytes,
        };

        let pending_responses = Arc::new(Mutex::new(HashMap::default()));
        let (broadcast_tx, broadcast) = StreamBroadcast::new();

        let shutdown = ConnectionShutdownGuard::new(budget, pending_responses.clone());
        let io_task = {
            let pending_responses = pending_responses.clone();
            async move {
                Self::handle_io(
                    incoming_tx,
                    outgoing_rx,
                    outgoing_bytes,
                    incoming_bytes,
                    pending_responses.clone(),
                    broadcast_tx,
                    inflight,
                    fatal_rx,
                    limits.max_incoming_frame_bytes,
                    shutdown,
                )
                .await
            }
        };

        Self::handle_incoming(outgoing.clone(), incoming_rx, handler, spawn);

        let this = Self {
            outgoing,
            pending_responses,
            next_id: AtomicI64::new(0),
            broadcast,
        };

        (this, io_task)
    }

    pub(crate) fn subscribe(&self) -> StreamReceiver {
        self.broadcast.receiver()
    }

    pub(crate) async fn notify(
        &self,
        method: impl Into<Arc<str>>,
        params: Option<Remote::InNotification>,
    ) -> Result<()> {
        self.outgoing
            .send(OutgoingMessage::Notification(Notification {
                method: method.into(),
                params,
            }))
            .await
    }

    #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn request<Out: DeserializeOwned + Send + 'static>(
        &self,
        method: impl Into<Arc<str>>,
        params: Option<Remote::InRequest>,
    ) -> Result<impl Future<Output = Result<Out>>> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let id = RequestId::Number(id);
        let message = OutgoingMessage::Request(Request {
            id: id.clone(),
            method: method.into(),
            params,
        });
        let outgoing = self.outgoing.clone();
        let pending_responses = self.pending_responses.clone();
        Ok(async move {
            let queued = outgoing.prepare_and_reserve(message).await?;
            let (tx, rx) = oneshot::channel();
            pending_responses
                .lock()
                .unwrap()
                .insert(id.clone(), PendingResponse {
                    deserialize: |value| {
                        serde_json::from_str::<Out>(value.get())
                            .map(|out| Box::new(out) as _)
                            .map_err(|_| {
                                Error::internal_error().data("failed to deserialize response")
                            })
                    },
                    respond: tx,
                });
            if let Err(error) = outgoing.enqueue(queued) {
                pending_responses.lock().unwrap().remove(&id);
                return Err(error);
            }
            let _pending_guard = PendingRequestGuard {
                id: id.clone(),
                pending_responses: pending_responses.clone(),
            };
            let result = rx
                .await
                .map_err(|_| Error::internal_error().data("server shut down unexpectedly"))??
                .downcast::<Out>()
                .map_err(|_| Error::internal_error().data("failed to deserialize response"))?;

            Ok(*result)
        })
    }

    async fn handle_io(
        incoming_tx: UnboundedSender<IncomingDispatch<Local>>,
        outgoing_rx: UnboundedReceiver<QueuedOutgoingMessage<Local, Remote>>,
        outgoing_bytes: impl Unpin + AsyncWrite,
        incoming_bytes: impl Unpin + AsyncRead,
        pending_responses: Arc<Mutex<HashMap<RequestId, PendingResponse>>>,
        broadcast: StreamSender,
        inflight: InflightBudget,
        mut fatal_rx: UnboundedReceiver<Error>,
        max_incoming_frame_bytes: Option<usize>,
        mut shutdown: ConnectionShutdownGuard,
    ) -> Result<()> {
        let reader = Self::read_incoming(
            incoming_tx,
            incoming_bytes,
            pending_responses.clone(),
            broadcast.clone(),
            inflight,
            max_incoming_frame_bytes,
        )
        .fuse();
        let writer = Self::write_outgoing(outgoing_rx, outgoing_bytes, broadcast).fuse();
        let fatal = fatal_rx.next().fuse();
        pin_mut!(reader, writer, fatal);
        let result = select_biased! {
            error = fatal => Err(error.unwrap_or_else(|| Error::internal_error().data("ACP connection failed"))),
            result = writer => result,
            result = reader => result,
        };
        let error = result
            .as_ref()
            .err()
            .cloned()
            .unwrap_or_else(|| Error::internal_error().data("ACP connection closed"));
        shutdown.close(error);
        result
    }

    async fn write_outgoing(
        mut outgoing_rx: UnboundedReceiver<QueuedOutgoingMessage<Local, Remote>>,
        mut outgoing_bytes: impl Unpin + AsyncWrite,
        broadcast: StreamSender,
    ) -> Result<()> {
        while let Some(queued) = outgoing_rx.next().await {
            let PreparedOutgoingMessage { message, line } = &queued.prepared;
            log::trace!("send: {}", String::from_utf8_lossy(line));
            outgoing_bytes
                .write_all(line)
                .await
                .map_err(Error::into_internal_error)?;
            broadcast.outgoing(message);
        }
        Ok(())
    }

    async fn read_incoming(
        incoming_tx: UnboundedSender<IncomingDispatch<Local>>,
        incoming_bytes: impl Unpin + AsyncRead,
        pending_responses: Arc<Mutex<HashMap<RequestId, PendingResponse>>>,
        broadcast: StreamSender,
        inflight: InflightBudget,
        max_incoming_frame_bytes: Option<usize>,
    ) -> Result<()> {
        let mut input_reader = BufReader::new(incoming_bytes);
        let mut incoming_line = Vec::new();
        loop {
            if Self::read_line(
                &mut input_reader,
                &mut incoming_line,
                max_incoming_frame_bytes,
            )
            .await?
                == 0
            {
                break;
            }
            let incoming_line =
                std::str::from_utf8(&incoming_line).map_err(Error::into_internal_error)?;
            log::trace!("recv: {incoming_line}");

            match serde_json::from_str::<RawIncomingMessage<'_>>(incoming_line) {
                Ok(message) => {
                    if let Some(id) = message.id {
                        if let Some(method) = message.method {
                            // Request
                            match Local::decode_request(&method, message.params) {
                                Ok(request) => {
                                    broadcast.incoming_request(id.clone(), &*method, &request);
                                    let permit = inflight.try_acquire()?;
                                    if incoming_tx
                                        .unbounded_send(IncomingDispatch::Message {
                                            message: IncomingMessage::Request { id, request },
                                            permit,
                                        })
                                        .is_err()
                                    {
                                        log::warn!("failed to send request to handler");
                                    }
                                },
                                Err(error) => {
                                    let permit = inflight.try_acquire()?;
                                    if incoming_tx
                                        .unbounded_send(IncomingDispatch::ErrorResponse {
                                            id,
                                            error,
                                            permit,
                                        })
                                        .is_err()
                                    {
                                        log::warn!("failed to send error response to handler");
                                    }
                                },
                            }
                        } else if let Some(pending_response) =
                            pending_responses.lock().unwrap().remove(&id)
                        {
                            // Response
                            if let Some(result_value) = message.result {
                                broadcast.incoming_response(id, Ok(Some(result_value)));

                                let result = (pending_response.deserialize)(result_value);
                                pending_response.respond.send(result).ok();
                            } else if let Some(error) = message.error {
                                broadcast.incoming_response(id, Err(&error));

                                pending_response.respond.send(Err(error)).ok();
                            } else {
                                broadcast.incoming_response(id, Ok(None));

                                let result = (pending_response.deserialize)(
                                    &RawValue::from_string("null".into()).unwrap(),
                                );
                                pending_response.respond.send(result).ok();
                            }
                        } else {
                            log::error!("received response for unknown request id: {id:?}");
                        }
                    } else if let Some(method) = message.method {
                        // Notification
                        match Local::decode_notification(&method, message.params) {
                            Ok(notification) => {
                                broadcast.incoming_notification(&*method, &notification);
                                let permit = inflight.try_acquire()?;
                                if incoming_tx
                                    .unbounded_send(IncomingDispatch::Message {
                                        message: IncomingMessage::Notification { notification },
                                        permit,
                                    })
                                    .is_err()
                                {
                                    log::warn!("failed to send notification to handler");
                                }
                            },
                            Err(err) => {
                                log::error!("failed to decode {:?}: {err}", message.params);
                            },
                        }
                    } else {
                        log::error!("received message with neither id nor method");
                    }
                },
                Err(error) => {
                    log::error!("failed to parse incoming message: {error}. Raw: {incoming_line}");
                },
            }
        }
        Ok(())
    }

    async fn read_line(
        reader: &mut (impl AsyncBufRead + Unpin),
        output: &mut Vec<u8>,
        max_bytes: Option<usize>,
    ) -> Result<usize> {
        output.clear();
        loop {
            let available = reader
                .fill_buf()
                .await
                .map_err(Error::into_internal_error)?;
            if available.is_empty() {
                return Ok(output.len());
            }
            let consumed = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |position| position + 1);
            if max_bytes.is_some_and(|max_bytes| output.len().saturating_add(consumed) > max_bytes)
            {
                return Err(Error::internal_error().data("incoming ACP frame exceeds byte limit"));
            }
            let complete = available.get(consumed.saturating_sub(1)) == Some(&b'\n');
            output.extend_from_slice(&available[..consumed]);
            Pin::new(&mut *reader).consume(consumed);
            if complete {
                return Ok(output.len());
            }
        }
    }

    fn handle_incoming<Handler: MessageHandler<Local> + 'static>(
        outgoing: OutgoingSender<Local, Remote>,
        mut incoming_rx: UnboundedReceiver<IncomingDispatch<Local>>,
        handler: Handler,
        spawn: impl Fn(LocalBoxFuture<'static, ()>) + 'static,
    ) {
        let spawn = Rc::new(spawn);
        let handler = Rc::new(handler);
        spawn({
            let spawn = spawn.clone();
            async move {
                while let Some(message) = incoming_rx.next().await {
                    match message {
                        IncomingDispatch::ErrorResponse { id, error, permit } => {
                            let outgoing = outgoing.clone();
                            spawn(
                                async move {
                                    let _permit = permit;
                                    let _send_result = outgoing
                                        .send_response(OutgoingMessage::Response(Response::Error {
                                            id,
                                            error,
                                        }))
                                        .await;
                                }
                                .boxed_local(),
                            );
                        },
                        IncomingDispatch::Message { message, permit } => match message {
                            IncomingMessage::Request { id, request } => {
                                let outgoing = outgoing.clone();
                                let handler = handler.clone();
                                spawn(
                                    async move {
                                        let _permit = permit;
                                        let result = handler.handle_request(request).await;
                                        let _send_result = outgoing
                                            .send_response(OutgoingMessage::Response(
                                                Response::new(id, result),
                                            ))
                                            .await;
                                    }
                                    .boxed_local(),
                                );
                            },
                            IncomingMessage::Notification { notification } => {
                                let handler = handler.clone();
                                spawn(
                                    async move {
                                        let _permit = permit;
                                        if let Err(err) =
                                            handler.handle_notification(notification).await
                                        {
                                            log::error!("failed to handle notification: {err:?}");
                                        }
                                    }
                                    .boxed_local(),
                                );
                            },
                        },
                    }
                }
            }
            .boxed_local()
        });
    }
}

fn fail_pending_responses(
    pending_responses: &Arc<Mutex<HashMap<RequestId, PendingResponse>>>,
    error: Error,
) {
    let pending = pending_responses
        .lock()
        .unwrap()
        .drain()
        .map(|(_, pending)| pending)
        .collect::<Vec<_>>();
    for pending in pending {
        pending.respond.send(Err(error.clone())).ok();
    }
}

struct ConnectionShutdownGuard {
    budget: Option<ByteBudget>,
    pending_responses: Arc<Mutex<HashMap<RequestId, PendingResponse>>>,
    closed: bool,
}

impl ConnectionShutdownGuard {
    fn new(
        budget: Option<ByteBudget>,
        pending_responses: Arc<Mutex<HashMap<RequestId, PendingResponse>>>,
    ) -> Self {
        Self {
            budget,
            pending_responses,
            closed: false,
        }
    }

    fn close(&mut self, error: Error) {
        if let Some(budget) = &self.budget {
            budget.close(error.clone());
        }
        fail_pending_responses(&self.pending_responses, error);
        self.closed = true;
    }
}

impl Drop for ConnectionShutdownGuard {
    fn drop(&mut self) {
        if !self.closed {
            self.close(Error::internal_error().data("ACP connection task stopped"));
        }
    }
}

enum IncomingDispatch<Local: Side> {
    Message {
        message: IncomingMessage<Local>,
        permit: InflightPermit,
    },
    ErrorResponse {
        id: RequestId,
        error: Error,
        permit: InflightPermit,
    },
}

#[derive(Debug, Deserialize)]
pub struct RawIncomingMessage<'a> {
    id: Option<RequestId>,
    #[serde(borrow)]
    method: Option<Cow<'a, str>>,
    #[serde(borrow)]
    params: Option<&'a RawValue>,
    #[serde(borrow)]
    result: Option<&'a RawValue>,
    error: Option<Error>,
}

#[derive(Debug)]
pub enum IncomingMessage<Local: Side> {
    Request {
        id: RequestId,
        request: Local::InRequest,
    },
    Notification {
        notification: Local::InNotification,
    },
}

pub trait MessageHandler<Local: Side> {
    fn handle_request(
        &self,
        request: Local::InRequest,
    ) -> impl Future<Output = Result<Local::OutResponse>>;

    fn handle_notification(
        &self,
        notification: Local::InNotification,
    ) -> impl Future<Output = Result<()>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn byte_budget_blocks_until_permit_is_released() {
        let budget = ByteBudget::new(10);
        let first = budget.acquire(6).await.unwrap();
        let second = budget.acquire(5);
        pin_mut!(second);
        assert!(futures::poll!(&mut second).is_pending());

        drop(first);
        assert_eq!(second.await.unwrap().bytes, 5);
    }

    #[tokio::test]
    async fn byte_budget_rejects_oversized_acquisitions() {
        let error = ByteBudget::new(10).acquire(11).await.unwrap_err();
        assert!(error.data.is_some());
    }

    #[tokio::test]
    async fn closing_byte_budget_wakes_waiters() {
        let budget = ByteBudget::new(10);
        let _first = budget.acquire(10).await.unwrap();
        let second = budget.acquire(1);
        pin_mut!(second);
        assert!(futures::poll!(&mut second).is_pending());

        budget.close(Error::internal_error().data("writer failed"));
        assert!(second.await.is_err());
    }

    #[test]
    fn test_raw_incoming_message_with_escaped_slash() {
        // JSON with escaped forward slash in method name (valid per RFC 8259).
        // Some JSON encoders (especially behind WebSocket proxies) produce
        // `\/` instead of `/`.  The Cow<str> field in RawIncomingMessage ensures
        // serde can allocate a new String when unescaping is required.
        //
        // Before the fix, this would fail because `&'a str` cannot hold an
        // unescaped value that differs from the source bytes.
        let json_str = r#"{"jsonrpc":"2.0","id":1,"method":"session\/update","params":{}}"#;
        let parsed: RawIncomingMessage<'_> = serde_json::from_str(json_str).unwrap();
        assert_eq!(parsed.method.unwrap(), "session/update");
        assert_eq!(parsed.params.unwrap().to_string(), "{}");
    }

    #[test]
    fn test_raw_incoming_message_without_escape() {
        // Normal method name without escapes should still work (zero-copy borrow via Cow::Borrowed).
        let json_str = r#"{"jsonrpc":"2.0","id":2,"method":"session/update","params":{}}"#;
        let parsed: RawIncomingMessage<'_> = serde_json::from_str(json_str).unwrap();
        assert_eq!(parsed.method.unwrap(), "session/update");
        assert_eq!(parsed.params.unwrap().to_string(), "{}");
    }
}
