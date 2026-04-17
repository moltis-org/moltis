import { onEvent } from "./events";

export const VALIDATION_HINT_TEXT = "";
export const VALIDATION_HINT_RUNNING_TEXT = "Discovering models...";

const VALIDATION_PROGRESS_EVENT = "providers.validate.progress";

interface ValidationProgressUpdate {
	value: number;
	message: string;
}

interface ValidationEventPayload {
	requestId?: string;
	phase?: string;
	message?: string;
	modelCount?: number;
	totalAttempts?: number;
	attempt?: number;
	modelId?: string;
}

function normalizeAttempt(value: number | undefined, fallback: number): number {
	if (!Number.isFinite(value)) return fallback;
	return Math.max(1, Math.floor(value as number));
}

function stripModelNamespace(modelId: string | undefined): string | undefined {
	if (!modelId || typeof modelId !== "string") return modelId;
	const sep = modelId.lastIndexOf("::");
	return sep >= 0 ? modelId.slice(sep + 2) : modelId;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: maps backend validation phases to progress UI updates.
function progressFromValidationEvent(payload: ValidationEventPayload): ValidationProgressUpdate | null {
	if (!payload?.phase) return null;
	const phase = payload.phase;
	if (phase === "start") {
		return { value: 8, message: payload.message || "Starting provider validation..." };
	}
	if (phase === "candidates_discovered") {
		const count = Number.isFinite(payload.modelCount) ? payload.modelCount : null;
		const message = count == null ? "Discovered candidate models." : `Discovered ${count} candidate models.`;
		return { value: 24, message };
	}
	if (phase === "probe_started" || phase === "probe_failed" || phase === "probe_timeout") {
		const total = normalizeAttempt(payload.totalAttempts, 1);
		const attempt = Math.min(normalizeAttempt(payload.attempt, 1), total);
		const value = 24 + (attempt / total) * 62;
		const modelName = stripModelNamespace(payload.modelId);
		const defaultMessage = modelName
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

export function clampValidationProgressPercent(value: number): number {
	if (!Number.isFinite(value)) return 0;
	return Math.max(0, Math.min(100, value));
}

export function createValidationRequestId(): string {
	const nonce = Math.random().toString(36).slice(2, 10);
	return `validate-${Date.now()}-${nonce}`;
}

export function subscribeValidationProgress(
	requestId: string,
	onProgress: (update: ValidationProgressUpdate, payload: ValidationEventPayload) => void,
): () => void {
	if (!(requestId && typeof onProgress === "function")) {
		return () => undefined;
	}
	const off = onEvent(VALIDATION_PROGRESS_EVENT, (rawPayload: unknown) => {
		const payload = rawPayload as ValidationEventPayload;
		if (!payload || payload.requestId !== requestId) return;
		const update = progressFromValidationEvent(payload);
		if (!update) return;
		onProgress(update, payload);
	});
	return () => {
		off();
	};
}
