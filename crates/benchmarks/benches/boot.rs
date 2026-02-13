#![allow(clippy::unwrap_used, clippy::expect_used)]
fn main() {
    divan::main();
}

// ── Config parsing ──────────────────────────────────────────────────────────

/// Benchmark generating the default TOML config template.
#[divan::bench]
fn config_template_generation() -> String {
    divan::black_box(moltis_config::template::default_config_template(8080))
}

/// Benchmark constructing a `MoltisConfig` with all defaults.
#[divan::bench]
fn config_default_construction() -> moltis_config::MoltisConfig {
    divan::black_box(moltis_config::MoltisConfig::default())
}

/// Benchmark loading + parsing a TOML config from disk (the full boot path).
#[divan::bench]
fn config_load_toml(bencher: divan::Bencher) {
    let toml_content = moltis_config::template::default_config_template(8080);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("moltis.toml");
    std::fs::write(&path, &toml_content).unwrap();

    bencher.bench_local(|| divan::black_box(moltis_config::loader::load_config(&path).unwrap()));
}

/// Benchmark config round-trip: serialize MoltisConfig to TOML, then deserialize.
#[divan::bench]
fn config_serde_roundtrip() {
    let config = moltis_config::MoltisConfig::default();
    let toml_str = divan::black_box(toml::to_string_pretty(&config).unwrap());
    let _: moltis_config::MoltisConfig = divan::black_box(toml::from_str(&toml_str).unwrap());
}

/// Benchmark validating a TOML config string (schema checks, semantic warnings).
#[divan::bench]
fn config_validate_toml(bencher: divan::Bencher) {
    let toml_content = moltis_config::template::default_config_template(8080);

    bencher.bench_local(|| {
        divan::black_box(moltis_config::validate::validate_toml_str(&toml_content))
    });
}

// ── Provider model lookups ──────────────────────────────────────────────────

const MODEL_IDS: &[&str] = &[
    "claude-sonnet-4-5-20250929",
    "gpt-4o",
    "gpt-5",
    "gemini-2.0-flash",
    "codestral-latest",
    "mistral-large-latest",
    "o3",
    "kimi-k2.5",
    "unknown-model-xyz",
];

#[divan::bench(args = MODEL_IDS)]
fn context_window_lookup(model_id: &str) -> u32 {
    divan::black_box(moltis_agents::providers::context_window_for_model(model_id))
}

#[divan::bench(args = MODEL_IDS)]
fn vision_support_lookup(model_id: &str) -> bool {
    divan::black_box(moltis_agents::providers::supports_vision_for_model(
        model_id,
    ))
}

#[divan::bench]
fn namespaced_model_id() -> String {
    divan::black_box(moltis_agents::providers::namespaced_model_id(
        "openai", "gpt-4o",
    ))
}

// ── Session store ───────────────────────────────────────────────────────────

const SESSION_KEYS: &[&str] = &[
    "default",
    "project:backend:debug-auth",
    "2026-02-09T12:00:00Z",
    "user@host:session:42",
];

#[divan::bench(args = SESSION_KEYS)]
fn session_key_to_filename(key: &str) -> String {
    divan::black_box(moltis_sessions::store::SessionStore::key_to_filename(key))
}

fn build_sanitize_input(payload_bytes: usize) -> String {
    let image_blob = "A".repeat(payload_bytes);
    let hex_blob = "deadbeef".repeat(payload_bytes / 8);
    format!("before data:image/png;base64,{image_blob} middle {hex_blob} after")
}

#[divan::bench(args = [10_000, 100_000, 1_000_000])]
fn sanitize_tool_result(bencher: divan::Bencher, payload_bytes: usize) {
    let input = build_sanitize_input(payload_bytes);
    bencher.bench_local(|| {
        divan::black_box(moltis_agents::runner::sanitize_tool_result(&input, 50_000))
    });
}

#[divan::bench(args = [10_000, 100_000, 1_000_000])]
fn tool_result_to_content_vision(bencher: divan::Bencher, payload_bytes: usize) {
    let input = build_sanitize_input(payload_bytes);
    bencher.bench_local(|| {
        divan::black_box(moltis_agents::runner::tool_result_to_content(
            &input, 50_000, true,
        ))
    });
}

fn build_persisted_messages(n: usize) -> Vec<serde_json::Value> {
    let mut values = Vec::with_capacity(n + 1);
    values.push(serde_json::json!({
        "role": "system",
        "content": "You are a helpful assistant."
    }));

    for i in 0..n {
        match i % 6 {
            0 => values.push(serde_json::json!({
                "role": "user",
                "content": format!("How do I fix issue #{i}?"),
            })),
            1 => values.push(serde_json::json!({
                "role": "assistant",
                "content": format!("Try step {}", i % 5),
            })),
            2 => values.push(serde_json::json!({
                "role": "assistant",
                "content": format!("Calling tool {i}"),
                "tool_calls": [{
                    "id": format!("tool_{i}"),
                    "type": "function",
                    "function": {
                        "name": "web.search",
                        "arguments": r#"{"q":"moltis release notes"}"#,
                    }
                }],
            })),
            3 => values.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": format!("tool_{i}"),
                "content": {"ok": true, "items": i},
            })),
            4 => values.push(serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "Please inspect this screenshot"},
                    {
                        "type": "image_url",
                        "image_url": {"url": "data:image/png;base64,AAAA"},
                    }
                ],
            })),
            _ => values.push(serde_json::json!({
                "role": "tool_result",
                "tool_call_id": format!("tool_{i}"),
                "content": {"success": true},
            })),
        }
    }

    values
}

#[divan::bench(args = [50, 500, 2000])]
fn values_to_chat_messages(bencher: divan::Bencher, n: usize) {
    let values = build_persisted_messages(n);
    bencher
        .bench_local(|| divan::black_box(moltis_agents::model::values_to_chat_messages(&values)));
}

// ── Env substitution ────────────────────────────────────────────────────────

#[divan::bench]
fn env_substitution(bencher: divan::Bencher) {
    let input = r#"
        api_key = "${MOLTIS_API_KEY}"
        base_url = "${MOLTIS_BASE_URL:-https://api.example.com}"
        name = "no-vars-here"
        port = 8080
    "#;

    bencher.bench_local(|| divan::black_box(moltis_config::env_subst::substitute_env(input)));
}

// ── Config load from disk (simulated boot) ──────────────────────────────────

/// Full boot-path simulation: generate template, write to disk, load, validate.
#[divan::bench]
fn full_config_boot_path(bencher: divan::Bencher) {
    let toml_content = moltis_config::template::default_config_template(8080);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("moltis.toml");
    std::fs::write(&path, &toml_content).unwrap();

    bencher.bench_local(|| {
        let config = moltis_config::loader::load_config(&path).unwrap();
        let _ = moltis_config::validate::validate_toml_str(&toml_content);
        divan::black_box(config)
    });
}
