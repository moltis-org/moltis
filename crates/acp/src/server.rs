//! Transport wiring: run a [`MoltisAgent`] over a byte stream.
//!
//! # stdout is the wire
//!
//! When served over stdio, stdout carries JSON-RPC framing and nothing else. A
//! stray `println!`, or a `tracing` subscriber whose writer defaults to stdout,
//! corrupts the stream and the client disconnects with a parse error. Callers
//! must point logging at stderr before calling [`run_stdio`] — see
//! `moltis acp` in the CLI, and the `stdout_is_only_protocol_framing` test.

use std::{rc::Rc, sync::Arc};

use {
    agent_client_protocol as acp,
    tokio::io::{AsyncRead, AsyncWrite},
    tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt},
};

use crate::{agent::MoltisAgent, backend::AcpBackend};

/// Serves one client over the given streams until the connection closes.
///
/// Must be called from inside a [`tokio::task::LocalSet`]: the protocol handler
/// is `!Send` and its tasks are spawned with `spawn_local`. Use [`run_stdio`]
/// if you do not already have one.
pub async fn serve<R, W>(backend: Arc<dyn AcpBackend>, input: R, output: W) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin + 'static,
    W: AsyncWrite + Unpin + 'static,
{
    let agent = Rc::new(MoltisAgent::new(backend));
    let (connection, io_task) = acp::AgentSideConnection::new(
        Rc::clone(&agent),
        output.compat_write(),
        input.compat(),
        |future| {
            tokio::task::spawn_local(future);
        },
    );
    // Held here for the lifetime of the connection; the agent keeps only a weak
    // reference so the two do not form a cycle.
    let connection = Rc::new(connection);
    agent.set_connection(&connection);

    io_task
        .await
        .map_err(|error| anyhow::anyhow!("ACP connection failed: {error}"))
}

/// Serves one client over stdio, creating the [`tokio::task::LocalSet`].
///
/// Logging must already be pointed away from stdout.
pub async fn run_stdio(backend: Arc<dyn AcpBackend>) -> anyhow::Result<()> {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(serve(backend, tokio::io::stdin(), tokio::io::stdout()))
        .await
}
