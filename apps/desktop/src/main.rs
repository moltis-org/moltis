//! Moltis Desktop: a native client for a running moltis daemon.
//!
//! The daemon runs on its own (installed as a user service); this app
//! connects to it, locally or remotely, and can quit without stopping it.

mod actions;
mod app;
mod assets;
#[cfg(feature = "capture")]
mod capture;
mod chat_view;
mod daemon;
mod input;
mod local_daemon;
mod logging;
mod markdown;
#[cfg(feature = "mockup")]
mod mockup;
mod runtime;
mod servers;
mod servers_view;
mod sessions;
mod sidebar;
mod status_view;
mod theme;
mod titlebar;
mod transcript;
mod ui;
mod workspace;

fn main() {
    #[cfg(feature = "tracing")]
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("MOLTIS_DESKTOP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    // Look where the daemon looks: its CLI applies MOLTIS_CONFIG_DIR as an
    // override, which config file discovery honours (the variable alone is not).
    if let Some(dir) = std::env::var_os("MOLTIS_CONFIG_DIR").filter(|d| !d.is_empty()) {
        moltis_config::set_config_dir(dir.into());
    }
    #[cfg(feature = "mockup")]
    if let Some(options) = mockup::Options::from_args(std::env::args().skip(1)) {
        return mockup::run(options);
    }
    app::run();
}
