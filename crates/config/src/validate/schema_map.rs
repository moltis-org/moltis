use {super::*, std::collections::HashMap};

/// Represents the expected shape of the configuration schema.
pub(super) enum KnownKeys {
    /// A struct with fixed field names.
    Struct(HashMap<&'static str, KnownKeys>),
    /// A map with dynamic keys (providers, mcp.servers, etc.) whose values
    /// have a known shape.
    Map(Box<KnownKeys>),
    /// A map with dynamic keys plus explicit static keys.
    MapWithFields {
        value: Box<KnownKeys>,
        fields: HashMap<&'static str, KnownKeys>,
    },
    /// An array of typed items.
    Array(Box<KnownKeys>),
    /// Scalar value — stop recursion.
    Leaf,
}

/// Build the full schema map mirroring every field in `schema.rs`.
pub(super) fn build_schema_map() -> KnownKeys {
    use KnownKeys::{Array, Leaf, Map, MapWithFields, Struct};

    let tool_policy_entry = || {
        Struct(HashMap::from([
            ("allow", Leaf),
            ("deny", Leaf),
            ("profile", Leaf),
        ]))
    };

    let provider_entry = || {
        Struct(HashMap::from([
            ("enabled", Leaf),
            ("api_key", Leaf),
            ("base_url", Leaf),
            ("url", Leaf),
            ("models", Leaf),
            ("fetch_models", Leaf),
            ("stream_transport", Leaf),
            ("wire_api", Leaf),
            ("alias", Leaf),
            ("tool_mode", Leaf),
            ("cache_retention", Leaf),
            ("strict_tools", Leaf),
            ("policy", tool_policy_entry()),
        ]))
    };

    let resource_limits = || {
        Struct(HashMap::from([
            ("memory_limit", Leaf),
            ("cpu_quota", Leaf),
            ("pids_max", Leaf),
        ]))
    };

    let wasm_tool_limit_override = || Struct(HashMap::from([("fuel", Leaf), ("memory", Leaf)]));

    let wasm_tool_limits = || {
        Struct(HashMap::from([
            ("default_memory", Leaf),
            ("default_fuel", Leaf),
            ("tool_overrides", Map(Box::new(wasm_tool_limit_override()))),
        ]))
    };

    let sandbox = || {
        Struct(HashMap::from([
            ("mode", Leaf),
            ("scope", Leaf),
            ("workspace_mount", Leaf),
            ("host_data_dir", Leaf),
            ("home_persistence", Leaf),
            ("shared_home_dir", Leaf),
            ("image", Leaf),
            ("container_prefix", Leaf),
            ("no_network", Leaf),
            ("network", Leaf),
            ("trusted_domains", Array(Box::new(Leaf))),
            ("backend", Leaf),
            ("resource_limits", resource_limits()),
            ("packages", Leaf),
            ("wasm_fuel_limit", Leaf),
            ("wasm_epoch_interval_ms", Leaf),
            ("wasm_tool_limits", wasm_tool_limits()),
            ("tools_policy", tool_policy_entry()),
        ]))
    };

    let perplexity = || {
        Struct(HashMap::from([
            ("api_key", Leaf),
            ("base_url", Leaf),
            ("model", Leaf),
        ]))
    };

    let web_search = || {
        Struct(HashMap::from([
            ("enabled", Leaf),
            ("provider", Leaf),
            ("api_key", Leaf),
            ("max_results", Leaf),
            ("timeout_seconds", Leaf),
            ("cache_ttl_minutes", Leaf),
            ("duckduckgo_fallback", Leaf),
            ("perplexity", perplexity()),
        ]))
    };

    let web_fetch = || {
        Struct(HashMap::from([
            ("enabled", Leaf),
            ("max_chars", Leaf),
            ("timeout_seconds", Leaf),
            ("cache_ttl_minutes", Leaf),
            ("max_redirects", Leaf),
            ("readability", Leaf),
            ("ssrf_allowlist", Leaf),
        ]))
    };

    let firecrawl = || {
        Struct(HashMap::from([
            ("enabled", Leaf),
            ("api_key", Leaf),
            ("base_url", Leaf),
            ("only_main_content", Leaf),
            ("timeout_seconds", Leaf),
            ("cache_ttl_minutes", Leaf),
            ("web_fetch_fallback", Leaf),
        ]))
    };

    let exec = || {
        Struct(HashMap::from([
            ("default_timeout_secs", Leaf),
            ("max_output_bytes", Leaf),
            ("approval_mode", Leaf),
            ("security_level", Leaf),
            ("allowlist", Leaf),
            ("sandbox", sandbox()),
            ("host", Leaf),
            ("node", Leaf),
            ("ssh_target", Leaf),
        ]))
    };

    let browser = || {
        Struct(HashMap::from([
            ("enabled", Leaf),
            ("chrome_path", Leaf),
            ("headless", Leaf),
            ("viewport_width", Leaf),
            ("viewport_height", Leaf),
            ("device_scale_factor", Leaf),
            ("max_instances", Leaf),
            ("memory_limit_percent", Leaf),
            ("idle_timeout_secs", Leaf),
            ("navigation_timeout_ms", Leaf),
            ("user_agent", Leaf),
            ("chrome_args", Leaf),
            ("sandbox", Leaf),
            ("sandbox_image", Leaf),
            ("allowed_domains", Leaf),
            ("low_memory_threshold_mb", Leaf),
            ("persist_profile", Leaf),
            ("profile_dir", Leaf),
            ("container_host", Leaf),
            ("browserless_api_version", Leaf),
        ]))
    };

    let tools = || {
        Struct(HashMap::from([
            ("exec", exec()),
            ("browser", browser()),
            (
                "policy",
                Struct(HashMap::from([
                    ("allow", Leaf),
                    ("deny", Leaf),
                    ("profile", Leaf),
                ])),
            ),
            (
                "web",
                Struct(HashMap::from([
                    ("search", web_search()),
                    ("fetch", web_fetch()),
                    ("firecrawl", firecrawl()),
                ])),
            ),
            ("maps", Struct(HashMap::from([("provider", Leaf)]))),
            (
                "fs",
                Struct(HashMap::from([
                    ("workspace_root", Leaf),
                    ("allow_paths", Array(Box::new(Leaf))),
                    ("deny_paths", Array(Box::new(Leaf))),
                    ("track_reads", Leaf),
                    ("must_read_before_write", Leaf),
                    ("require_approval", Leaf),
                    ("max_read_bytes", Leaf),
                    ("binary_policy", Leaf),
                    ("respect_gitignore", Leaf),
                    ("checkpoint_before_mutation", Leaf),
                    ("context_window_tokens", Leaf),
                ])),
            ),
            ("agent_timeout_secs", Leaf),
            ("agent_max_iterations", Leaf),
            ("agent_max_auto_continues", Leaf),
            ("agent_auto_continue_min_tool_calls", Leaf),
            ("max_tool_result_bytes", Leaf),
            ("registry_mode", Leaf),
            ("agent_loop_detector_window", Leaf),
            ("agent_loop_detector_strip_tools_on_second_fire", Leaf),
        ]))
    };

    let mcp_oauth_override = || {
        Struct(HashMap::from([
            ("client_id", Leaf),
            ("auth_url", Leaf),
            ("token_url", Leaf),
            ("scopes", Leaf),
        ]))
    };

    let mcp_server_entry = || {
        Struct(HashMap::from([
            ("command", Leaf),
            ("args", Leaf),
            ("env", Map(Box::new(Leaf))),
            ("enabled", Leaf),
            ("request_timeout_secs", Leaf),
            ("transport", Leaf),
            ("url", Leaf),
            ("headers", Map(Box::new(Leaf))),
            ("oauth", mcp_oauth_override()),
        ]))
    };

    let shell_hook_entry = || {
        Struct(HashMap::from([
            ("name", Leaf),
            ("command", Leaf),
            ("events", Leaf),
            ("timeout", Leaf),
            ("env", Map(Box::new(Leaf))),
        ]))
    };

    let active_hours = || {
        Struct(HashMap::from([
            ("start", Leaf),
            ("end", Leaf),
            ("timezone", Leaf),
        ]))
    };

    let qmd_collection = || Struct(HashMap::from([("paths", Leaf), ("globs", Leaf)]));

    let qmd = || {
        Struct(HashMap::from([
            ("command", Leaf),
            ("collections", Map(Box::new(qmd_collection()))),
            ("max_results", Leaf),
            ("timeout_ms", Leaf),
        ]))
    };

    let agent_preset = || {
        Struct(HashMap::from([
            (
                "identity",
                Struct(HashMap::from([
                    ("name", Leaf),
                    ("emoji", Leaf),
                    ("theme", Leaf),
                ])),
            ),
            ("model", Leaf),
            (
                "tools",
                Struct(HashMap::from([("allow", Leaf), ("deny", Leaf)])),
            ),
            ("delegate_only", Leaf),
            ("system_prompt_suffix", Leaf),
            ("max_iterations", Leaf),
            ("timeout_secs", Leaf),
            (
                "sessions",
                Struct(HashMap::from([
                    ("key_prefix", Leaf),
                    ("allowed_keys", Leaf),
                    ("can_send", Leaf),
                    ("cross_agent", Leaf),
                ])),
            ),
            (
                "memory",
                Struct(HashMap::from([("scope", Leaf), ("max_lines", Leaf)])),
            ),
            ("reasoning_effort", Leaf),
        ]))
    };

    Struct(HashMap::from([
        (
            "server",
            Struct(HashMap::from([
                ("bind", Leaf),
                ("port", Leaf),
                ("http_request_logs", Leaf),
                ("ws_request_logs", Leaf),
                ("log_buffer_size", Leaf),
                ("update_releases_url", Leaf),
                ("db_pool_max_connections", Leaf),
                ("shiki_cdn_url", Leaf),
                ("terminal_enabled", Leaf),
            ])),
        ),
        ("providers", MapWithFields {
            value: Box::new(provider_entry()),
            fields: HashMap::from([
                ("offered", Array(Box::new(Leaf))),
                ("show_legacy_models", Leaf),
            ]),
        }),
        (
            "chat",
            Struct(HashMap::from([
                ("message_queue_mode", Leaf),
                ("prompt_memory_mode", Leaf),
                ("workspace_file_max_chars", Leaf),
                ("priority_models", Leaf),
                ("allowed_models", Leaf),
                (
                    "compaction",
                    Struct(HashMap::from([
                        ("mode", Leaf),
                        ("threshold_percent", Leaf),
                        ("protect_head", Leaf),
                        ("protect_tail_min", Leaf),
                        ("tail_budget_ratio", Leaf),
                        ("tool_prune_char_threshold", Leaf),
                        ("summary_model", Leaf),
                        ("max_summary_tokens", Leaf),
                        ("show_settings_hint", Leaf),
                    ])),
                ),
            ])),
        ),
        (
            "agents",
            Struct(HashMap::from([
                ("default_preset", Leaf),
                ("presets", Map(Box::new(agent_preset()))),
            ])),
        ),
        ("tools", tools()),
        (
            "skills",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("search_paths", Leaf),
                ("auto_load", Leaf),
                ("enable_agent_sidecar_files", Leaf),
            ])),
        ),
        (
            "mcp",
            Struct(HashMap::from([
                ("request_timeout_secs", Leaf),
                ("servers", Map(Box::new(mcp_server_entry()))),
            ])),
        ),
        ("channels", {
            // Channel accounts are stored as serde_json::Value but we
            // recognise a `tools` sub-key with typed group/sender policy.
            let group_policy = || {
                Struct(HashMap::from([
                    ("allow", Leaf),
                    ("deny", Leaf),
                    ("by_sender", Map(Box::new(tool_policy_entry()))),
                ]))
            };
            let channel_tools =
                || Struct(HashMap::from([("groups", Map(Box::new(group_policy())))]));
            let channel_account = || MapWithFields {
                value: Box::new(Leaf),
                fields: HashMap::from([("tools", channel_tools())]),
            };
            MapWithFields {
                // Dynamic keys: extra channel types via #[serde(flatten)]
                value: Box::new(Map(Box::new(channel_account()))),
                fields: HashMap::from([
                    ("offered", Array(Box::new(Leaf))),
                    ("telegram", Map(Box::new(channel_account()))),
                    ("whatsapp", Map(Box::new(channel_account()))),
                    ("msteams", Map(Box::new(channel_account()))),
                    ("discord", Map(Box::new(channel_account()))),
                    ("slack", Map(Box::new(channel_account()))),
                    ("nostr", Map(Box::new(channel_account()))),
                ]),
            }
        }),
        (
            "tls",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("auto_generate", Leaf),
                ("cert_path", Leaf),
                ("key_path", Leaf),
                ("ca_cert_path", Leaf),
                ("http_redirect_port", Leaf),
            ])),
        ),
        ("auth", Struct(HashMap::from([("disabled", Leaf)]))),
        ("graphql", Struct(HashMap::from([("enabled", Leaf)]))),
        (
            "metrics",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("prometheus_endpoint", Leaf),
                ("history_points", Leaf),
                ("labels", Map(Box::new(Leaf))),
            ])),
        ),
        (
            "identity",
            Struct(HashMap::from([
                ("name", Leaf),
                ("emoji", Leaf),
                ("theme", Leaf),
            ])),
        ),
        (
            "user",
            Struct(HashMap::from([("name", Leaf), ("timezone", Leaf)])),
        ),
        (
            "hooks",
            Struct(HashMap::from([(
                "hooks",
                Array(Box::new(shell_hook_entry())),
            )])),
        ),
        (
            "memory",
            Struct(HashMap::from([
                ("style", Leaf),
                ("agent_write_mode", Leaf),
                ("user_profile_write_mode", Leaf),
                ("backend", Leaf),
                ("provider", Leaf),
                ("embedding_provider", Leaf),
                ("disable_rag", Leaf),
                ("base_url", Leaf),
                ("embedding_base_url", Leaf),
                ("model", Leaf),
                ("embedding_model", Leaf),
                ("api_key", Leaf),
                ("embedding_api_key", Leaf),
                ("embedding_dimensions", Leaf),
                ("citations", Leaf),
                ("llm_reranking", Leaf),
                ("search_merge_strategy", Leaf),
                ("session_export", Leaf),
                ("qmd", qmd()),
            ])),
        ),
        (
            "ngrok",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("authtoken", Leaf),
                ("domain", Leaf),
            ])),
        ),
        (
            "tailscale",
            Struct(HashMap::from([("mode", Leaf), ("reset_on_exit", Leaf)])),
        ),
        (
            "failover",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("fallback_models", Leaf),
            ])),
        ),
        (
            "heartbeat",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("every", Leaf),
                ("model", Leaf),
                ("prompt", Leaf),
                ("ack_max_chars", Leaf),
                ("active_hours", active_hours()),
                ("deliver", Leaf),
                ("channel", Leaf),
                ("to", Leaf),
                ("sandbox_enabled", Leaf),
                ("sandbox_image", Leaf),
            ])),
        ),
        (
            "cron",
            Struct(HashMap::from([
                ("rate_limit_max", Leaf),
                ("rate_limit_window_secs", Leaf),
                ("session_retention_days", Leaf),
                ("auto_prune_cron_containers", Leaf),
            ])),
        ),
        ("env", Map(Box::new(Leaf))),
        ("upstream_proxy", Leaf),
        (
            "caldav",
            Struct(HashMap::from([
                ("enabled", Leaf),
                ("default_account", Leaf),
                (
                    "accounts",
                    Map(Box::new(Struct(HashMap::from([
                        ("url", Leaf),
                        ("username", Leaf),
                        ("password", Leaf),
                        ("provider", Leaf),
                        ("timeout_seconds", Leaf),
                    ])))),
                ),
            ])),
        ),
        (
            "webhooks",
            Struct(HashMap::from([(
                "rate_limit",
                Struct(HashMap::from([
                    ("enabled", Leaf),
                    ("requests_per_minute", Leaf),
                    ("burst", Leaf),
                    ("cleanup_interval_secs", Leaf),
                ])),
            )])),
        ),
        (
            "voice",
            Struct(HashMap::from([
                (
                    "tts",
                    Struct(HashMap::from([
                        ("enabled", Leaf),
                        ("provider", Leaf),
                        ("providers", Leaf),
                        (
                            "elevenlabs",
                            Struct(HashMap::from([
                                ("api_key", Leaf),
                                ("voice_id", Leaf),
                                ("model", Leaf),
                            ])),
                        ),
                        (
                            "openai",
                            Struct(HashMap::from([
                                ("api_key", Leaf),
                                ("base_url", Leaf),
                                ("voice", Leaf),
                                ("model", Leaf),
                            ])),
                        ),
                        (
                            "google",
                            Struct(HashMap::from([
                                ("api_key", Leaf),
                                ("language_code", Leaf),
                                ("voice", Leaf),
                                ("speaking_rate", Leaf),
                                ("pitch", Leaf),
                            ])),
                        ),
                        ("piper", Struct(HashMap::from([("model_path", Leaf)]))),
                        (
                            "coqui",
                            Struct(HashMap::from([
                                ("base_url", Leaf),
                                ("voice_id", Leaf),
                                ("endpoint", Leaf),
                            ])),
                        ),
                    ])),
                ),
                (
                    "stt",
                    Struct(HashMap::from([
                        ("enabled", Leaf),
                        ("provider", Leaf),
                        ("providers", Leaf),
                        (
                            "whisper",
                            Struct(HashMap::from([
                                ("api_key", Leaf),
                                ("base_url", Leaf),
                                ("model", Leaf),
                                ("language", Leaf),
                            ])),
                        ),
                        (
                            "groq",
                            Struct(HashMap::from([
                                ("api_key", Leaf),
                                ("model", Leaf),
                                ("language", Leaf),
                            ])),
                        ),
                        (
                            "deepgram",
                            Struct(HashMap::from([
                                ("api_key", Leaf),
                                ("model", Leaf),
                                ("language", Leaf),
                                ("smart_format", Leaf),
                            ])),
                        ),
                        (
                            "google",
                            Struct(HashMap::from([("api_key", Leaf), ("language_code", Leaf)])),
                        ),
                        (
                            "mistral",
                            Struct(HashMap::from([
                                ("api_key", Leaf),
                                ("model", Leaf),
                                ("language", Leaf),
                            ])),
                        ),
                        (
                            "elevenlabs",
                            Struct(HashMap::from([
                                ("api_key", Leaf),
                                ("model", Leaf),
                                ("language", Leaf),
                            ])),
                        ),
                        (
                            "voxtral_local",
                            Struct(HashMap::from([
                                ("base_url", Leaf),
                                ("model", Leaf),
                                ("endpoint", Leaf),
                            ])),
                        ),
                        (
                            "whisper_cli",
                            Struct(HashMap::from([
                                ("binary_path", Leaf),
                                ("model_path", Leaf),
                                ("language", Leaf),
                            ])),
                        ),
                        (
                            "sherpa_onnx",
                            Struct(HashMap::from([
                                ("model_dir", Leaf),
                                ("language", Leaf),
                                ("sample_rate", Leaf),
                            ])),
                        ),
                    ])),
                ),
            ])),
        ),
    ]))
}

// ── Levenshtein distance ────────────────────────────────────────────────────

/// Compute the Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_len = a.len();
    let b_len = b.len();
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr = vec![0; b_len + 1];

    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let cost = if ca == cb {
                0
            } else {
                1
            };
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_len]
}

/// Find the best match for `needle` among `candidates` using Levenshtein
/// distance. Returns `Some(best)` if the distance is <= `max_distance`.
fn suggest<'a>(needle: &str, candidates: &[&'a str], max_distance: usize) -> Option<&'a str> {
    let mut best: Option<(&'a str, usize)> = None;
    for &candidate in candidates {
        let d = levenshtein(needle, candidate);
        if d > 0 && d <= max_distance && best.as_ref().is_none_or(|(_, bd)| d < *bd) {
            best = Some((candidate, d));
        }
    }
    best.map(|(s, _)| s)
}

// ── Core validation ─────────────────────────────────────────────────────────

/// Walk the TOML value tree against the schema tree and flag unknown keys.
pub(super) fn check_unknown_fields(
    value: &toml::Value,
    schema: &KnownKeys,
    prefix: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match (value, schema) {
        (toml::Value::Table(table), KnownKeys::Struct(fields)) => {
            let known_keys: Vec<&str> = fields.keys().copied().collect();
            for (key, child_value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                if let Some(child_schema) = fields.get(key.as_str()) {
                    check_unknown_fields(child_value, child_schema, &path, diagnostics);
                } else {
                    let level = if prefix.is_empty() {
                        "at top level "
                    } else {
                        ""
                    };
                    let suggestion = suggest(key, &known_keys, 3);
                    let msg = if let Some(s) = suggestion {
                        format!("unknown field {level}(did you mean \"{s}\"?)")
                    } else {
                        format!("unknown field {level}")
                    };
                    diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        category: "unknown-field",
                        path,
                        message: msg.trim().to_string(),
                    });
                }
            }
        },
        (toml::Value::Table(table), KnownKeys::Map(value_schema)) => {
            for (key, child_value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                check_unknown_fields(child_value, value_schema, &path, diagnostics);
            }
        },
        (
            toml::Value::Table(table),
            KnownKeys::MapWithFields {
                value: value_schema,
                fields,
            },
        ) => {
            for (key, child_value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                if let Some(child_schema) = fields.get(key.as_str()) {
                    check_unknown_fields(child_value, child_schema, &path, diagnostics);
                } else {
                    check_unknown_fields(child_value, value_schema, &path, diagnostics);
                }
            }
        },
        (toml::Value::Array(arr), KnownKeys::Array(item_schema)) => {
            for (i, item) in arr.iter().enumerate() {
                let path = format!("{prefix}[{i}]");
                check_unknown_fields(item, item_schema, &path, diagnostics);
            }
        },
        // Leaf or type mismatch — stop recursion (type errors caught later)
        _ => {},
    }
}
