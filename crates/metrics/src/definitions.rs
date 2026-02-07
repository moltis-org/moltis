//! Metric name and label definitions.
//!
//! This module defines all metric names and common label keys used throughout moltis.
//! Centralizing these definitions ensures consistency and makes it easier to document
//! what metrics are available.

/// HTTP request metrics
pub mod http {
    /// Total number of HTTP requests handled
    pub const REQUESTS_TOTAL: &str = "moltis_http_requests_total";
    /// Duration of HTTP requests in seconds
    pub const REQUEST_DURATION_SECONDS: &str = "moltis_http_request_duration_seconds";
    /// Number of currently in-flight HTTP requests
    pub const REQUESTS_IN_FLIGHT: &str = "moltis_http_requests_in_flight";
    /// Total bytes received in HTTP requests
    pub const REQUEST_BYTES_TOTAL: &str = "moltis_http_request_bytes_total";
    /// Total bytes sent in HTTP responses
    pub const RESPONSE_BYTES_TOTAL: &str = "moltis_http_response_bytes_total";
}

/// WebSocket metrics
pub mod websocket {
    /// Total number of WebSocket connections established
    pub const CONNECTIONS_TOTAL: &str = "moltis_websocket_connections_total";
    /// Number of currently active WebSocket connections
    pub const CONNECTIONS_ACTIVE: &str = "moltis_websocket_connections_active";
    /// Total number of WebSocket messages received
    pub const MESSAGES_RECEIVED_TOTAL: &str = "moltis_websocket_messages_received_total";
    /// Total number of WebSocket messages sent
    pub const MESSAGES_SENT_TOTAL: &str = "moltis_websocket_messages_sent_total";
    /// WebSocket message processing duration in seconds
    pub const MESSAGE_DURATION_SECONDS: &str = "moltis_websocket_message_duration_seconds";
}

/// LLM/Agent metrics
pub mod llm {
    /// Total number of LLM completions requested
    pub const COMPLETIONS_TOTAL: &str = "moltis_llm_completions_total";
    /// Duration of LLM completion requests in seconds
    pub const COMPLETION_DURATION_SECONDS: &str = "moltis_llm_completion_duration_seconds";
    /// Total input tokens processed
    pub const INPUT_TOKENS_TOTAL: &str = "moltis_llm_input_tokens_total";
    /// Total output tokens generated
    pub const OUTPUT_TOKENS_TOTAL: &str = "moltis_llm_output_tokens_total";
    /// Total cache read tokens (for providers that support caching)
    pub const CACHE_READ_TOKENS_TOTAL: &str = "moltis_llm_cache_read_tokens_total";
    /// Total cache write tokens (for providers that support caching)
    pub const CACHE_WRITE_TOKENS_TOTAL: &str = "moltis_llm_cache_write_tokens_total";
    /// LLM completion errors
    pub const COMPLETION_ERRORS_TOTAL: &str = "moltis_llm_completion_errors_total";
    /// Time to first token in seconds (streaming latency)
    pub const TIME_TO_FIRST_TOKEN_SECONDS: &str = "moltis_llm_time_to_first_token_seconds";
    /// Tokens per second generation rate
    pub const TOKENS_PER_SECOND: &str = "moltis_llm_tokens_per_second";
}

/// Session metrics
pub mod session {
    /// Total number of sessions created
    pub const CREATED_TOTAL: &str = "moltis_sessions_created_total";
    /// Number of currently active sessions
    pub const ACTIVE: &str = "moltis_sessions_active";
    /// Total number of messages in sessions
    pub const MESSAGES_TOTAL: &str = "moltis_session_messages_total";
    /// Session duration in seconds
    pub const DURATION_SECONDS: &str = "moltis_session_duration_seconds";
}

/// Chat metrics
pub mod chat {
    /// Total number of chat messages sent
    pub const MESSAGES_SENT_TOTAL: &str = "moltis_chat_messages_sent_total";
    /// Total number of chat messages received
    pub const MESSAGES_RECEIVED_TOTAL: &str = "moltis_chat_messages_received_total";
    /// Chat message processing duration in seconds
    pub const PROCESSING_DURATION_SECONDS: &str = "moltis_chat_processing_duration_seconds";
}

/// Tool execution metrics
pub mod tools {
    /// Total number of tool executions
    pub const EXECUTIONS_TOTAL: &str = "moltis_tool_executions_total";
    /// Tool execution duration in seconds
    pub const EXECUTION_DURATION_SECONDS: &str = "moltis_tool_execution_duration_seconds";
    /// Tool execution errors
    pub const EXECUTION_ERRORS_TOTAL: &str = "moltis_tool_execution_errors_total";
    /// Number of currently running tool executions
    pub const EXECUTIONS_IN_FLIGHT: &str = "moltis_tool_executions_in_flight";
}

/// Sandbox metrics
pub mod sandbox {
    /// Total number of sandbox command executions
    pub const COMMAND_EXECUTIONS_TOTAL: &str = "moltis_sandbox_command_executions_total";
    /// Sandbox command execution duration in seconds
    pub const COMMAND_DURATION_SECONDS: &str = "moltis_sandbox_command_duration_seconds";
    /// Sandbox command errors
    pub const COMMAND_ERRORS_TOTAL: &str = "moltis_sandbox_command_errors_total";
    /// Number of sandbox images available
    pub const IMAGES_AVAILABLE: &str = "moltis_sandbox_images_available";
}

/// MCP (Model Context Protocol) metrics
pub mod mcp {
    /// Total number of MCP server connections
    pub const SERVER_CONNECTIONS_TOTAL: &str = "moltis_mcp_server_connections_total";
    /// Number of currently connected MCP servers
    pub const SERVERS_CONNECTED: &str = "moltis_mcp_servers_connected";
    /// Total number of MCP tool calls
    pub const TOOL_CALLS_TOTAL: &str = "moltis_mcp_tool_calls_total";
    /// MCP tool call duration in seconds
    pub const TOOL_CALL_DURATION_SECONDS: &str = "moltis_mcp_tool_call_duration_seconds";
    /// MCP tool call errors
    pub const TOOL_CALL_ERRORS_TOTAL: &str = "moltis_mcp_tool_call_errors_total";
    /// Total number of MCP resource reads
    pub const RESOURCE_READS_TOTAL: &str = "moltis_mcp_resource_reads_total";
    /// Total number of MCP prompt fetches
    pub const PROMPT_FETCHES_TOTAL: &str = "moltis_mcp_prompt_fetches_total";
}

/// Channel metrics (Telegram, etc.)
pub mod channels {
    /// Total number of channel messages received
    pub const MESSAGES_RECEIVED_TOTAL: &str = "moltis_channel_messages_received_total";
    /// Total number of channel messages sent
    pub const MESSAGES_SENT_TOTAL: &str = "moltis_channel_messages_sent_total";
    /// Number of active channels
    pub const ACTIVE: &str = "moltis_channels_active";
    /// Channel errors
    pub const ERRORS_TOTAL: &str = "moltis_channel_errors_total";
}

/// Memory/embedding metrics
pub mod memory {
    /// Total number of memory searches performed
    pub const SEARCHES_TOTAL: &str = "moltis_memory_searches_total";
    /// Memory search duration in seconds
    pub const SEARCH_DURATION_SECONDS: &str = "moltis_memory_search_duration_seconds";
    /// Total number of embeddings generated
    pub const EMBEDDINGS_GENERATED_TOTAL: &str = "moltis_memory_embeddings_generated_total";
    /// Number of documents in memory
    pub const DOCUMENTS_COUNT: &str = "moltis_memory_documents_count";
    /// Total memory size in bytes
    pub const SIZE_BYTES: &str = "moltis_memory_size_bytes";
}

/// Plugin metrics
pub mod plugins {
    /// Number of loaded plugins
    pub const LOADED: &str = "moltis_plugins_loaded";
    /// Total plugin executions
    pub const EXECUTIONS_TOTAL: &str = "moltis_plugin_executions_total";
    /// Plugin execution duration in seconds
    pub const EXECUTION_DURATION_SECONDS: &str = "moltis_plugin_execution_duration_seconds";
    /// Plugin errors
    pub const ERRORS_TOTAL: &str = "moltis_plugin_errors_total";
    /// Plugin installation attempts
    pub const INSTALLATION_ATTEMPTS_TOTAL: &str = "moltis_plugin_installation_attempts_total";
    /// Plugin installation duration in seconds
    pub const INSTALLATION_DURATION_SECONDS: &str = "moltis_plugin_installation_duration_seconds";
    /// Plugin installation errors
    pub const INSTALLATION_ERRORS_TOTAL: &str = "moltis_plugin_installation_errors_total";
    /// Hook executions by type
    pub const HOOK_EXECUTIONS_TOTAL: &str = "moltis_plugin_hook_executions_total";
    /// Hook execution duration in seconds
    pub const HOOK_EXECUTION_DURATION_SECONDS: &str =
        "moltis_plugin_hook_execution_duration_seconds";
    /// Hook execution errors
    pub const HOOK_ERRORS_TOTAL: &str = "moltis_plugin_hook_errors_total";
    /// Git clone attempts
    pub const GIT_CLONE_ATTEMPTS_TOTAL: &str = "moltis_plugin_git_clone_attempts_total";
    /// Git clone fallbacks to HTTP
    pub const GIT_CLONE_FALLBACK_TOTAL: &str = "moltis_plugin_git_clone_fallback_total";
}

/// Cron job metrics
pub mod cron {
    /// Number of scheduled cron jobs
    pub const JOBS_SCHEDULED: &str = "moltis_cron_jobs_scheduled";
    /// Jobs currently due to run
    pub const JOBS_DUE: &str = "moltis_cron_jobs_due";
    /// Total cron job executions
    pub const EXECUTIONS_TOTAL: &str = "moltis_cron_executions_total";
    /// Cron job execution duration in seconds
    pub const EXECUTION_DURATION_SECONDS: &str = "moltis_cron_execution_duration_seconds";
    /// Cron job errors
    pub const ERRORS_TOTAL: &str = "moltis_cron_errors_total";
    /// Stuck jobs cleared (exceeded 2h threshold)
    pub const STUCK_JOBS_CLEARED_TOTAL: &str = "moltis_cron_stuck_jobs_cleared_total";
    /// Input tokens from cron agent runs
    pub const INPUT_TOKENS_TOTAL: &str = "moltis_cron_input_tokens_total";
    /// Output tokens from cron agent runs
    pub const OUTPUT_TOKENS_TOTAL: &str = "moltis_cron_output_tokens_total";
    /// Timer loop latency (delay from due time to execution start)
    pub const TIMER_LOOP_LATENCY_SECONDS: &str = "moltis_cron_timer_loop_latency_seconds";
    /// Store operation duration by operation type
    pub const STORE_OPERATION_DURATION_SECONDS: &str =
        "moltis_cron_store_operation_duration_seconds";
}

/// Authentication metrics
pub mod auth {
    /// Total login attempts
    pub const LOGIN_ATTEMPTS_TOTAL: &str = "moltis_auth_login_attempts_total";
    /// Successful logins
    pub const LOGIN_SUCCESS_TOTAL: &str = "moltis_auth_login_success_total";
    /// Failed logins
    pub const LOGIN_FAILURES_TOTAL: &str = "moltis_auth_login_failures_total";
    /// Active sessions
    pub const ACTIVE_SESSIONS: &str = "moltis_auth_active_sessions";
    /// API key authentications
    pub const API_KEY_AUTH_TOTAL: &str = "moltis_auth_api_key_auth_total";
}

/// System/runtime metrics
pub mod system {
    /// Process uptime in seconds
    pub const UPTIME_SECONDS: &str = "moltis_uptime_seconds";
    /// Build information (labels: version, commit, build_date)
    pub const BUILD_INFO: &str = "moltis_build_info";
    /// Number of connected clients
    pub const CONNECTED_CLIENTS: &str = "moltis_connected_clients";
}

/// Auto-reply pipeline metrics
pub mod auto_reply {
    /// Total messages received for processing
    pub const MESSAGES_RECEIVED_TOTAL: &str = "moltis_auto_reply_messages_received_total";
    /// Message processing duration in seconds
    pub const PROCESSING_DURATION_SECONDS: &str = "moltis_auto_reply_processing_duration_seconds";
    /// Queue size by mode (per_message, batch, debounce)
    pub const QUEUE_SIZE: &str = "moltis_auto_reply_queue_size";
    /// Messages dropped due to policy
    pub const MESSAGES_DROPPED_TOTAL: &str = "moltis_auto_reply_messages_dropped_total";
    /// Directive parse errors
    pub const DIRECTIVE_PARSE_ERRORS_TOTAL: &str = "moltis_auto_reply_directive_parse_errors_total";
    /// Response chunk operations
    pub const CHUNK_OPERATIONS_TOTAL: &str = "moltis_auto_reply_chunk_operations_total";
    /// Delivery failures by channel type
    pub const DELIVERY_FAILURES_TOTAL: &str = "moltis_auto_reply_delivery_failures_total";
}

/// Browser automation metrics
pub mod browser {
    /// Active browser instances
    pub const INSTANCES_ACTIVE: &str = "moltis_browser_instances_active";
    /// Total browser instances created
    pub const INSTANCES_CREATED_TOTAL: &str = "moltis_browser_instances_created_total";
    /// Total browser instances destroyed
    pub const INSTANCES_DESTROYED_TOTAL: &str = "moltis_browser_instances_destroyed_total";
    /// Total screenshots taken
    pub const SCREENSHOTS_TOTAL: &str = "moltis_browser_screenshots_total";
    /// Navigation duration in seconds
    pub const NAVIGATION_DURATION_SECONDS: &str = "moltis_browser_navigation_duration_seconds";
    /// Browser errors by type
    pub const ERRORS_TOTAL: &str = "moltis_browser_errors_total";
    /// Browser pool utilization (0-1)
    pub const POOL_UTILIZATION: &str = "moltis_browser_pool_utilization";
}

/// Canvas (A2UI) metrics
pub mod canvas {
    /// Active WebSocket connections
    pub const CONNECTIONS_ACTIVE: &str = "moltis_canvas_connections_active";
    /// Total messages received from UI
    pub const MESSAGES_RECEIVED_TOTAL: &str = "moltis_canvas_messages_received_total";
    /// Total messages sent to UI
    pub const MESSAGES_SENT_TOTAL: &str = "moltis_canvas_messages_sent_total";
    /// Message round-trip latency in seconds
    pub const MESSAGE_LATENCY_SECONDS: &str = "moltis_canvas_message_latency_seconds";
    /// Page serve duration in seconds
    pub const PAGE_SERVE_DURATION_SECONDS: &str = "moltis_canvas_page_serve_duration_seconds";
    /// WebSocket errors
    pub const WEBSOCKET_ERRORS_TOTAL: &str = "moltis_canvas_websocket_errors_total";
    /// Content size in bytes
    pub const CONTENT_SIZE_BYTES: &str = "moltis_canvas_content_size_bytes";
}

/// Media pipeline metrics
pub mod media {
    /// Total downloads attempted
    pub const DOWNLOADS_TOTAL: &str = "moltis_media_downloads_total";
    /// Download duration in seconds
    pub const DOWNLOAD_DURATION_SECONDS: &str = "moltis_media_download_duration_seconds";
    /// Download errors by type
    pub const DOWNLOAD_ERRORS_TOTAL: &str = "moltis_media_download_errors_total";
    /// Total bytes downloaded
    pub const DOWNLOAD_BYTES_TOTAL: &str = "moltis_media_download_bytes_total";
    /// Files stored
    pub const FILES_STORED_TOTAL: &str = "moltis_media_files_stored_total";
    /// Total storage size in bytes
    pub const STORAGE_SIZE_BYTES: &str = "moltis_media_storage_size_bytes";
    /// Image resize operations by format
    pub const IMAGE_RESIZES_TOTAL: &str = "moltis_media_image_resizes_total";
    /// Image resize duration in seconds
    pub const IMAGE_RESIZE_DURATION_SECONDS: &str = "moltis_media_image_resize_duration_seconds";
    /// Audio transcription operations
    pub const TRANSCRIPTIONS_TOTAL: &str = "moltis_media_transcriptions_total";
    /// Transcription duration in seconds
    pub const TRANSCRIPTION_DURATION_SECONDS: &str = "moltis_media_transcription_duration_seconds";
    /// TTL cleanup operations
    pub const CLEANUP_OPERATIONS_TOTAL: &str = "moltis_media_cleanup_operations_total";
    /// Files expired and removed
    pub const FILES_EXPIRED_TOTAL: &str = "moltis_media_files_expired_total";
}

/// OAuth metrics
pub mod oauth {
    /// OAuth flow starts by provider
    pub const FLOW_STARTS_TOTAL: &str = "moltis_oauth_flow_starts_total";
    /// OAuth flow completions by provider and status
    pub const FLOW_COMPLETIONS_TOTAL: &str = "moltis_oauth_flow_completions_total";
    /// OAuth flow duration in seconds
    pub const FLOW_DURATION_SECONDS: &str = "moltis_oauth_flow_duration_seconds";
    /// Token refreshes by provider
    pub const TOKEN_REFRESH_TOTAL: &str = "moltis_oauth_token_refresh_total";
    /// Token refresh failures by provider
    pub const TOKEN_REFRESH_FAILURES_TOTAL: &str = "moltis_oauth_token_refresh_failures_total";
    /// Device flow attempts by provider
    pub const DEVICE_FLOW_ATTEMPTS_TOTAL: &str = "moltis_oauth_device_flow_attempts_total";
    /// Device flow errors by provider and type
    pub const DEVICE_FLOW_ERRORS_TOTAL: &str = "moltis_oauth_device_flow_errors_total";
    /// Code exchange operations
    pub const CODE_EXCHANGE_TOTAL: &str = "moltis_oauth_code_exchange_total";
    /// Code exchange errors
    pub const CODE_EXCHANGE_ERRORS_TOTAL: &str = "moltis_oauth_code_exchange_errors_total";
    /// Callback server requests
    pub const CALLBACK_REQUESTS_TOTAL: &str = "moltis_oauth_callback_requests_total";
}

/// Onboarding wizard metrics
pub mod onboarding {
    /// Onboarding sessions started
    pub const SESSIONS_STARTED_TOTAL: &str = "moltis_onboarding_sessions_started_total";
    /// Onboarding sessions completed
    pub const SESSIONS_COMPLETED_TOTAL: &str = "moltis_onboarding_sessions_completed_total";
    /// Onboarding sessions abandoned
    pub const SESSIONS_ABANDONED_TOTAL: &str = "moltis_onboarding_sessions_abandoned_total";
    /// Session duration in seconds
    pub const SESSION_DURATION_SECONDS: &str = "moltis_onboarding_session_duration_seconds";
    /// Step duration in seconds by step name
    pub const STEP_DURATION_SECONDS: &str = "moltis_onboarding_step_duration_seconds";
    /// Step abandonments by step name
    pub const STEP_ABANDONMENTS_TOTAL: &str = "moltis_onboarding_step_abandonments_total";
    /// Data validation errors by field
    pub const VALIDATION_ERRORS_TOTAL: &str = "moltis_onboarding_validation_errors_total";
}

/// Projects metrics
pub mod projects {
    /// Total projects managed
    pub const TOTAL: &str = "moltis_projects_total";
    /// Projects created
    pub const CREATED_TOTAL: &str = "moltis_projects_created_total";
    /// Projects auto-detected
    pub const DETECTED_TOTAL: &str = "moltis_projects_detected_total";
    /// Context load duration (CLAUDE.md/AGENTS.md)
    pub const CONTEXT_LOAD_DURATION_SECONDS: &str = "moltis_projects_context_load_duration_seconds";
    /// Context load errors by file type
    pub const CONTEXT_LOAD_ERRORS_TOTAL: &str = "moltis_projects_context_load_errors_total";
    /// Worktree creation operations
    pub const WORKTREE_CREATIONS_TOTAL: &str = "moltis_projects_worktree_creations_total";
    /// Worktree creation duration in seconds
    pub const WORKTREE_CREATION_DURATION_SECONDS: &str =
        "moltis_projects_worktree_creation_duration_seconds";
    /// Worktree errors by type
    pub const WORKTREE_ERRORS_TOTAL: &str = "moltis_projects_worktree_errors_total";
    /// Detection duration in seconds
    pub const DETECTION_DURATION_SECONDS: &str = "moltis_projects_detection_duration_seconds";
}

/// Protocol metrics
pub mod protocol {
    /// Frame validation errors by frame type
    pub const FRAME_VALIDATION_ERRORS_TOTAL: &str = "moltis_protocol_frame_validation_errors_total";
    /// Handshake duration in seconds
    pub const HANDSHAKE_DURATION_SECONDS: &str = "moltis_protocol_handshake_duration_seconds";
    /// Handshake timeouts
    pub const HANDSHAKE_TIMEOUTS_TOTAL: &str = "moltis_protocol_handshake_timeouts_total";
    /// Payload size violations by limit type
    pub const PAYLOAD_SIZE_VIOLATIONS_TOTAL: &str = "moltis_protocol_payload_size_violations_total";
    /// Deduplication operations
    pub const DEDUPE_OPERATIONS_TOTAL: &str = "moltis_protocol_dedupe_operations_total";
    /// Frame rate exceeded events
    pub const FRAME_RATE_EXCEEDED_TOTAL: &str = "moltis_protocol_frame_rate_exceeded_total";
}

/// Routing metrics
pub mod routing {
    /// Route resolutions by binding level
    pub const RESOLUTIONS_TOTAL: &str = "moltis_routing_resolutions_total";
    /// Resolution duration in seconds
    pub const RESOLUTION_DURATION_SECONDS: &str = "moltis_routing_resolution_duration_seconds";
    /// Resolution errors by type
    pub const RESOLUTION_ERRORS_TOTAL: &str = "moltis_routing_resolution_errors_total";
    /// Fallback to default agent
    pub const FALLBACK_TO_DEFAULT_TOTAL: &str = "moltis_routing_fallback_to_default_total";
    /// Session key generations
    pub const SESSION_KEY_GENERATIONS_TOTAL: &str = "moltis_routing_session_key_generations_total";
}

/// Skills metrics
pub mod skills {
    /// Total skills discovered
    pub const TOTAL: &str = "moltis_skills_total";
    /// Discovery operations
    pub const DISCOVERY_OPERATIONS_TOTAL: &str = "moltis_skills_discovery_operations_total";
    /// Discovery duration in seconds
    pub const DISCOVERY_DURATION_SECONDS: &str = "moltis_skills_discovery_duration_seconds";
    /// Parse operations (SKILL.md)
    pub const PARSE_OPERATIONS_TOTAL: &str = "moltis_skills_parse_operations_total";
    /// Parse errors by type
    pub const PARSE_ERRORS_TOTAL: &str = "moltis_skills_parse_errors_total";
    /// Installation attempts
    pub const INSTALLATION_ATTEMPTS_TOTAL: &str = "moltis_skills_installation_attempts_total";
    /// Installation duration in seconds
    pub const INSTALLATION_DURATION_SECONDS: &str = "moltis_skills_installation_duration_seconds";
    /// Installation errors
    pub const INSTALLATION_ERRORS_TOTAL: &str = "moltis_skills_installation_errors_total";
    /// Prompt generation operations
    pub const PROMPT_GENERATION_TOTAL: &str = "moltis_skills_prompt_generation_total";
    /// Prompt generation duration
    pub const PROMPT_GENERATION_DURATION_SECONDS: &str =
        "moltis_skills_prompt_generation_duration_seconds";
}

/// Telegram channel metrics
pub mod telegram {
    /// Messages received from Telegram
    pub const MESSAGES_RECEIVED_TOTAL: &str = "moltis_telegram_messages_received_total";
    /// Messages sent to Telegram
    pub const MESSAGES_SENT_TOTAL: &str = "moltis_telegram_messages_sent_total";
    /// Message send duration in seconds
    pub const MESSAGE_SEND_DURATION_SECONDS: &str = "moltis_telegram_message_send_duration_seconds";
    /// Message send errors by type
    pub const MESSAGE_SEND_ERRORS_TOTAL: &str = "moltis_telegram_message_send_errors_total";
    /// Streaming edit operations
    pub const STREAMING_EDITS_TOTAL: &str = "moltis_telegram_streaming_edits_total";
    /// Bot connection duration
    pub const BOT_CONNECTION_DURATION_SECONDS: &str =
        "moltis_telegram_bot_connection_duration_seconds";
    /// Bot connection errors
    pub const BOT_CONNECTION_ERRORS_TOTAL: &str = "moltis_telegram_bot_connection_errors_total";
    /// Active Telegram accounts/bots
    pub const ACTIVE_ACCOUNTS: &str = "moltis_telegram_active_accounts";
    /// Access control denials
    pub const ACCESS_CONTROL_DENIALS_TOTAL: &str = "moltis_telegram_access_control_denials_total";
    /// Update polling duration
    pub const POLLING_DURATION_SECONDS: &str = "moltis_telegram_polling_duration_seconds";
    /// OTP challenges issued to non-allowlisted users
    pub const OTP_CHALLENGES_TOTAL: &str = "moltis_telegram_otp_challenges_total";
    /// OTP verification attempts (labelled by result: approved, wrong_code, locked_out, expired)
    pub const OTP_VERIFICATIONS_TOTAL: &str = "moltis_telegram_otp_verifications_total";
}

/// Config loading metrics
pub mod config {
    /// Config load duration in seconds
    pub const LOAD_DURATION_SECONDS: &str = "moltis_config_load_duration_seconds";
    /// Config parse errors by format
    pub const PARSE_ERRORS_TOTAL: &str = "moltis_config_parse_errors_total";
    /// Environment substitution failures
    pub const ENV_SUBSTITUTION_FAILURES_TOTAL: &str =
        "moltis_config_env_substitution_failures_total";
    /// Config migration operations
    pub const MIGRATION_OPERATIONS_TOTAL: &str = "moltis_config_migration_operations_total";
    /// Config reload duration in seconds
    pub const RELOAD_DURATION_SECONDS: &str = "moltis_config_reload_duration_seconds";
    /// Validation errors by rule type
    pub const VALIDATION_ERRORS_TOTAL: &str = "moltis_config_validation_errors_total";
}

/// Common/shared metrics
pub mod common {
    /// Application errors by type
    pub const ERRORS_TOTAL: &str = "moltis_errors_total";
    /// Hook executions
    pub const HOOKS_EXECUTED_TOTAL: &str = "moltis_hooks_executed_total";
    /// Validation failures by category
    pub const VALIDATION_FAILURES_TOTAL: &str = "moltis_validation_failures_total";
}

/// Common label keys used across metrics
pub mod labels {
    pub const ENDPOINT: &str = "endpoint";
    pub const METHOD: &str = "method";
    pub const STATUS: &str = "status";
    pub const PROVIDER: &str = "provider";
    pub const MODEL: &str = "model";
    pub const TOOL: &str = "tool";
    pub const CHANNEL: &str = "channel";
    pub const SERVER: &str = "server";
    pub const ERROR_TYPE: &str = "error_type";
    pub const ROLE: &str = "role";
    pub const SUCCESS: &str = "success";
    pub const OPERATION: &str = "operation";
    pub const STEP: &str = "step";
    pub const BINDING_LEVEL: &str = "binding_level";
    pub const HOOK_TYPE: &str = "hook_type";
    pub const FORMAT: &str = "format";
    pub const FRAME_TYPE: &str = "frame_type";
    pub const SEARCH_TYPE: &str = "search_type";
    pub const MODE: &str = "mode";
    pub const ACCOUNT_ID: &str = "account_id";
    pub const FILE_TYPE: &str = "file_type";
}

/// Standard histogram buckets for different metric types
pub mod buckets {
    use once_cell::sync::Lazy;

    /// HTTP request duration buckets (in seconds)
    /// Covers 1ms to 60s
    pub static HTTP_DURATION: Lazy<Vec<f64>> = Lazy::new(|| {
        vec![
            0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
        ]
    });

    /// LLM completion duration buckets (in seconds)
    /// Covers 100ms to 5 minutes (LLM calls can be slow)
    pub static LLM_DURATION: Lazy<Vec<f64>> = Lazy::new(|| {
        vec![
            0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 180.0, 300.0,
        ]
    });

    /// Time to first token buckets (in seconds)
    /// Covers 10ms to 30s
    pub static TTFT: Lazy<Vec<f64>> = Lazy::new(|| {
        vec![
            0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0,
        ]
    });

    /// Tool execution duration buckets (in seconds)
    /// Covers 1ms to 5 minutes
    pub static TOOL_DURATION: Lazy<Vec<f64>> = Lazy::new(|| {
        vec![
            0.001, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0,
        ]
    });

    /// Token count buckets
    /// Covers 1 to 200k tokens
    pub static TOKEN_COUNT: Lazy<Vec<f64>> = Lazy::new(|| {
        vec![
            1.0, 10.0, 50.0, 100.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0, 32000.0,
            64000.0, 128000.0, 200000.0,
        ]
    });

    /// Tokens per second buckets
    /// Covers 1 to 500 tokens/sec
    pub static TOKENS_PER_SECOND: Lazy<Vec<f64>> = Lazy::new(|| {
        vec![
            1.0, 5.0, 10.0, 20.0, 30.0, 40.0, 50.0, 75.0, 100.0, 150.0, 200.0, 300.0, 500.0,
        ]
    });

    /// Download/upload duration buckets (in seconds)
    /// Covers 10ms to 5 minutes
    pub static DOWNLOAD_DURATION: Lazy<Vec<f64>> = Lazy::new(|| {
        vec![
            0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0,
        ]
    });

    /// File size buckets (in bytes)
    /// Covers 1KB to 1GB
    pub static FILE_SIZE: Lazy<Vec<f64>> = Lazy::new(|| {
        vec![
            1024.0,       // 1KB
            10240.0,      // 10KB
            102400.0,     // 100KB
            1048576.0,    // 1MB
            10485760.0,   // 10MB
            104857600.0,  // 100MB
            1073741824.0, // 1GB
        ]
    });

    /// Queue size buckets
    /// Covers 1 to 10000
    pub static QUEUE_SIZE: Lazy<Vec<f64>> = Lazy::new(|| {
        vec![
            1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 5000.0, 10000.0,
        ]
    });
}
