mod completion;
mod core;
mod request;
mod streaming;
mod websocket;

// Re-export the struct so submodules can reach it via `super::OpenAiProvider`.
use super::OpenAiProvider;
