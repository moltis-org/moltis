//! Moltis Desktop: a native client for a running moltis daemon.
//!
//! The daemon runs on its own (installed as a user service); this app
//! connects to it, locally or remotely, and can quit without stopping it.

mod actions;
mod app;
#[cfg(feature = "capture")]
mod capture;
mod chat_view;
mod daemon;
mod input;
mod local_daemon;
mod markdown;
mod runtime;
mod servers;
mod servers_view;
mod sessions;
mod sidebar;
mod status_view;
mod theme;
mod transcript;
mod ui;
mod workspace;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("MOLTIS_DESKTOP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    app::run();
}
