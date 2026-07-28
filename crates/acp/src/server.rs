//! Transport wiring: run a [`MoltisAgent`] over a byte stream.
//!
//! # stdout is the wire
//!
//! When served over stdio, stdout carries JSON-RPC framing and nothing else. A
//! stray `println!`, or a `tracing` subscriber whose writer defaults to stdout,
//! corrupts the stream and the client disconnects with a parse error. Callers
//! must point logging at stderr before calling [`run_stdio`] — see
//! `moltis acp` in the CLI, and the `stdout_is_only_protocol_framing` test.

use std::{io, rc::Rc, sync::Arc};

use {
    agent_client_protocol as acp,
    bytes::Bytes,
    futures::TryStreamExt,
    tokio::io::{AsyncRead, AsyncWrite},
    tokio_util::{
        codec::{FramedRead, LinesCodec},
        compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt},
        io::StreamReader,
    },
};

use crate::{agent::MoltisAgent, backend::AcpBackend};

/// Maximum size of one newline-delimited JSON-RPC message.
pub const MAX_JSON_RPC_FRAME_BYTES: usize = 4 * 1024 * 1024;

fn bounded_input<R>(input: R) -> impl AsyncRead + Unpin
where
    R: AsyncRead + Unpin,
{
    let frames = FramedRead::new(
        input,
        LinesCodec::new_with_max_length(MAX_JSON_RPC_FRAME_BYTES),
    )
    .map_ok(|line| Bytes::from(format!("{line}\n")))
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
    StreamReader::new(frames)
}

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
    let agent = Rc::new(MoltisAgent::new(Arc::clone(&backend)));
    let (connection, io_task) = acp::AgentSideConnection::new(
        Rc::clone(&agent),
        output.compat_write(),
        bounded_input(input).compat(),
        |future| {
            tokio::task::spawn_local(future);
        },
    );
    // Held here for the lifetime of the connection; the agent keeps only a weak
    // reference so the two do not form a cycle.
    let connection = Rc::new(connection);
    agent.set_connection(&connection);

    let result = io_task
        .await
        .map_err(|error| anyhow::anyhow!("ACP connection failed: {error}"));
    let shutdown = backend.shutdown().await;
    result.and(shutdown)
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

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    async fn oversized_json_rpc_frames_are_rejected() {
        let (mut writer, reader) = tokio::io::duplex(MAX_JSON_RPC_FRAME_BYTES + 2);
        let write = tokio::spawn(async move {
            writer
                .write_all(&vec![b'x'; MAX_JSON_RPC_FRAME_BYTES + 1])
                .await
                .expect("write oversized frame");
            writer.write_all(b"\n").await.expect("write newline");
        });
        let mut bounded = bounded_input(reader);
        let mut output = Vec::new();
        let error = bounded
            .read_to_end(&mut output)
            .await
            .expect_err("oversized frame must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        write.await.expect("writer task");
    }
}
