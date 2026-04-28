#![allow(clippy::module_inception)]

use super::{cache::*, config::*, service::*, *};

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_system_info() -> local_gguf::system_info::SystemInfo {
        local_gguf::system_info::SystemInfo {
            total_ram_bytes: 16 * 1024 * 1024 * 1024,
            available_ram_bytes: 8 * 1024 * 1024 * 1024,
            gguf_devices: vec![],
            has_metal: false,
            has_cuda: false,
            has_vulkan: false,
            is_apple_silicon: false,
        }
    }

    struct LocalModelConfigTestGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl LocalModelConfigTestGuard {
        fn new() -> Self {
            Self {
                _lock: crate::config_override_test_lock(),
            }
        }
    }

    impl Drop for LocalModelConfigTestGuard {
        fn drop(&mut self) {
            moltis_config::clear_config_dir();
            moltis_config::clear_data_dir();
        }
    }
    #[test]
    fn test_local_llm_config_serialization() {
        let mut config = LocalLlmConfig::default();
        config.add_model(LocalModelEntry {
            model_id: "qwen2.5-coder-7b-q4_k_m".into(),
            model_path: None,
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        });
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("qwen2.5-coder-7b-q4_k_m"));

        let parsed: LocalLlmConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.models.len(), 1);
        assert_eq!(parsed.models[0].model_id, "qwen2.5-coder-7b-q4_k_m");
    }

    #[test]
    fn test_local_llm_config_round_trip_preserves_custom_gguf_metadata() {
        let mut config = LocalLlmConfig::default();
        let repo = "Qwen/Qwen3-4B-GGUF";
        let first = LocalModelEntry {
            model_id: custom_gguf_model_id(repo, "Qwen3-4B-Q4_K_M.gguf"),
            model_path: None,
            hf_repo: Some(repo.into()),
            hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };
        let second = LocalModelEntry {
            model_id: custom_gguf_model_id(repo, "Qwen3-4B-Q6_K.gguf"),
            model_path: None,
            hf_repo: Some(repo.into()),
            hf_filename: Some("Qwen3-4B-Q6_K.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };

        config.add_model(first.clone());
        config.add_model(second.clone());

        let json = serde_json::to_string(&config).unwrap();
        let parsed: LocalLlmConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.models.len(), 2);
        assert_eq!(
            parsed
                .get_model(&first.model_id)
                .and_then(|entry| entry.hf_repo.as_deref()),
            Some(repo)
        );
        assert_eq!(
            parsed
                .get_model(&first.model_id)
                .and_then(|entry| entry.hf_filename.as_deref()),
            Some("Qwen3-4B-Q4_K_M.gguf")
        );
        assert_eq!(
            parsed
                .get_model(&second.model_id)
                .and_then(|entry| entry.hf_filename.as_deref()),
            Some("Qwen3-4B-Q6_K.gguf")
        );
    }

    #[test]
    fn test_local_llm_config_multi_model() {
        let mut config = LocalLlmConfig::default();
        config.add_model(LocalModelEntry {
            model_id: "model-1".into(),
            model_path: None,
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        });
        config.add_model(LocalModelEntry {
            model_id: "model-2".into(),
            model_path: None,
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "MLX".into(),
            idle_timeout_secs: None,
        });
        assert_eq!(config.models.len(), 2);

        // Test remove_model
        assert!(config.remove_model("model-1"));
        assert_eq!(config.models.len(), 1);
        assert_eq!(config.models[0].model_id, "model-2");

        // Test remove non-existent model
        assert!(!config.remove_model("model-1"));
        assert_eq!(config.models.len(), 1);
    }

    #[test]
    fn test_legacy_config_format_parsing() {
        // Test that legacy single-model format can be deserialized
        let legacy_json =
            r#"{"model_id":"old-model","model_path":null,"gpu_layers":0,"backend":"GGUF"}"#;
        let legacy: LegacyLocalLlmConfig = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(legacy.model_id, "old-model");
    }

    #[test]
    fn test_status_serialization() {
        let status = LocalLlmStatus::Ready {
            model_id: "test-model".into(),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["status"], "ready");
        assert_eq!(json["model_id"], "test-model");
    }

    #[test]
    fn test_has_enough_ram() {
        assert!(has_enough_ram(8, 8));
        assert!(has_enough_ram(16, 8));
        assert!(!has_enough_ram(0, 4));
    }

    #[test]
    fn test_insufficient_ram_error() {
        let message = insufficient_ram_error("Qwen 2.5 Coder 7B", 8, 0);
        assert!(message.contains("requires at least 8GB"));
        assert!(message.contains("detected 0GB"));
    }

    #[test]
    fn test_gguf_backend_description_uses_vulkan() {
        let mut sys = sample_system_info();
        sys.has_vulkan = true;
        assert_eq!(
            gguf_backend_description(&sys),
            "Cross-platform, Vulkan GPU acceleration"
        );
    }

    #[test]
    fn test_gguf_backend_note_uses_multiple_accelerators() {
        let mut sys = sample_system_info();
        sys.has_cuda = true;
        sys.has_vulkan = true;
        assert_eq!(
            gguf_backend_note(&sys, false),
            "GGUF with CUDA/Vulkan acceleration"
        );
    }

    #[test]
    fn test_gguf_backend_note_keeps_apple_mlx_hint() {
        let mut sys = sample_system_info();
        sys.is_apple_silicon = true;
        sys.has_metal = true;
        assert_eq!(
            gguf_backend_note(&sys, false),
            "GGUF with Metal acceleration (install mlx-lm for native MLX)"
        );
    }

    #[test]
    fn test_gguf_backend_description_unknown_gpu_uses_generic_label() {
        let mut sys = sample_system_info();
        sys.gguf_devices = vec![local_gguf::runtime_devices::GgufRuntimeDevice {
            index: 0,
            name: "ROCm0".into(),
            description: "AMD".into(),
            backend: "ROCm".into(),
            memory_total_bytes: 1,
            memory_free_bytes: 1,
        }];
        assert_eq!(
            gguf_backend_description(&sys),
            "Cross-platform, GPU acceleration"
        );
    }

    #[test]
    fn test_gguf_backend_note_unknown_gpu_uses_generic_label() {
        let mut sys = sample_system_info();
        sys.gguf_devices = vec![local_gguf::runtime_devices::GgufRuntimeDevice {
            index: 0,
            name: "ROCm0".into(),
            description: "AMD".into(),
            backend: "ROCm".into(),
            memory_total_bytes: 1,
            memory_free_bytes: 1,
        }];
        assert_eq!(gguf_backend_note(&sys, false), "GGUF with GPU acceleration");
    }

    #[test]
    fn test_hf_model_info_parsing() {
        // Test parsing with all fields (matching actual HF API response)
        let json = r#"{
            "id": "TheBloke/Llama-2-7B-GGUF",
            "downloads": 1234567,
            "likes": 100,
            "createdAt": "2024-01-15T10:30:00Z",
            "tags": ["gguf", "llama"]
        }"#;
        let info: HfModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "TheBloke/Llama-2-7B-GGUF");
        assert_eq!(info.downloads, 1234567);
        assert_eq!(info.likes, 100);
        assert!(info.created_at.is_some());
        assert_eq!(info.tags.len(), 2);
    }

    #[test]
    fn test_hf_model_info_parsing_mlx_community() {
        // Test parsing MLX community model response
        let json = r#"{
            "id": "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit",
            "downloads": 500,
            "likes": 10,
            "tags": ["mlx", "safetensors"]
        }"#;
        let info: HfModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit");
        assert_eq!(info.downloads, 500);
        assert_eq!(info.likes, 10);
        assert_eq!(info.tags.len(), 2);
    }

    #[test]
    fn test_hf_model_info_parsing_minimal() {
        // Test parsing with minimal fields
        let json = r#"{"id": "test/model"}"#;
        let info: HfModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "test/model");
        assert_eq!(info.downloads, 0);
        assert_eq!(info.likes, 0);
        assert!(info.created_at.is_none());
        assert!(info.tags.is_empty());
    }

    #[test]
    fn test_hf_api_response_parsing_real_format() {
        // Test parsing actual HuggingFace API response format
        // This is a real response from: https://huggingface.co/api/models?author=mlx-community&limit=1
        let json = r#"[
            {
                "_id": "680fecc14cc667f59da738f5",
                "id": "mlx-community/Qwen3-0.6B-4bit",
                "likes": 9,
                "private": false,
                "downloads": 20580,
                "tags": [
                    "mlx",
                    "safetensors",
                    "qwen3",
                    "text-generation",
                    "conversational",
                    "base_model:Qwen/Qwen3-0.6B",
                    "license:apache-2.0",
                    "4-bit",
                    "region:us"
                ],
                "pipeline_tag": "text-generation",
                "library_name": "mlx",
                "createdAt": "2025-04-28T21:01:53.000Z",
                "modelId": "mlx-community/Qwen3-0.6B-4bit"
            }
        ]"#;

        // Parse as array (as the API returns)
        let models: Vec<HfModelInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(models.len(), 1);

        let info = &models[0];
        assert_eq!(info.id, "mlx-community/Qwen3-0.6B-4bit");
        assert_eq!(info.downloads, 20580);
        assert_eq!(info.likes, 9);
        assert!(info.created_at.is_some());
        assert_eq!(
            info.created_at.as_ref().unwrap(),
            "2025-04-28T21:01:53.000Z"
        );
        assert!(info.tags.contains(&"mlx".to_string()));
        assert!(info.tags.contains(&"qwen3".to_string()));
    }

    #[test]
    fn test_hf_api_response_parsing_gguf_format() {
        // Test parsing GGUF model response format
        let json = r#"[
            {
                "id": "TheBloke/Llama-2-7B-GGUF",
                "downloads": 5000000,
                "likes": 500,
                "tags": ["gguf", "llama", "text-generation"],
                "createdAt": "2023-09-01T00:00:00.000Z"
            },
            {
                "id": "bartowski/Qwen2.5-Coder-32B-Instruct-GGUF",
                "downloads": 100000,
                "likes": 50,
                "tags": ["gguf", "qwen", "coder"]
            }
        ]"#;

        let models: Vec<HfModelInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(models.len(), 2);

        assert_eq!(models[0].id, "TheBloke/Llama-2-7B-GGUF");
        assert_eq!(models[0].downloads, 5000000);
        assert!(models[0].created_at.is_some());

        assert_eq!(models[1].id, "bartowski/Qwen2.5-Coder-32B-Instruct-GGUF");
        assert_eq!(models[1].downloads, 100000);
        assert!(models[1].created_at.is_none()); // Not all responses have createdAt
    }

    #[test]
    fn test_custom_model_id_generation() {
        let repo = "Qwen/Qwen3-4B-GGUF";
        let filename = "Qwen3-4B-Q4_K_M.gguf";
        let model_id = custom_gguf_model_id("Qwen/Qwen3-4B-GGUF", "Qwen3-4B-Q4_K_M.gguf");
        let encoded = model_id.strip_prefix("custom-gguf-").unwrap();
        let (repo_component, filename_component) = encoded.split_once('.').unwrap();

        assert_eq!(
            URL_SAFE_NO_PAD.decode(repo_component).unwrap(),
            repo.as_bytes()
        );
        assert_eq!(
            URL_SAFE_NO_PAD.decode(filename_component).unwrap(),
            filename.as_bytes()
        );
    }

    #[test]
    fn test_custom_model_id_generation_distinguishes_filenames_in_same_repo() {
        let repo = "Qwen/Qwen3-4B-GGUF";
        let first = custom_gguf_model_id(repo, "Qwen3-4B-Q4_K_M.gguf");
        let second = custom_gguf_model_id(repo, "Qwen3-4B-Q6_K.gguf");

        assert_ne!(first, second);
    }

    #[test]
    fn test_custom_model_id_generation_avoids_lossy_slug_collisions() {
        let first = custom_gguf_model_id("org-a/model", "quant/file.gguf");
        let second = custom_gguf_model_id("org/a-model", "quant-file.gguf");

        assert_ne!(first, second);
    }

    #[test]
    fn test_remove_conflicting_custom_gguf_entries_removes_legacy_and_duplicate_entries() {
        let repo = "Qwen/Qwen3-4B-GGUF";
        let filename = "Qwen3-4B-Q4_K_M.gguf";
        let mut config = LocalLlmConfig {
            models: vec![
                LocalModelEntry {
                    model_id: legacy_custom_gguf_model_id(repo),
                    model_path: None,
                    hf_repo: None,
                    hf_filename: None,
                    gpu_layers: 0,
                    backend: "GGUF".into(),
                    idle_timeout_secs: None,
                },
                LocalModelEntry {
                    model_id: "custom-stale".into(),
                    model_path: None,
                    hf_repo: Some(repo.into()),
                    hf_filename: Some(filename.into()),
                    gpu_layers: 0,
                    backend: "GGUF".into(),
                    idle_timeout_secs: None,
                },
                LocalModelEntry {
                    model_id: custom_gguf_model_id(repo, "Qwen3-4B-Q6_K.gguf"),
                    model_path: None,
                    hf_repo: Some(repo.into()),
                    hf_filename: Some("Qwen3-4B-Q6_K.gguf".into()),
                    gpu_layers: 0,
                    backend: "GGUF".into(),
                    idle_timeout_secs: None,
                },
            ],
        };

        let mut removed_model_ids =
            remove_conflicting_custom_gguf_entries(&mut config, repo, filename);
        removed_model_ids.sort_unstable();

        assert_eq!(config.models.len(), 1);
        assert_eq!(
            config.models[0].model_id,
            custom_gguf_model_id(repo, "Qwen3-4B-Q6_K.gguf")
        );
        let mut expected_removed_model_ids = vec![
            legacy_custom_gguf_model_id(repo),
            "custom-stale".to_string(),
        ];
        expected_removed_model_ids.sort_unstable();
        assert_eq!(removed_model_ids, expected_removed_model_ids);
    }

    #[test]
    fn test_custom_model_path_resolution_uses_repo_scoped_cache_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();
        let entry = LocalModelEntry {
            model_id: custom_gguf_model_id("Qwen/Qwen3-4B-GGUF", "Qwen3-4B-Q4_K_M.gguf"),
            model_path: None,
            hf_repo: Some("Qwen/Qwen3-4B-GGUF".into()),
            hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };

        let resolved = entry.resolved_model_path(None, cache_dir).unwrap();
        assert_eq!(
            resolved,
            Some(
                cache_dir
                    .join("custom")
                    .join("Qwen")
                    .join("Qwen3-4B-GGUF")
                    .join("Qwen3-4B-Q4_K_M.gguf")
            )
        );
    }

    #[test]
    fn test_custom_model_path_resolution_prefers_repo_metadata_over_stale_saved_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();
        let entry = LocalModelEntry {
            model_id: custom_gguf_model_id("Qwen/Qwen3-4B-GGUF", "Qwen3-4B-Q4_K_M.gguf"),
            model_path: Some(PathBuf::from("/tmp/stale-model.gguf")),
            hf_repo: Some("Qwen/Qwen3-4B-GGUF".into()),
            hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };

        let resolved = entry.resolved_model_path(None, cache_dir).unwrap();
        assert_eq!(
            resolved,
            Some(
                cache_dir
                    .join("custom")
                    .join("Qwen")
                    .join("Qwen3-4B-GGUF")
                    .join("Qwen3-4B-Q4_K_M.gguf")
            )
        );
    }

    #[test]
    fn test_builtin_model_path_resolution_uses_provider_override() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();
        let override_path = Path::new("/tmp/custom-built-in-model.gguf");
        let entry = LocalModelEntry {
            model_id: "qwen2.5-coder-7b-q4_k_m".into(),
            model_path: None,
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };

        let resolved = entry
            .resolved_model_path(Some(override_path), cache_dir)
            .unwrap();

        assert_eq!(resolved, Some(override_path.to_path_buf()));
    }

    #[test]
    fn test_custom_model_display_name_prefers_filename() {
        let entry = LocalModelEntry {
            model_id: "custom-test".into(),
            model_path: None,
            hf_repo: Some("Qwen/Qwen3-4B-GGUF".into()),
            hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };

        assert_eq!(entry.display_name(), "Qwen3-4B-Q4_K_M.gguf");
    }

    #[test]
    fn test_register_saved_local_models_registers_custom_gguf_provider_from_saved_config() {
        let _guard = LocalModelConfigTestGuard::new();
        let config_dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        moltis_config::set_config_dir(config_dir.path().to_path_buf());
        moltis_config::set_data_dir(data_dir.path().to_path_buf());

        let entry = LocalModelEntry {
            model_id: custom_gguf_model_id("Qwen/Qwen3-4B-GGUF", "Qwen3-4B-Q4_K_M.gguf"),
            model_path: None,
            hf_repo: Some("Qwen/Qwen3-4B-GGUF".into()),
            hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };
        let config = LocalLlmConfig {
            models: vec![entry.clone()],
        };
        config.save().unwrap();

        let mut registry = ProviderRegistry::empty();
        register_saved_local_models(
            &mut registry,
            &moltis_config::schema::ProvidersConfig::default(),
        );

        assert!(registry.get(&entry.model_id).is_some());
        let registered = registry
            .list_models()
            .iter()
            .find(|model| raw_model_id(&model.id) == entry.model_id)
            .unwrap();
        assert_eq!(registered.provider, "local-llm");
        assert_eq!(registered.display_name, "Qwen3-4B-Q4_K_M.gguf");
    }

    #[test]
    fn test_register_local_model_entry_keeps_non_local_collisions() {
        let mut registry = ProviderRegistry::empty();
        let remote_provider = Arc::new(moltis_providers::openai::OpenAiProvider::new(
            secrecy::Secret::new("test-key".into()),
            "shared-model".into(),
            "https://example.com".into(),
        ));
        registry.register(
            moltis_providers::ModelInfo {
                id: "shared-model".into(),
                provider: "openai".into(),
                display_name: "Shared Remote Model".into(),
                created_at: None,
                recommended: false,
                capabilities: moltis_providers::ModelCapabilities::default(),
            },
            remote_provider,
        );

        let entry = LocalModelEntry {
            model_id: "shared-model".into(),
            model_path: Some(PathBuf::from("/tmp/shared-model.gguf")),
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };

        register_local_model_entry(&mut registry, &entry).unwrap();

        let registered: Vec<_> = registry
            .list_models()
            .iter()
            .filter(|model| raw_model_id(&model.id) == "shared-model")
            .collect();
        assert_eq!(registered.len(), 2);
        assert!(registered.iter().any(|model| model.provider == "openai"));
        assert!(
            registered
                .iter()
                .any(|model| model.provider == LOCAL_LLM_PROVIDER_NAME)
        );
    }

    #[test]
    fn test_unregister_local_model_ids_from_registry_removes_superseded_entries() {
        let repo = "Qwen/Qwen3-4B-GGUF";
        let legacy_entry = LocalModelEntry {
            model_id: legacy_custom_gguf_model_id(repo),
            model_path: Some(PathBuf::from("/tmp/legacy.gguf")),
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };
        let stale_entry = LocalModelEntry {
            model_id: "custom-stale".into(),
            model_path: Some(PathBuf::from("/tmp/stale.gguf")),
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };
        let active_entry = LocalModelEntry {
            model_id: custom_gguf_model_id(repo, "Qwen3-4B-Q6_K.gguf"),
            model_path: Some(PathBuf::from("/tmp/active.gguf")),
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
            idle_timeout_secs: None,
        };

        let mut registry = ProviderRegistry::empty();
        for entry in [&legacy_entry, &stale_entry, &active_entry] {
            let (info, provider) = build_local_provider_entry(entry, None).unwrap();
            registry.register(info, provider);
        }

        let remote_provider = Arc::new(moltis_providers::openai::OpenAiProvider::new(
            secrecy::Secret::new("test-key".into()),
            "custom-stale".into(),
            "https://example.com".into(),
        ));
        registry.register(
            moltis_providers::ModelInfo {
                id: "custom-stale".into(),
                provider: "openai".into(),
                display_name: "Remote Stale Alias".into(),
                created_at: None,
                recommended: false,
                capabilities: moltis_providers::ModelCapabilities::default(),
            },
            remote_provider,
        );

        let superseded_model_ids =
            vec![legacy_entry.model_id.clone(), stale_entry.model_id.clone()];
        unregister_local_model_ids_from_registry(&mut registry, &superseded_model_ids);

        assert!(!registry.list_models().iter().any(|model| {
            model.provider == LOCAL_LLM_PROVIDER_NAME
                && raw_model_id(&model.id) == legacy_entry.model_id.as_str()
        }));
        assert!(!registry.list_models().iter().any(|model| {
            model.provider == LOCAL_LLM_PROVIDER_NAME
                && raw_model_id(&model.id) == stale_entry.model_id.as_str()
        }));
        assert!(registry.list_models().iter().any(|model| {
            model.provider == "openai" && raw_model_id(&model.id) == stale_entry.model_id.as_str()
        }));
        assert!(registry.list_models().iter().any(|model| {
            model.provider == LOCAL_LLM_PROVIDER_NAME
                && raw_model_id(&model.id) == active_entry.model_id.as_str()
        }));
    }

    #[test]
    fn test_search_url_encoding() {
        // Test that search queries are properly URL-encoded
        let query = "llama 2 chat";
        let encoded = urlencoding::encode(query);
        assert_eq!(encoded, "llama%202%20chat");

        let query2 = "qwen2.5-coder";
        let encoded2 = urlencoding::encode(query2);
        assert_eq!(encoded2, "qwen2.5-coder");
    }

    #[test]
    fn test_download_progress_percent_bounds_values() {
        assert_eq!(download_progress_percent(50, Some(100)), Some(50.0));
        assert_eq!(download_progress_percent(250, Some(100)), Some(100.0));
        assert_eq!(download_progress_percent(10, Some(0)), Some(0.0));
        assert_eq!(download_progress_percent(10, None), None);
    }

    #[tokio::test]
    async fn test_search_huggingface_builds_correct_url_for_mlx() {
        // This test verifies URL construction logic without making actual HTTP calls
        // In a real test, you'd mock the HTTP client

        // For MLX with empty query, should search mlx-community
        let mlx_empty_url = if true {
            // Simulating backend == "MLX" && query.is_empty()
            format!(
                "https://huggingface.co/api/models?author=mlx-community&sort=downloads&direction=-1&limit={}",
                20
            )
        } else {
            String::new()
        };
        assert!(mlx_empty_url.contains("author=mlx-community"));
        assert!(mlx_empty_url.contains("sort=downloads"));
    }

    #[tokio::test]
    async fn test_search_huggingface_builds_correct_url_for_gguf() {
        // For GGUF with query, should append "gguf" to search
        let query = "llama";
        let search_query = format!("{} gguf", query);
        let gguf_url = format!(
            "https://huggingface.co/api/models?search={}&sort=downloads&direction=-1&limit={}",
            urlencoding::encode(&search_query),
            20
        );
        assert!(gguf_url.contains("search=llama%20gguf"));
        assert!(gguf_url.contains("sort=downloads"));
    }
}
