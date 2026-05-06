import { sendRpc } from "./helpers";
import type { RpcResponse } from "./types";

const MODEL_SERVICE_NOT_CONFIGURED = "model service not configured";
const MODEL_TEST_RETRY_ATTEMPTS = 40;
const MODEL_TEST_RETRY_DELAY_MS = 250;

interface ProbeResult {
	status?: string;
	error?: string;
}

interface DetectPayload {
	results?: ProbeResult[];
	total?: number;
	supported?: number;
	unsupported?: number;
	errors?: number;
}

interface ValidateKeyPayload {
	valid?: boolean;
	models?: ValidatedModel[];
	error?: string;
}

export interface ValidatedModel {
	id: string;
	displayName: string;
	provider?: string;
	supportsTools?: boolean;
}

export interface ValidateKeyResult {
	valid: boolean;
	models?: ValidatedModel[];
	error?: string;
}

export interface TestModelResult {
	ok: boolean;
	error?: string;
}

export interface ValidateConnectionResult {
	ok: boolean;
	message: string | null;
}

function firstProbeFailure(payload: DetectPayload | undefined): string | null {
	const results = Array.isArray(payload?.results) ? payload?.results : [];
	const failed = results.find((r) => r?.status === "error" || r?.status === "unsupported");
	if (!failed) return null;
	if (typeof failed.error === "string" && failed.error.trim()) {
		return failed.error.trim();
	}
	return null;
}

/**
 * Map raw error strings to user-friendly messages.
 */
export function humanizeProbeError(error: string | null | undefined): string | null | undefined {
	if (!error || typeof error !== "string") return error;
	const lower = error.toLowerCase();

	if (
		lower.includes("401") ||
		lower.includes("unauthorized") ||
		lower.includes("invalid api key") ||
		lower.includes("invalid x-api-key")
	) {
		return "Invalid API key. Please double-check and try again.";
	}
	if (lower.includes("403") || lower.includes("forbidden")) {
		return "Your API key doesn't have access. Check your account permissions.";
	}
	if (lower.includes("permission")) {
		return error;
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
	if (lower.includes("ollama pull")) {
		return error;
	}
	if (lower.includes("404") || lower.includes("not found")) {
		return "Model not found at this endpoint. Make sure it is installed and try again.";
	}

	return error;
}

export function isModelServiceNotConfigured(error: string): boolean {
	if (!error || typeof error !== "string") return false;
	return error.toLowerCase().includes(MODEL_SERVICE_NOT_CONFIGURED);
}

export function isTimeoutError(error: string): boolean {
	if (!error || typeof error !== "string") return false;
	const lower = error.toLowerCase();
	return lower.includes("timeout") || lower.includes("timed out");
}

/**
 * Validate provider credentials without saving them.
 * Returns { valid, models?, error? }.
 */
export async function validateProviderKey(
	provider: string,
	apiKey: string,
	baseUrl: string | null,
	model: string | null,
	requestId?: string,
): Promise<ValidateKeyResult> {
	const payload: Record<string, string> = { provider, apiKey };
	if (baseUrl) payload.baseUrl = baseUrl;
	if (model) payload.model = model;
	if (requestId) payload.requestId = requestId;

	const res = (await sendRpc("providers.validate_key", payload)) as RpcResponse<ValidateKeyPayload>;
	if (!res?.ok) {
		return {
			valid: false,
			error: humanizeProbeError(res?.error?.message || "Failed to validate credentials.") as string,
		};
	}

	const data = res.payload || {};
	if (data.valid) {
		return { valid: true, models: data.models || [] };
	}
	return {
		valid: false,
		error: humanizeProbeError(data.error || "Validation failed.") as string,
	};
}

/**
 * Test a single model from the live registry.
 * Returns { ok, error? }.
 */
export async function testModel(modelId: string): Promise<TestModelResult> {
	for (let attempt = 0; attempt < MODEL_TEST_RETRY_ATTEMPTS; attempt++) {
		const res: RpcResponse = await sendRpc("models.test", { modelId });
		if (res?.ok) {
			return { ok: true };
		}

		const message = res?.error?.message || "Model test failed.";
		const lower = String(message).toLowerCase();
		const shouldRetry = lower.includes(MODEL_SERVICE_NOT_CONFIGURED) && attempt < MODEL_TEST_RETRY_ATTEMPTS - 1;

		if (!shouldRetry) {
			return {
				ok: false,
				error: humanizeProbeError(message) as string,
			};
		}

		await new Promise<void>((resolve) => {
			window.setTimeout(resolve, MODEL_TEST_RETRY_DELAY_MS);
		});
	}

	return {
		ok: false,
		error: humanizeProbeError("Model test failed.") as string,
	};
}

/**
 * Build the payload for a `providers.save_key` RPC call.
 */
export function buildSaveKeyPayload(
	providerName: string,
	apiKey: string,
	baseUrl: string | null,
	model: string | null,
): Record<string, string> {
	const payload: Record<string, string> = { provider: providerName, apiKey };
	if (baseUrl) payload.baseUrl = baseUrl;
	if (model) payload.model = model;
	return payload;
}

/**
 * Persist provider credentials via the `providers.save_key` RPC.
 * Returns the RPC response (check `.ok` for success).
 */
export function saveProviderKey(
	providerName: string,
	apiKey: string,
	baseUrl: string | null,
	model: string | null,
): Promise<RpcResponse> {
	const payload = buildSaveKeyPayload(providerName, apiKey, baseUrl, model);
	return sendRpc("providers.save_key", payload);
}

export async function validateProviderConnection(providerName: string): Promise<ValidateConnectionResult> {
	const res = await sendRpc("models.detect_supported", {
		provider: providerName,
		reason: "provider_credentials_validation",
	});

	if (!res?.ok) {
		return {
			ok: false,
			message: res?.error?.message || "Failed to validate provider credentials.",
		};
	}

	const payload = (res.payload || {}) as DetectPayload;
	const total = payload.total || 0;
	const supported = payload.supported || 0;
	const unsupported = payload.unsupported || 0;
	const errors = payload.errors || 0;

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

	let reason = firstProbeFailure(payload);
	if (!reason) {
		reason = `0/${total} models responded successfully (unsupported: ${unsupported}, errors: ${errors}).`;
	}

	return {
		ok: false,
		message: `Credentials were saved, but validation failed: ${reason}`,
	};
}
