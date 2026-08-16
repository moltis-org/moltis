//! Langfuse backend.
//!
//! Traces reach Langfuse over OTLP — that is Langfuse's own modern ingest path,
//! the one its v3+ Python and v4+ JS SDKs use, and the only one that carries the
//! full observation taxonomy (`AGENT`, `TOOL`, `RETRIEVER`, ...). The native
//! `/api/public/ingestion` event `observation-create` is marked deprecated
//! upstream and its non-deprecated `span-create`/`generation-create` pair
//! cannot express the newer types at all.
//!
//! Everything OTLP *cannot* express lives in the REST modules here: scores,
//! managed prompts, datasets and experiment runs, media, and aggregated cost.

pub mod client;
pub mod config;
pub mod scores;

pub use {
    client::LangfuseClient,
    config::{
        DAILY_METRICS_PATH, DATASET_ITEMS_PATH, DATASET_RUN_ITEMS_PATH, DATASETS_PATH, HEALTH_PATH,
        INGESTION_PATH, LangfuseConfig, MEDIA_PATH, OTEL_TRACES_PATH, PROMPTS_PATH, SCORES_PATH,
    },
    scores::{ScoreSink, ScoreTransport},
};
