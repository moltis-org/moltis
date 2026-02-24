import { onEvent } from "./events.js";

export var VALIDATION_HINT_TEXT = "Validation can take up to 20 seconds for some providers.";
export var VALIDATION_HINT_RUNNING_TEXT = "Validating models... this can take up to 20 seconds.";

var VALIDATION_PROGRESS_EVENT = "providers.validate.progress";

function normalizeAttempt(value, fallback) {
	if (!Number.isFinite(value)) return fallback;
	return Math.max(1, Math.floor(value));
}

function stripModelNamespace(modelId) {
	if (!modelId || typeof modelId !== "string") return modelId;
	var sep = modelId.lastIndexOf("::");
	return sep >= 0 ? modelId.slice(sep + 2) : modelId;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: maps backend validation phases to progress UI updates.
function progressFromValidationEvent(payload) {
	if (!payload?.phase) return null;
	var phase = payload.phase;
	if (phase === "start") {
		return { value: 8, message: payload.message || "Starting provider validation..." };
	}
	if (phase === "candidates_discovered") {
		var count = Number.isFinite(payload.modelCount) ? payload.modelCount : null;
		var message = count == null ? "Discovered candidate models." : `Discovered ${count} candidate models.`;
		return { value: 24, message };
	}
	if (phase === "probe_started" || phase === "probe_failed" || phase === "probe_timeout") {
		var total = normalizeAttempt(payload.totalAttempts, 1);
		var attempt = Math.min(normalizeAttempt(payload.attempt, 1), total);
		var value = 24 + (attempt / total) * 62;
		var modelName = stripModelNamespace(payload.modelId);
		var defaultMessage = modelName
			? `Probing ${modelName} (${attempt}/${total})...`
			: `Probing model ${attempt}/${total}...`;
		return {
			value,
			message: payload.message || defaultMessage,
		};
	}
	if (phase === "probe_succeeded") {
		return { value: 94, message: payload.message || "Model probe succeeded." };
	}
	if (phase === "complete") {
		return { value: 100, message: payload.message || "Validation complete." };
	}
	if (phase === "error") {
		return { value: 98, message: payload.message || "Validation failed." };
	}
	return null;
}

export function clampValidationProgressPercent(value) {
	if (!Number.isFinite(value)) return 0;
	return Math.max(0, Math.min(100, value));
}

export function createValidationRequestId() {
	var nonce = Math.random().toString(36).slice(2, 10);
	return `validate-${Date.now()}-${nonce}`;
}

export function subscribeValidationProgress(requestId, onProgress) {
	if (!(requestId && typeof onProgress === "function")) {
		return () => undefined;
	}
	var off = onEvent(VALIDATION_PROGRESS_EVENT, (payload) => {
		if (!payload || payload.requestId !== requestId) return;
		var update = progressFromValidationEvent(payload);
		if (!update) return;
		onProgress(update, payload);
	});
	return () => {
		off();
	};
}
