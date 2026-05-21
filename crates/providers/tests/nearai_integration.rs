//! Live integration tests for the NEAR AI Cloud provider.
//!
//! Requires `NEARAI_API_KEY`. Run with:
//!   cargo test --test nearai_integration -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    futures::StreamExt,
    moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent},
    moltis_providers::{nearai, openai::OpenAiProvider},
    secrecy::Secret,
};

const BASE_URL: &str = "https://cloud-api.near.ai/v1";
const DEFAULT_TEST_MODEL: &str = "zai-org/GLM-5.1-FP8";

fn api_key() -> Secret<String> {
    Secret::new(
        std::env::var("NEARAI_API_KEY").expect("NEARAI_API_KEY must be set for integration tests"),
    )
}

fn test_model() -> String {
    std::env::var("NEARAI_TEST_MODEL").unwrap_or_else(|_| DEFAULT_TEST_MODEL.to_string())
}

fn make_provider() -> OpenAiProvider {
    OpenAiProvider::new_with_name(
        api_key(),
        test_model(),
        BASE_URL.to_string(),
        "nearai".to_string(),
    )
}

#[tokio::test]
#[ignore]
async fn model_catalog_discovers_chat_models() {
    let models = nearai::fetch_models_from_api(BASE_URL.to_string())
        .await
        .expect("NEAR AI model catalog should load");

    assert!(
        models.iter().any(|model| model.id == DEFAULT_TEST_MODEL),
        "{DEFAULT_TEST_MODEL} should be present in the NEAR AI catalog"
    );
    assert!(
        models.iter().all(|model| model.capabilities.is_some()),
        "NEAR AI catalog entries should carry provider capabilities"
    );
}

#[tokio::test]
#[ignore]
async fn probe_succeeds() {
    let provider = make_provider();
    provider
        .probe()
        .await
        .expect("probe should succeed against live NEAR AI Cloud API");
}

#[tokio::test]
#[ignore]
async fn non_streaming_chat_completes() {
    let provider = make_provider();
    let response = provider
        .complete(&[ChatMessage::user("Reply with exactly: NEAR AI OK")], &[])
        .await
        .expect("chat completion should succeed");
    let text = response.text.expect("response should contain text");

    assert!(
        text.to_ascii_lowercase().contains("near ai ok"),
        "unexpected response: {text:?}"
    );
}

#[tokio::test]
#[ignore]
async fn streaming_chat_completes() {
    let provider = make_provider();
    let mut stream = provider.stream(vec![ChatMessage::user(
        "Reply with exactly: NEAR AI STREAM OK",
    )]);
    let mut full_text = String::new();
    let mut saw_done = false;
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta(chunk) => full_text.push_str(&chunk),
            StreamEvent::Done(_) => {
                saw_done = true;
                break;
            },
            StreamEvent::Error(err) => panic!("stream error: {err}"),
            _ => {},
        }
    }

    assert!(saw_done, "stream must emit Done");
    assert!(
        full_text.to_ascii_lowercase().contains("near ai stream ok"),
        "unexpected response: {full_text:?}"
    );
}
