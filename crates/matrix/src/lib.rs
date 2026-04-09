#![recursion_limit = "256"]

pub mod access;
pub mod client;
pub mod config;
pub mod error;
pub mod handler;
pub mod outbound;
pub mod plugin;
pub mod state;
pub mod verification;

pub use plugin::MatrixPlugin;
