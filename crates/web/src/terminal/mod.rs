mod auth;
mod handlers;
mod pty;
mod tmux;
mod types;
mod websocket;

// Re-export the public API (same surface as the original single-file module).
pub use handlers::{
    api_terminal_windows_create_handler, api_terminal_windows_handler,
    api_terminal_ws_upgrade_handler,
};
