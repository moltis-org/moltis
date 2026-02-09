import { sendRpc } from "./helpers.js";

function firstProbeFailure(payload) {
	var results = Array.isArray(payload?.results) ? payload.results : [];
	var failed = results.find((r) => r?.status === "error" || r?.status === "unsupported");
	if (!failed) return null;
	if (typeof failed.error === "string" && failed.error.trim()) {
		return failed.error.trim();
	}
	return null;
}

/**
 * Map raw error strings to user-friendly messages.
 */
export function humanizeProbeError(error) {
	if (!error || typeof error !== "string") return error;
	var lower = error.toLowerCase();

	if (
		lower.includes("401") ||
		lower.includes("unauthorized") ||
		lower.includes("invalid api key") ||
		lower.includes("invalid x-api-key")
	) {
		return "Invalid API key. Please double-check and try again.";
	}
	if (lower.includes("403") || lower.includes("forbidden") || lower.includes("permission")) {
		return "Your API key doesn't have access. Check your account permissions.";
	}
	if (lower.includes("429") || lower.includes("rate limit") || lower.includes("too many requests")) {
		return "Rate limited by the provider. Wait a moment and try again.";
	}
	if (lower.includes("timeout") || lower.includes("timed out")) {
		return "Connection timed out. Check your endpoint URL and try again.";
	}
	if (lower.includes("connection refused") || lower.includes("econnrefused")) {
		return "Connection refused. Make sure the provider endpoint is running and reachable.";
	}
	if (lower.includes("dns") || lower.includes("getaddrinfo") || lower.includes("name or service not known")) {
		return "Could not resolve the endpoint address. Check the URL and try again.";
	}

	return error;
}

/**
 * Validate provider credentials without saving them.
 * Returns { valid, models?, error? }.
 */
export async function validateProviderKey(provider, apiKey, baseUrl, model) {
	var payload = { provider, apiKey };
	if (baseUrl) payload.baseUrl = baseUrl;
	if (model) payload.model = model;

	var res = await sendRpc("providers.validate_key", payload);
	if (!res?.ok) {
		return {
			valid: false,
			error: humanizeProbeError(res?.error?.message || "Failed to validate credentials."),
		};
	}

	var data = res.payload || {};
	if (data.valid) {
		return { valid: true, models: data.models || [] };
	}
	return {
		valid: false,
		error: humanizeProbeError(data.error || "Validation failed."),
	};
}

/**
 * Test a single model from the live registry.
 * Returns { ok, error? }.
 */
export async function testModel(modelId) {
	var res = await sendRpc("models.test", { modelId });
	if (res?.ok) {
		return { ok: true };
	}
	return {
		ok: false,
		error: humanizeProbeError(res?.error?.message || "Model test failed."),
	};
}

export async function validateProviderConnection(providerName) {
	var res = await sendRpc("models.detect_supported", {
		provider: providerName,
		reason: "provider_credentials_validation",
	});

	if (!res?.ok) {
		return {
			ok: false,
			message: res?.error?.message || "Failed to validate provider credentials.",
		};
	}

	var payload = res.payload || {};
	var total = payload.total || 0;
	var supported = payload.supported || 0;
	var unsupported = payload.unsupported || 0;
	var errors = payload.errors || 0;

	if (supported > 0) {
		return {
			ok: true,
			message: null,
		};
	}

	// No probe targets usually means no model is configured yet.
	if (total === 0) {
		return {
			ok: true,
			message: null,
		};
	}

	var reason = firstProbeFailure(payload);
	if (!reason) {
		reason = `0/${total} models responded successfully (unsupported: ${unsupported}, errors: ${errors}).`;
	}

	return {
		ok: false,
		message: `Credentials were saved, but validation failed: ${reason}`,
	};
}
