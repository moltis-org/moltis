// ── Provider modal shared utilities and state ────────────────

import { onEvent } from "../events";
import { ensureProviderModal } from "../modals";
import * as S from "../state";
import type {
	ProviderInfo,
	ProviderModalElements,
	ValidationEventPayload,
	ValidationProgressState,
	ValidationProgressUpdate,
} from "./types";

// ── Module state ────────────────────────────────────────────

let _els: ProviderModalElements | null = null;

export function els(): ProviderModalElements {
	if (!_els) {
		ensureProviderModal();
		_els = {
			modal: S.$("providerModal")!,
			body: S.$("providerModalBody")!,
			title: S.$("providerModalTitle")!,
			close: S.$("providerModalClose")!,
		};
		_els.close.addEventListener("click", closeProviderModal);
		_els.modal.addEventListener("click", (e: MouseEvent) => {
			if (e.target === _els?.modal) closeProviderModal();
		});
	}
	return _els;
}

// ── Constants ───────────────────────────────────────────────

export const OPENAI_COMPATIBLE_PROVIDERS: string[] = [
	"openai",
	"mistral",
	"openrouter",
	"cerebras",
	"minimax",
	"moonshot",
	"venice",
	"ollama",
];

export const BYOM_PROVIDERS: string[] = ["venice"];
const VALIDATION_HINT_TEXT = "";
const VALIDATION_PROGRESS_EVENT = "providers.validate.progress";

// ── OAuth status timer ──────────────────────────────────────

let oauthStatusTimer: ReturnType<typeof setInterval> | null = null;

export function clearOAuthStatusTimer(): void {
	if (!oauthStatusTimer) return;
	clearInterval(oauthStatusTimer);
	oauthStatusTimer = null;
}

export function setOAuthStatusTimer(timer: ReturnType<typeof setInterval>): void {
	oauthStatusTimer = timer;
}

// ── Modal lifecycle ─────────────────────────────────────────

// Re-export for backwards compat with page-providers.js
export function getProviderModal(): HTMLElement {
	return els().modal;
}

// Lazy import to avoid circular dependency at module level.
// openProviderModal needs showApiKeyForm/showOAuthFlow/showLocalModelFlow/showCustomProviderForm,
// and those modules need openProviderModal for "Back" buttons.
export function openProviderModal(): void {
	// Dynamic import breaks the cycle.
	import("./open-modal").then((mod) => mod.openProviderModalImpl());
}

export function closeProviderModal(): void {
	clearOAuthStatusTimer();
	els().modal.classList.add("hidden");
}

// ── Shared utilities ────────────────────────────────────────

export function setFormError(errorPanel: HTMLElement | null, message: string | null): void {
	if (!errorPanel) return;
	if (!message) {
		errorPanel.style.display = "none";
		errorPanel.textContent = "";
		return;
	}
	errorPanel.textContent = `Error: ${message}`;
	errorPanel.style.display = "";
}

export function normalizeEndpointForCompare(rawUrl: string | null | undefined): string | null {
	if (!rawUrl) return null;
	const trimmed = rawUrl.trim();
	if (!trimmed) return null;
	try {
		const parsed = new URL(trimmed);
		const pathname = parsed.pathname.replace(/\/+$/, "");
		return `${parsed.protocol.toLowerCase()}//${parsed.host.toLowerCase()}${pathname}`;
	} catch {
		return trimmed.replace(/\/+$/, "").toLowerCase();
	}
}

export function shouldUseCustomProviderForOpenAi(
	provider: ProviderInfo | null | undefined,
	endpointVal: string | null | undefined,
): boolean {
	if (provider?.name !== "openai") return false;
	const normalizedEndpoint = normalizeEndpointForCompare(endpointVal);
	if (!normalizedEndpoint) return false;
	const normalizedDefault = normalizeEndpointForCompare(provider.defaultBaseUrl || "https://api.openai.com/v1");
	return normalizedDefault !== null && normalizedEndpoint !== normalizedDefault;
}

export function stripModelNamespace(modelId: string | null | undefined): string {
	if (!modelId || typeof modelId !== "string") return modelId || "";
	const sep = modelId.lastIndexOf("::");
	return sep >= 0 ? modelId.slice(sep + 2) : modelId;
}

// ── Validation progress helpers ─────────────────────────────

export function createValidationProgress(form: HTMLElement, marginClass?: string): ValidationProgressState {
	const wrapper = document.createElement("div");
	wrapper.className = `flex flex-col gap-2 ${marginClass || "mt-2"}`;

	const progress = document.createElement("div");
	progress.className = "download-progress";

	const progressBar = document.createElement("div");
	progressBar.className = "download-progress-bar";
	progressBar.style.width = "0%";
	progress.appendChild(progressBar);
	wrapper.appendChild(progress);

	const progressText = document.createElement("div");
	progressText.className = "text-xs text-[var(--muted)]";
	progressText.textContent = VALIDATION_HINT_TEXT;
	wrapper.appendChild(progressText);

	form.appendChild(wrapper);

	return {
		progress,
		progressBar,
		progressText,
		value: 0,
	};
}

function clampProgressPercent(value: number): number {
	if (!Number.isFinite(value)) return 0;
	return Math.max(0, Math.min(100, value));
}

export function setValidationProgress(state: ValidationProgressState | null, value: number, message?: string): void {
	if (!state) return;
	const next = clampProgressPercent(value);
	state.value = Math.max(state.value, next);
	state.progress.classList.remove("indeterminate");
	state.progressBar.style.width = `${state.value.toFixed(1)}%`;
	if (message) {
		state.progressText.textContent = message;
	}
}

export function resetValidationProgress(state: ValidationProgressState | null): void {
	if (!state) return;
	state.value = 0;
	state.progress.classList.remove("indeterminate");
	state.progressBar.style.width = "0%";
	state.progressText.textContent = VALIDATION_HINT_TEXT;
}

export function completeValidationProgress(state: ValidationProgressState | null, text?: string): void {
	if (!state) return;
	setValidationProgress(state, 100, text || "Validation complete.");
}

export function createValidationRequestId(): string {
	const nonce = Math.random().toString(36).slice(2, 10);
	return `validate-${Date.now()}-${nonce}`;
}

function normalizeAttempt(value: number | undefined, fallback: number): number {
	if (!Number.isFinite(value)) return fallback;
	return Math.max(1, Math.floor(value!));
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: maps backend validation phases to progress UI updates.
function progressFromValidationEvent(
	payload: ValidationEventPayload | null | undefined,
): ValidationProgressUpdate | null {
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

export function bindValidationProgressEvents(
	state: ValidationProgressState | null,
	requestId: string | undefined,
): () => void {
	if (!(state && requestId)) return () => undefined;
	const off = onEvent(VALIDATION_PROGRESS_EVENT, (payload: unknown) => {
		const p = payload as ValidationEventPayload;
		if (!p || p.requestId !== requestId) return;
		const update = progressFromValidationEvent(p);
		if (!update) return;
		setValidationProgress(state, update.value, update.message);
	});
	return () => {
		off();
	};
}
