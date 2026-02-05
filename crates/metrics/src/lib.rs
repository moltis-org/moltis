//! Metrics collection and export for moltis.
//!
//! This crate provides a unified metrics interface using the `metrics` crate facade.
//! When the `prometheus` feature is enabled, metrics are exported in Prometheus format.
//! When the `tracing` feature is enabled, span context is propagated to metrics labels.
//!
//! # Usage
//!
//! ```rust,ignore
//! use moltis_metrics::{counter, gauge, histogram};
//!
//! // Record metrics using the facade macros
//! counter!("http_requests_total", "endpoint" => "/api/chat").increment(1);
//! gauge!("active_sessions").set(42.0);
//! histogram!("request_duration_seconds").record(0.123);
//! ```
//!
//! # Features
//!
//! - `prometheus`: Enable Prometheus metrics export via `/metrics` endpoint
//! - `tracing`: Enable tracing span context propagation to metrics labels

mod definitions;
mod recorder;
mod snapshot;
#[cfg(feature = "sqlite")]
mod store;
pub mod tracing_integration;

pub use {
    definitions::*,
    recorder::{MetricsHandle, MetricsRecorderConfig, init_metrics},
    snapshot::{MetricSnapshot, MetricType, MetricsSnapshot},
};

#[cfg(feature = "sqlite")]
pub use store::{MetricsHistoryPoint, MetricsStore, ProviderTokens, SqliteMetricsStore};

// Re-export metrics macros for convenience
pub use metrics::{counter, gauge, histogram};
