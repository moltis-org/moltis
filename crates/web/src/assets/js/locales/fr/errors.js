// ── Error English strings ───────────────────────────────────
// Error classification titles and details.

export default {
	generic: {
		title: "Error",
	},
	usageLimitReached: {
		title: "Usage limit reached",
		detail: "Your {{planType}} plan limit has been reached.",
	},
	rateLimited: {
		title: "Rate limited",
		detail: "Too many requests. Please wait a moment.",
		detailShort: "Too many requests.",
	},
	authError: {
		title: "Authentication error",
		detail: "Your session may have expired.",
	},
	serverError: {
		title: "Server error",
		detail: "The upstream provider returned an error.",
	},
	chat: {
		usageLimitReached: {
			title: "Usage limit reached",
			detail: "Your {{planType}} plan limit has been reached.",
		},
		rateLimited: {
			title: "Rate limited",
			detail: "Too many requests. Please wait a moment and try again.",
		},
		authError: {
			title: "Authentication error",
			detail: "Your session may have expired or credentials are invalid.",
		},
		serverError: {
			title: "Server error",
			detail: "The upstream provider returned an error. Please try again later.",
		},
		unsupportedModel: {
			title: "Model not supported",
		},
	},
	wsNotConnected: "WebSocket not connected",
	codes: {
		NOT_LINKED: "No project is linked for this action.",
		NOT_PAIRED: "Device pairing is required before continuing.",
		AGENT_TIMEOUT: "The agent took too long to respond. Please try again.",
		INVALID_REQUEST: "The request was invalid. Please check the input and try again.",
		UNAVAILABLE: "Service temporarily unavailable. Please try again.",
		RATE_LIMITED: "Too many requests. Please wait a moment.",
		AUTH_SETUP_REQUIRED: "Setup is required before continuing.",
		AUTH_NOT_AUTHENTICATED: "You are not authenticated.",
		METRICS_NOT_ENABLED: "Metrics are not enabled.",
		UPLOAD_EMPTY_BODY: "Upload body is empty.",
		UPLOAD_BODY_TOO_LARGE: "Upload is too large.",
		UPLOAD_SESSION_STORE_UNAVAILABLE: "Upload session store is unavailable.",
		UPLOAD_SAVE_FAILED: "Failed to save uploaded file.",
		CONFIG_AUTH_REQUIRED: "Configuration access requires authentication.",
		CONFIG_READ_FAILED: "Failed to read configuration.",
		CONFIG_TOML_REQUIRED: "Configuration payload is missing TOML.",
		CONFIG_INVALID_TOML: "Configuration TOML is invalid.",
		CONFIG_SAVE_FAILED: "Failed to save configuration.",
		CONFIG_RESTART_INVALID: "Cannot restart, configuration is invalid.",
		CONFIG_RESTART_READ_FAILED: "Cannot restart, failed to read configuration.",
		TAILSCALE_STATUS_FAILED: "Failed to query Tailscale status.",
		TAILSCALE_MODE_INVALID: "Invalid Tailscale mode.",
		TAILSCALE_CONFIG_INVALID: "Invalid Tailscale configuration.",
		TAILSCALE_CONFIGURE_FAILED: "Failed to configure Tailscale.",
		IMAGE_CACHE_LIST_FAILED: "Failed to list cached images.",
		IMAGE_CACHE_DELETE_FAILED: "Failed to delete cached image.",
		IMAGE_CACHE_PRUNE_FAILED: "Failed to prune cached images.",
		SANDBOX_CHECK_PACKAGES_FAILED: "Failed to check sandbox packages.",
		SANDBOX_BACKEND_UNAVAILABLE: "No sandbox backend available.",
		SANDBOX_IMAGE_NAME_REQUIRED: "Image name is required.",
		SANDBOX_IMAGE_PACKAGES_REQUIRED: "At least one package is required.",
		SANDBOX_IMAGE_NAME_INVALID: "Image name is invalid.",
		SANDBOX_TMP_DIR_CREATE_FAILED: "Failed to create temporary build directory.",
		SANDBOX_DOCKERFILE_WRITE_FAILED: "Failed to write Dockerfile for sandbox image.",
		SANDBOX_IMAGE_BUILD_FAILED: "Failed to build sandbox image.",
	},
	countdown: {
		resetReady: "Limit should be reset now \u2014 try again!",
		resetsIn: "Resets in {{time}}",
	},
	copyFailed: "Copy failed. Copy the link manually.",
};
