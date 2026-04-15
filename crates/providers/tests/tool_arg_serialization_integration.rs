//! Live integration tests for scenario-driven tool argument serialization
//! checks on OpenAI-compatible providers.
//!
//! Requires provider credentials in the environment. Run with:
//!   cargo test -p moltis-providers --test tool_arg_serialization_integration -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use {
    moltis_agents::model::{ChatMessage, LlmProvider},
    moltis_providers::openai::OpenAiProvider,
    secrecy::Secret,
    serde::Deserialize,
};

const SCENARIOS_JSON: &str = include_str!("e2e_scenarios/tool_arg_serialization.json");

#[derive(Debug, Deserialize)]
struct ScenarioSuite {
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
struct Scenario {
    id: String,
    prompt: String,
    tool: serde_json::Value,
    expected_arguments: serde_json::Value,
    providers: BTreeMap<String, ProviderExpectation>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum ProviderExpectation {
    MustPass,
    AllowKnownMismatch {
        observed_arguments: serde_json::Value,
        reason: String,
    },
}

#[derive(Debug)]
struct ProviderConfig {
    provider_name: &'static str,
    api_key_env: &'static str,
    base_url_env: Option<&'static str>,
    default_base_url: &'static str,
    model_env: &'static str,
    default_model: &'static str,
}

fn load_suite() -> ScenarioSuite {
    serde_json::from_str(SCENARIOS_JSON).expect("scenario file must be valid JSON")
}

fn optional_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn configured_provider(config: &ProviderConfig) -> Option<OpenAiProvider> {
    let Some(api_key) = optional_var(config.api_key_env) else {
        eprintln!(
            "skipping {} serialization probe: {} is not set",
            config.provider_name, config.api_key_env
        );
        return None;
    };

    let base_url = match config.base_url_env {
        Some(name) => {
            let Some(value) = optional_var(name) else {
                eprintln!(
                    "skipping {} serialization probe: {} is not set",
                    config.provider_name, name
                );
                return None;
            };
            value
        },
        None => config.default_base_url.to_string(),
    };

    let model = optional_var(config.model_env).unwrap_or_else(|| config.default_model.to_string());

    Some(OpenAiProvider::new_with_name(
        Secret::new(api_key),
        model,
        base_url,
        config.provider_name.to_string(),
    ))
}

fn assert_provider_result(
    provider_name: &str,
    scenario: &Scenario,
    actual_arguments: &serde_json::Value,
) {
    let expectation = scenario.providers.get(provider_name).unwrap_or_else(|| {
        panic!(
            "scenario {} missing provider {}",
            scenario.id, provider_name
        )
    });

    match expectation {
        ProviderExpectation::MustPass => {
            assert_eq!(
                actual_arguments, &scenario.expected_arguments,
                "provider {provider_name} failed scenario {}",
                scenario.id
            );
        },
        ProviderExpectation::AllowKnownMismatch {
            observed_arguments,
            reason,
        } => {
            if actual_arguments == &scenario.expected_arguments {
                eprintln!(
                    "provider {provider_name} scenario {} now passes; previous known mismatch resolved",
                    scenario.id
                );
                return;
            }

            if actual_arguments == observed_arguments {
                eprintln!(
                    "provider {provider_name} scenario {} matched known mismatch: {}",
                    scenario.id, reason
                );
                return;
            }

            panic!(
                "provider {provider_name} scenario {} had an unexpected result.\nexpected: {:?}\nknown mismatch: {:?}\nactual: {:?}",
                scenario.id, scenario.expected_arguments, observed_arguments, actual_arguments
            );
        },
    }
}

async fn run_provider_scenarios(provider_name: &str, provider: OpenAiProvider) {
    let suite = load_suite();
    assert!(
        !suite.scenarios.is_empty(),
        "serialization scenario suite must not be empty"
    );

    for scenario in &suite.scenarios {
        if !scenario.providers.contains_key(provider_name) {
            continue;
        }

        let response = provider
            .complete(
                &[
                    ChatMessage::system(
                        "Call the provided tool exactly once. Do not answer with prose.",
                    ),
                    ChatMessage::user(scenario.prompt.clone()),
                ],
                std::slice::from_ref(&scenario.tool),
            )
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "provider {provider_name} request failed for scenario {}: {error}",
                    scenario.id
                )
            });

        assert_eq!(
            response.tool_calls.len(),
            1,
            "provider {provider_name} scenario {} expected exactly one tool call, got text {:?}",
            scenario.id,
            response.text
        );

        let tool_call = &response.tool_calls[0];
        let expected_name = scenario
            .tool
            .get("name")
            .and_then(serde_json::Value::as_str)
            .expect("tool name must be present");
        assert_eq!(
            tool_call.name, expected_name,
            "provider {provider_name} scenario {} called wrong tool",
            scenario.id
        );

        eprintln!(
            "provider {provider_name} scenario {} arguments: {:?}",
            scenario.id, tool_call.arguments
        );
        assert_provider_result(provider_name, scenario, &tool_call.arguments);
    }
}

#[tokio::test]
#[ignore]
async fn zai_serialization_scenarios_non_streaming() {
    let config = ProviderConfig {
        provider_name: "zai",
        api_key_env: "Z_API_KEY",
        base_url_env: None,
        default_base_url: "https://api.z.ai/api/paas/v4",
        model_env: "SERIALIZATION_TEST_ZAI_MODEL",
        default_model: "glm-5",
    };
    let Some(provider) = configured_provider(&config) else {
        return;
    };

    run_provider_scenarios(config.provider_name, provider).await;
}

#[tokio::test]
#[ignore]
async fn alibaba_coding_serialization_scenarios_non_streaming() {
    let config = ProviderConfig {
        provider_name: "alibaba-coding",
        api_key_env: "ALIBABA_CODING_API_KEY",
        base_url_env: Some("ALIBABA_CODING_BASE_URL"),
        default_base_url: "",
        model_env: "SERIALIZATION_TEST_ALIBABA_MODEL",
        default_model: "qwen3.5-plus",
    };
    let Some(provider) = configured_provider(&config) else {
        return;
    };

    run_provider_scenarios(config.provider_name, provider).await;
}

#[tokio::test]
#[ignore]
async fn openrouter_google_serialization_scenarios_non_streaming() {
    let config = ProviderConfig {
        provider_name: "openrouter-google",
        api_key_env: "OPENROUTER_API_KEY",
        base_url_env: None,
        default_base_url: "https://openrouter.ai/api/v1",
        model_env: "SERIALIZATION_TEST_OPENROUTER_GOOGLE_MODEL",
        default_model: "google/gemma-4-31b-it:free",
    };
    let Some(provider) = configured_provider(&config) else {
        return;
    };

    run_provider_scenarios(config.provider_name, provider).await;
}
