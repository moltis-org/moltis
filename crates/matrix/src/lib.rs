pub mod access;
pub mod client;
pub mod config;
pub mod error;
pub mod handlers;
pub mod markdown;
pub mod media;
pub mod otp;
pub mod outbound;
pub mod plugin;
pub mod state;
pub mod stream;

pub use {
    config::MatrixAccountConfig,
    error::{Error, Result},
    plugin::MatrixPlugin,
};
