use std::sync::Arc;

use {super::common::LocalModelConfigTestGuard, secrecy::Secret};

#[cfg(feature = "local-llm")]
#[test]
fn restore_saved_local_llm_models_rehydrates_custom_models_after_registry_rebuild() {
    let _guard = LocalModelConfigTestGuard::new();
    let config_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    moltis_config::set_config_dir(config_dir.path().to_path_buf());
    moltis_config::set_data_dir(data_dir.path().to_path_buf());

    let saved_entry = crate::local_llm_setup::LocalModelEntry {
        model_id: "custom-qwen".into(),
        model_path: Some(std::path::PathBuf::from("/tmp/custom-qwen.gguf")),
        hf_repo: Some("Qwen/Qwen3-4B-GGUF".into()),
        hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
        gpu_layers: 0,
        backend: "GGUF".into(),
    };
    crate::local_llm_setup::LocalLlmConfig {
        models: vec![saved_entry.clone()],
    }
    .save()
    .unwrap();

    let mut rebuilt_registry = moltis_providers::ProviderRegistry::empty();
    let remote_provider = Arc::new(moltis_providers::openai::OpenAiProvider::new(
        Secret::new("test-key".into()),
        "remote-model".into(),
        "https://example.com".into(),
    ));
    rebuilt_registry.register(
        moltis_providers::ModelInfo {
            id: "remote-model".into(),
            provider: "openai".into(),
            display_name: "Remote Model".into(),
            created_at: None,
            recommended: false,
            capabilities: moltis_providers::ModelCapabilities::default(),
        },
        remote_provider,
    );

    crate::server::helpers::restore_saved_local_llm_models(
        &mut rebuilt_registry,
        &moltis_config::schema::ProvidersConfig::default(),
    );

    assert!(
        rebuilt_registry
            .list_models()
            .iter()
            .any(|model| model.provider == "openai")
    );
    assert!(
        rebuilt_registry.list_models().iter().any(
            |model| moltis_providers::model_id::raw_model_id(&model.id) == saved_entry.model_id
        )
    );
}

#[cfg(feature = "local-llm")]
#[test]
fn restore_saved_local_llm_models_skips_when_local_provider_is_disabled() {
    let _guard = LocalModelConfigTestGuard::new();
    let config_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    moltis_config::set_config_dir(config_dir.path().to_path_buf());
    moltis_config::set_data_dir(data_dir.path().to_path_buf());

    let saved_entry = crate::local_llm_setup::LocalModelEntry {
        model_id: "custom-qwen".into(),
        model_path: Some(std::path::PathBuf::from("/tmp/custom-qwen.gguf")),
        hf_repo: Some("Qwen/Qwen3-4B-GGUF".into()),
        hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
        gpu_layers: 0,
        backend: "GGUF".into(),
    };
    crate::local_llm_setup::LocalLlmConfig {
        models: vec![saved_entry.clone()],
    }
    .save()
    .unwrap();

    let mut providers_config = moltis_config::schema::ProvidersConfig::default();
    providers_config
        .providers
        .insert("local-llm".into(), moltis_config::schema::ProviderEntry {
            enabled: false,
            ..Default::default()
        });

    let mut rebuilt_registry = moltis_providers::ProviderRegistry::empty();
    crate::server::helpers::restore_saved_local_llm_models(
        &mut rebuilt_registry,
        &providers_config,
    );

    assert!(
        !rebuilt_registry.list_models().iter().any(
            |model| moltis_providers::model_id::raw_model_id(&model.id) == saved_entry.model_id
        )
    );
}
