//! A tokio runtime beside gpui's executor.
//!
//! `moltis-client` is tokio-based; gpui runs its own executor. Network work
//! runs on this runtime, and gpui tasks await the results through channels.
//! tokio's channels do not need a tokio runtime to be awaited, so the gpui
//! side can `recv().await` directly.

use std::{future::Future, sync::Arc};

use {
    gpui::{App, Global},
    tokio::sync::oneshot,
};

#[derive(Clone)]
pub struct Tokio(Arc<tokio::runtime::Runtime>);

impl Global for Tokio {}

impl Tokio {
    /// Build the runtime and register it as a gpui global.
    pub fn install(cx: &mut App) -> anyhow::Result<()> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("moltis-net")
            .enable_all()
            .build()?;
        cx.set_global(Self(Arc::new(runtime)));
        Ok(())
    }

    #[must_use]
    pub fn handle(cx: &App) -> tokio::runtime::Handle {
        cx.global::<Self>().0.handle().clone()
    }
}

/// Run `future` on the tokio runtime; await the returned future anywhere.
/// It resolves to `None` only if the task was dropped (runtime shut down).
pub fn spawn<F>(cx: &App, future: F) -> impl Future<Output = Option<F::Output>> + use<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    Tokio::handle(cx).spawn(async move {
        let _ = tx.send(future.await);
    });
    async move { rx.await.ok() }
}
