//! C ABI bridge for embedding Moltis Rust functionality into native Swift apps.
//!
//! This crate is split into domain modules:
//! - [`types`]: Request/response structs, error types, and constants
//! - [`state`]: Global bridge state (tokio runtime, registry, session stores)
//! - [`callbacks`]: Log, session event, and network audit callback plumbing
//! - [`helpers`]: Encoding, parsing, metrics/tracing, and utility functions
//! - [`chat`]: Chat logic (provider resolution, streaming)
//! - [`ffi_core`]: Core FFI exports (version, providers, httpd, etc.)
//! - [`ffi_sessions`]: Session FFI exports
//! - [`ffi_config`]: Config, identity, soul, memory, and env var FFI exports
//! - [`ffi_auth`]: Authentication FFI exports
//! - [`ffi_sandbox`]: Sandbox FFI exports

mod callbacks;
mod chat;
mod ffi_auth;
mod ffi_config;
mod ffi_core;
mod ffi_sandbox;
mod ffi_sessions;
mod helpers;
mod state;
mod types;
