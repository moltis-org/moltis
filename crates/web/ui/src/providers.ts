// ── Provider modal ──────────────────────────────────────

import { onEvent } from "./events";
import { modelVersionScore, sendRpc } from "./helpers";
import { ensureProviderModal } from "./modals";
import { fetchModels } from "./models";
import { providerApiKeyHelp } from "./provider-key-help";
import { completeProviderOAuth, startProviderOAuth } from "./provider-oauth";
import {
	humanizeProbeError,
	isModelServiceNotConfigured,
	isTimeoutError,
	saveProviderKey,
	testModel,
	validateProviderKey,
} from "./provider-validation";
import type { TestModelResult } from "./provider-validation";
import type { RpcResponse } from "./types";
import * as S from "./state";

// ── Interfaces ──────────────────────────────────────────────

interface ProviderModalElements {
	modal: HTMLElement;
	body: HTMLElement;
	title: HTMLElement;
	close: HTMLElement;
}

interface ProviderInfo {
	name: string;
	displayName: string;
	authType: string;
	keyOptional?: boolean;
	configured?: boolean;
	isCustom?: boolean;
	uiOrder?: number;
	defaultBaseUrl?: string;
	models?: string[];
}

interface ModelEntry {
	id: string;
	displayName: string;
	provider?: string;
	supportsTools?: boolean;
	createdAt?: number;
}

interface ValidationProgressState {
	progress: HTMLElement;
	progressBar: HTMLElement;
	progressText: HTMLElement;
	value: number;
}

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

interface AddCustomPayload {
	providerName: string;
	displayName: string;
}

interface SystemInfo {
	totalRamGb: number;
	memoryTier: string;
	hasGpu: boolean;
	isAppleSilicon: boolean;
	recommendedBackend: string;
	availableBackends: BackendInfo[];
	backendNote?: string;
}

interface BackendInfo {
	id: string;
	name: string;
	description: string;
	available: boolean;
	installCommands?: string[];
}

interface ModelsData {
	recommended: LocalModelInfo[];
}

interface LocalModelInfo {
	id: string;
	displayName: string;
	backend: string;
	minRamGb: number;
	contextWindow: number;
	suggested?: boolean;
}

interface HfSearchResult {
	id: string;
	displayName: string;
	downloads?: number;
	likes?: number;
	backend: string;
}

interface LocalLlmDownloadPayload {
	modelId?: string;
	error?: string;
	complete?: boolean;
	progress?: number;
	downloaded?: number;
	total?: number;
}

interface ProbeResult {
	error?: string;
	timeout?: boolean;
}

// Model selector wrapper with attached properties
interface ModelSelectorWrapper extends HTMLElement {
	_errorArea?: HTMLElement;
	_resetSelection?: () => void;
	_renderModelsForBackend?: (backend: string) => void;
	_updateFilenameVisibility?: (backend: string) => void;
}

// ── Module state ────────────────────────────────────────────

let _els: ProviderModalElements | null = null;

function els(): ProviderModalElements {
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

// Re-export for backwards compat with page-providers.js
export function getProviderModal(): HTMLElement {
	return els().modal;
}

// Providers that support custom endpoint configuration
const OPENAI_COMPATIBLE_PROVIDERS: string[] = [
	"openai",
	"mistral",
	"openrouter",
	"cerebras",
	"minimax",
	"moonshot",
	"venice",
	"ollama",
];

const BYOM_PROVIDERS: string[] = ["venice"];
const VALIDATION_HINT_TEXT = "";
const VALIDATION_PROGRESS_EVENT = "providers.validate.progress";
let oauthStatusTimer: ReturnType<typeof setInterval> | null = null;

function clearOAuthStatusTimer(): void {
	if (!oauthStatusTimer) return;
	clearInterval(oauthStatusTimer);
	oauthStatusTimer = null;
}

function normalizeEndpointForCompare(rawUrl: string | null | undefined): string | null {
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

function shouldUseCustomProviderForOpenAi(provider: ProviderInfo | null | undefined, endpointVal: string | null | undefined): boolean {
	if (provider?.name !== "openai") return false;
	const normalizedEndpoint = normalizeEndpointForCompare(endpointVal);
	if (!normalizedEndpoint) return false;
	const normalizedDefault = normalizeEndpointForCompare(provider.defaultBaseUrl || "https://api.openai.com/v1");
	return normalizedDefault !== null && normalizedEndpoint !== normalizedDefault;
}

function stripModelNamespace(modelId: string | null | undefined): string {
	if (!modelId || typeof modelId !== "string") return modelId || "";
	const sep = modelId.lastIndexOf("::");
	return sep >= 0 ? modelId.slice(sep + 2) : modelId;
}

export function openProviderModal(): void {
	const m = els();
	m.modal.classList.remove("hidden");
	m.title.textContent = "Add LLM";
	m.body.textContent = "Loading...";
	sendRpc<ProviderInfo[]>("providers.available", {}).then((res: RpcResponse<ProviderInfo[]>) => {
		if (!res?.ok) {
			m.body.textContent = "Failed to load LLM providers.";
			return;
		}
		const providers: ProviderInfo[] = (res.payload as ProviderInfo[]) || [];

		providers.sort((a: ProviderInfo, b: ProviderInfo) => {
			const aOrder = Number.isFinite(a.uiOrder) ? a.uiOrder! : Number.MAX_SAFE_INTEGER;
			const bOrder = Number.isFinite(b.uiOrder) ? b.uiOrder! : Number.MAX_SAFE_INTEGER;
			if (aOrder !== bOrder) return aOrder - bOrder;
			return a.displayName.localeCompare(b.displayName);
		});

		m.body.textContent = "";
		providers.forEach((p: ProviderInfo) => {
			const item = document.createElement("div");
			// Don't gray out configured providers - users can add multiple
			item.className = "provider-item";
			const name = document.createElement("span");
			name.className = "provider-item-name";
			name.textContent = p.displayName;
			item.appendChild(name);

			const badges = document.createElement("div");
			badges.className = "badge-row";

			if (p.configured) {
				const check = document.createElement("span");
				check.className = "provider-item-badge configured";
				check.textContent = "configured";
				badges.appendChild(check);
			}

			if (p.isCustom) {
				const customBadge = document.createElement("span");
				customBadge.className = "provider-item-badge api-key";
				customBadge.textContent = "Custom";
				badges.appendChild(customBadge);
			} else {
				const badge = document.createElement("span");
				badge.className = `provider-item-badge ${p.authType}`;
				if (p.authType === "oauth") {
					badge.textContent = "OAuth";
				} else if (p.authType === "local") {
					badge.textContent = "Local";
				} else {
					badge.textContent = "API Key";
				}
				badges.appendChild(badge);
			}
			item.appendChild(badges);

			item.addEventListener("click", () => {
				if (p.authType === "api-key") showApiKeyForm(p);
				else if (p.authType === "oauth") showOAuthFlow(p);
				else if (p.authType === "local") showLocalModelFlow(p);
			});
			m.body.appendChild(item);
		});

		// Separator + "OpenAI Compatible" entry
		const separator = document.createElement("div");
		separator.className = "border-t border-[var(--border)] my-2";
		m.body.appendChild(separator);

		const customItem = document.createElement("div");
		customItem.className = "provider-item";

		const customName = document.createElement("span");
		customName.className = "provider-item-name";
		customName.textContent = "OpenAI Compatible";
		customItem.appendChild(customName);

		const customBadges = document.createElement("div");
		customBadges.className = "badge-row";
		const anyBadge = document.createElement("span");
		anyBadge.className = "provider-item-badge api-key";
		anyBadge.textContent = "Any Endpoint";
		customBadges.appendChild(anyBadge);
		customItem.appendChild(customBadges);

		customItem.addEventListener("click", showCustomProviderForm);
		m.body.appendChild(customItem);
	});
}

export function closeProviderModal(): void {
	clearOAuthStatusTimer();
	els().modal.classList.add("hidden");
}

function setFormError(errorPanel: HTMLElement | null, message: string | null): void {
	if (!errorPanel) return;
	if (!message) {
		errorPanel.style.display = "none";
		errorPanel.textContent = "";
		return;
	}
	errorPanel.textContent = `Error: ${message}`;
	errorPanel.style.display = "";
}

function createValidationProgress(form: HTMLElement, marginClass?: string): ValidationProgressState {
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

function setValidationProgress(state: ValidationProgressState | null, value: number, message?: string): void {
	if (!state) return;
	const next = clampProgressPercent(value);
	state.value = Math.max(state.value, next);
	state.progress.classList.remove("indeterminate");
	state.progressBar.style.width = `${state.value.toFixed(1)}%`;
	if (message) {
		state.progressText.textContent = message;
	}
}

function resetValidationProgress(state: ValidationProgressState | null): void {
	if (!state) return;
	state.value = 0;
	state.progress.classList.remove("indeterminate");
	state.progressBar.style.width = "0%";
	state.progressText.textContent = VALIDATION_HINT_TEXT;
}

function completeValidationProgress(state: ValidationProgressState | null, text?: string): void {
	if (!state) return;
	setValidationProgress(state, 100, text || "Validation complete.");
}

function createValidationRequestId(): string {
	const nonce = Math.random().toString(36).slice(2, 10);
	return `validate-${Date.now()}-${nonce}`;
}

function normalizeAttempt(value: number | undefined, fallback: number): number {
	if (!Number.isFinite(value)) return fallback;
	return Math.max(1, Math.floor(value!));
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: maps backend validation phases to progress UI updates.
function progressFromValidationEvent(payload: ValidationEventPayload | null | undefined): ValidationProgressUpdate | null {
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

function bindValidationProgressEvents(state: ValidationProgressState | null, requestId: string | undefined): () => void {
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

function showCustomProviderForm(): void {
	const m = els();
	m.title.textContent = "OpenAI Compatible";
	m.body.textContent = "";

	const form = document.createElement("div");
	form.className = "provider-key-form";

	// Endpoint URL
	const urlLabel = document.createElement("label");
	urlLabel.className = "text-xs text-[var(--muted)]";
	urlLabel.textContent = "Endpoint URL";
	form.appendChild(urlLabel);

	const urlInp = document.createElement("input");
	urlInp.className = "provider-key-input";
	urlInp.type = "text";
	urlInp.placeholder = "https://api.example.com/v1";
	form.appendChild(urlInp);

	// API Key
	const keyLabel = document.createElement("label");
	keyLabel.className = "text-xs text-[var(--muted)] mt-2";
	keyLabel.textContent = "API Key";
	form.appendChild(keyLabel);

	const keyInp = document.createElement("input");
	keyInp.className = "provider-key-input";
	keyInp.type = "password";
	keyInp.placeholder = "sk-...";
	form.appendChild(keyInp);

	// Model ID (optional)
	const modelLabel = document.createElement("label");
	modelLabel.className = "text-xs text-[var(--muted)] mt-2";
	modelLabel.textContent = "Model ID (optional)";
	form.appendChild(modelLabel);

	const modelInp = document.createElement("input");
	modelInp.className = "provider-key-input";
	modelInp.type = "text";
	modelInp.placeholder = "Leave blank for auto-discovery";
	form.appendChild(modelInp);

	const errorPanel = document.createElement("div");
	errorPanel.className = "alert-error-text text-[var(--error)] whitespace-pre-line";
	errorPanel.style.display = "none";
	form.appendChild(errorPanel);

	const validationProgress = createValidationProgress(form, "mt-1");

	const btns = document.createElement("div");
	btns.className = "btn-row";
	btns.style.marginTop = "12px";

	const backBtn = document.createElement("button");
	backBtn.className = "provider-btn provider-btn-secondary";
	backBtn.textContent = "Back";
	backBtn.addEventListener("click", openProviderModal);
	btns.appendChild(backBtn);

	const saveBtn = document.createElement("button");
	saveBtn.className = "provider-btn";
	saveBtn.textContent = "Add Provider";
	saveBtn.addEventListener("click", () => {
		const url = urlInp.value.trim();
		const key = keyInp.value.trim();
		const model = modelInp.value.trim() || null;

		if (!url) {
			setFormError(errorPanel, "Endpoint URL is required.");
			return;
		}
		if (!key) {
			setFormError(errorPanel, "API key is required.");
			return;
		}

		saveBtn.disabled = true;
		saveBtn.textContent = "Adding...";
		setValidationProgress(validationProgress, 8, "Saving provider settings...");
		setFormError(errorPanel, null);

		sendRpc<AddCustomPayload>("providers.add_custom", { baseUrl: url, apiKey: key, model: model })
			.then((res: RpcResponse<AddCustomPayload>) => {
				if (!res?.ok) {
					saveBtn.disabled = false;
					saveBtn.textContent = "Add Provider";
					resetValidationProgress(validationProgress);
					setFormError(errorPanel, res?.error?.message || "Failed to add provider.");
					return;
				}
				const result = res.payload as AddCustomPayload;
				const providerName = result.providerName;
				const displayName = result.displayName;
				const requestId = createValidationRequestId();
				setValidationProgress(validationProgress, 12, "Discovering models...");
				const stopProgressEvents = bindValidationProgressEvents(validationProgress, requestId);

				// Validate the provider to discover models
				validateProviderKey(providerName, key, url, model, requestId)
					.then((valResult) => {
						if (!(valResult.valid || model)) {
							saveBtn.disabled = false;
							saveBtn.textContent = "Add Provider";
							resetValidationProgress(validationProgress);
							setFormError(errorPanel, valResult.error || "No models discovered. Please specify a model ID.");
							return;
						}

						if (valResult.models && valResult.models.length > 0) {
							completeValidationProgress(validationProgress, "Done.");
							// Show model selector
							const customProvider: ProviderInfo = {
								name: providerName,
								displayName: displayName,
								authType: "api-key",
								keyOptional: false,
								isCustom: true,
							};
							showModelSelector(customProvider, valResult.models as ModelEntry[], key, url, model, true);
						} else if (model) {
							// Model specified manually -- save it and finish
							sendRpc("providers.save_model", { provider: providerName, model: model }).then(() => {
								completeValidationProgress(validationProgress, "Done.");
								fetchModels();
								if (S.refreshProvidersPage) S.refreshProvidersPage();
								m.body.textContent = "";
								const status = document.createElement("div");
								status.className = "provider-status";
								status.textContent = `${displayName} configured successfully!`;
								m.body.appendChild(status);
								setTimeout(closeProviderModal, 1500);
							});
						} else {
							saveBtn.disabled = false;
							saveBtn.textContent = "Add Provider";
							resetValidationProgress(validationProgress);
							setFormError(errorPanel, "No models discovered. Please specify a model ID.");
						}
					})
					.catch((err: Error) => {
						saveBtn.disabled = false;
						saveBtn.textContent = "Add Provider";
						resetValidationProgress(validationProgress);
						setFormError(errorPanel, err?.message || "Validation failed.");
					})
					.finally(() => {
						stopProgressEvents();
					});
			})
			.catch((err: Error) => {
				saveBtn.disabled = false;
				saveBtn.textContent = "Add Provider";
				resetValidationProgress(validationProgress);
				setFormError(errorPanel, err?.message || "Failed to add provider.");
			});
	});
	btns.appendChild(saveBtn);
	form.appendChild(btns);
	m.body.appendChild(form);
	urlInp.focus();
}

export function showApiKeyForm(provider: ProviderInfo): void {
	const m = els();
	m.title.textContent = provider.displayName;
	m.body.textContent = "";

	const form = document.createElement("div");
	form.className = "provider-key-form";

	// Check if this provider supports custom endpoint
	const supportsEndpoint = OPENAI_COMPATIBLE_PROVIDERS.includes(provider.name);

	// API Key field
	const keyLabel = document.createElement("label");
	keyLabel.className = "text-xs text-[var(--muted)]";
	keyLabel.textContent = "API Key";
	form.appendChild(keyLabel);

	const keyInp = document.createElement("input");
	keyInp.className = "provider-key-input";
	keyInp.type = "password";
	keyInp.placeholder = provider.keyOptional ? "(optional)" : "sk-...";
	form.appendChild(keyInp);

	const errorPanel = document.createElement("div");
	errorPanel.className = "alert-error-text text-[var(--error)] whitespace-pre-line";
	errorPanel.style.display = "none";
	form.appendChild(errorPanel);

	const keyHelp = providerApiKeyHelp(provider as Parameters<typeof providerApiKeyHelp>[0]);
	if (keyHelp) {
		const keyHelpLine = document.createElement("div");
		keyHelpLine.className = "text-xs text-[var(--muted)] mt-1";
		if (keyHelp.url) {
			keyHelpLine.append(`${keyHelp.text} `);
			const keyLink = document.createElement("a");
			keyLink.href = keyHelp.url;
			keyLink.target = "_blank";
			keyLink.rel = "noopener noreferrer";
			keyLink.className = "text-[var(--accent)] underline";
			keyLink.textContent = keyHelp.label || keyHelp.url;
			keyHelpLine.appendChild(keyLink);
		} else {
			keyHelpLine.textContent = keyHelp.text;
		}
		form.appendChild(keyHelpLine);
	}

	// Endpoint field for OpenAI-compatible providers
	let endpointInp: HTMLInputElement | null = null;
	if (supportsEndpoint) {
		const endpointLabel = document.createElement("label");
		endpointLabel.className = "text-xs text-[var(--muted)]";
		endpointLabel.style.marginTop = "8px";
		endpointLabel.textContent = "Endpoint (optional)";
		form.appendChild(endpointLabel);

		endpointInp = document.createElement("input");
		endpointInp.className = "provider-key-input";
		endpointInp.type = "text";
		endpointInp.placeholder = provider.defaultBaseUrl || "https://api.example.com/v1";
		form.appendChild(endpointInp);

		const hint = document.createElement("div");
		hint.className = "text-xs text-[var(--muted)]";
		hint.style.marginTop = "2px";
		hint.textContent = "Leave empty to use the default endpoint.";
		form.appendChild(hint);
	}

	// Model field for bring-your-own-model providers
	let modelInp: HTMLInputElement | null = null;
	const needsModel = BYOM_PROVIDERS.includes(provider.name);
	if (needsModel) {
		const modelLabel = document.createElement("label");
		modelLabel.className = "text-xs text-[var(--muted)]";
		modelLabel.style.marginTop = "8px";
		modelLabel.textContent = "Model ID";
		form.appendChild(modelLabel);

		modelInp = document.createElement("input");
		modelInp.className = "provider-key-input";
		modelInp.type = "text";
		modelInp.placeholder = "model-id";
		form.appendChild(modelInp);
	}

	const validationProgress = createValidationProgress(form, "mt-2");

	const btns = document.createElement("div");
	btns.className = "btn-row";
	btns.style.marginTop = "12px";

	const backBtn = document.createElement("button");
	backBtn.className = "provider-btn provider-btn-secondary";
	backBtn.textContent = "Back";
	backBtn.addEventListener("click", openProviderModal);
	btns.appendChild(backBtn);

	const saveBtn = document.createElement("button");
	saveBtn.className = "provider-btn";
	saveBtn.textContent = "Save";
	saveBtn.addEventListener("click", () => {
		const key = keyInp.value.trim();
		if (!(key || provider.keyOptional)) {
			setFormError(errorPanel, "API key is required.");
			return;
		}

		// Model is required for bring-your-own providers
		if (needsModel && modelInp && !modelInp.value.trim()) {
			setFormError(errorPanel, "Model ID is required.");
			return;
		}

		saveBtn.disabled = true;
		saveBtn.textContent = "Saving...";
		setValidationProgress(validationProgress, 10, "Discovering models...");
		setFormError(errorPanel, null);

		const keyVal = key || provider.name;
		const endpointVal = endpointInp?.value.trim() || null;
		const modelVal = modelInp?.value.trim() || null;
		const requestId = createValidationRequestId();
		const stopProgressEvents = bindValidationProgressEvents(validationProgress, requestId);

		validateProviderKey(provider.name, keyVal, endpointVal, modelVal, requestId)
			.then((result) => {
				if (!result.valid) {
					saveBtn.disabled = false;
					saveBtn.textContent = "Save";
					resetValidationProgress(validationProgress);
					setFormError(errorPanel, result.error || "Failed to connect. Please check your credentials.");
					return;
				}

				// BYOM providers already tested the specific model -- save directly.
				if (needsModel) {
					completeValidationProgress(validationProgress, "Done.");
					saveAndFinishProvider(provider, keyVal, endpointVal, modelVal, null, false);
					return;
				}

				// Regular providers -- show model selector.
				const models: ModelEntry[] = (result.models || []) as ModelEntry[];
				completeValidationProgress(validationProgress, "Done.");
				showModelSelector(provider, models, keyVal, endpointVal, modelVal);
			})
			.catch((err: Error) => {
				saveBtn.disabled = false;
				saveBtn.textContent = "Save";
				resetValidationProgress(validationProgress);
				setFormError(errorPanel, err?.message || "Failed to connect.");
			})
			.finally(() => {
				stopProgressEvents();
			});
	});
	btns.appendChild(saveBtn);
	form.appendChild(btns);
	m.body.appendChild(form);
	keyInp.focus();
}

function showModelSelector(provider: ProviderInfo, models: ModelEntry[], keyVal: string | null, endpointVal: string | null, modelVal: string | null, skipSave?: boolean): void {
	const m = els();
	m.title.textContent = `${provider.displayName} \u2014 Select Models`;
	m.body.textContent = "";

	const selectedIds: Set<string> = new Set();

	const wrapper = document.createElement("div") as ModelSelectorWrapper;
	wrapper.className = "provider-key-form flex flex-col min-h-0 flex-1";

	const label = document.createElement("div");
	label.className = "text-xs font-medium text-[var(--text-strong)] mb-1 shrink-0";
	label.textContent = "Select models to add";
	wrapper.appendChild(label);

	const hint = document.createElement("div");
	hint.className = "text-xs text-[var(--muted)] mb-2 shrink-0";
	hint.textContent = "Click models to toggle selection, or use Select All.";
	wrapper.appendChild(hint);

	// Search + Select All row when >5 models
	let searchInp: HTMLInputElement | null = null;
	if (models.length > 5) {
		searchInp = document.createElement("input");
		searchInp.type = "text";
		searchInp.className = "provider-key-input w-full text-xs mb-2 shrink-0";
		searchInp.placeholder = "Search models\u2026";
		wrapper.appendChild(searchInp);
	}

	const selectAllBtn = document.createElement("button");
	selectAllBtn.className = "provider-btn provider-btn-secondary text-xs mb-2 shrink-0";

	function getVisibleModels(): ModelEntry[] {
		const currentFilter = searchInp?.value.trim() || null;
		if (!currentFilter) return models;
		const q = currentFilter.toLowerCase();
		return models.filter((mdl: ModelEntry) => mdl.displayName.toLowerCase().includes(q) || mdl.id.toLowerCase().includes(q));
	}

	function updateSelectAllLabel(): void {
		const visible = getVisibleModels();
		const allVisible = visible.length > 0 && visible.every((mdl: ModelEntry) => selectedIds.has(mdl.id));
		selectAllBtn.textContent = allVisible ? "Deselect All" : "Select All";
	}
	updateSelectAllLabel();

	selectAllBtn.addEventListener("click", () => {
		const visible = getVisibleModels();
		const allVisible = visible.every((mdl: ModelEntry) => selectedIds.has(mdl.id));
		if (allVisible) {
			for (const mdl of visible) selectedIds.delete(mdl.id);
		} else {
			for (const visibleModel of visible) selectedIds.add(visibleModel.id);
		}
		updateSelectAllLabel();
		updateStatus();
		renderCards(searchInp?.value.trim() || null);
	});
	wrapper.appendChild(selectAllBtn);

	const list = document.createElement("div");
	list.className = "flex flex-col gap-1 overflow-y-auto flex-1 min-h-0 max-h-56";
	wrapper.appendChild(list);

	const statusArea = document.createElement("div");
	statusArea.className = "text-xs text-[var(--muted)] mt-2 shrink-0";
	wrapper.appendChild(statusArea);

	function updateStatus(): void {
		const count = selectedIds.size;
		statusArea.textContent = count === 0 ? "No models selected" : `${count} model${count > 1 ? "s" : ""} selected`;
	}

	const errorArea = document.createElement("div");
	errorArea.className = "alert-error-text text-[var(--error)] whitespace-pre-line shrink-0";
	errorArea.style.display = "none";
	wrapper.appendChild(errorArea);

	function renderCards(filter: string | null): void {
		list.textContent = "";
		let filtered = models;
		if (filter) {
			const q = filter.toLowerCase();
			filtered = models.filter((mdl: ModelEntry) => mdl.displayName.toLowerCase().includes(q) || mdl.id.toLowerCase().includes(q));
		}
		if (filtered.length === 0) {
			const empty = document.createElement("div");
			empty.className = "text-xs text-[var(--muted)] py-4 text-center";
			empty.textContent = "No models match your search.";
			list.appendChild(empty);
			return;
		}
		filtered.forEach((mdl: ModelEntry) => {
			const card = document.createElement("div");
			card.className = `model-card ${selectedIds.has(mdl.id) ? "selected" : ""}`;

			const header = document.createElement("div");
			header.className = "flex items-center justify-between";

			const name = document.createElement("span");
			name.className = "text-sm font-medium text-[var(--text)]";
			name.textContent = mdl.displayName;
			header.appendChild(name);

			const badges = document.createElement("div");
			badges.className = "flex gap-2";

			if (mdl.supportsTools) {
				const toolsBadge = document.createElement("span");
				toolsBadge.className = "recommended-badge";
				toolsBadge.textContent = "Tools";
				badges.appendChild(toolsBadge);
			}

			header.appendChild(badges);
			card.appendChild(header);

			const idLine = document.createElement("div");
			idLine.className = "text-xs text-[var(--muted)] mt-1 font-mono";
			idLine.textContent = mdl.id;
			card.appendChild(idLine);

			((modelId: string) => {
				card.addEventListener("click", () => {
					if (selectedIds.has(modelId)) {
						selectedIds.delete(modelId);
					} else {
						selectedIds.add(modelId);
					}
					updateSelectAllLabel();
					updateStatus();
					renderCards(searchInp?.value.trim() || null);
				});
			})(mdl.id);

			list.appendChild(card);
		});
	}

	renderCards(null);
	updateStatus();

	if (searchInp) {
		searchInp.addEventListener("input", () => {
			renderCards(searchInp?.value.trim());
		});
	}

	// Buttons
	const btns = document.createElement("div");
	btns.className = "btn-row mt-3 shrink-0";

	const backBtn = document.createElement("button");
	backBtn.className = "provider-btn provider-btn-secondary";
	backBtn.textContent = "Back";
	backBtn.addEventListener("click", () => {
		if (skipSave) {
			openProviderModal();
		} else {
			showApiKeyForm(provider);
		}
	});
	btns.appendChild(backBtn);

	const continueBtn = document.createElement("button");
	continueBtn.className = "provider-btn";
	continueBtn.textContent = "Continue";
	continueBtn.addEventListener("click", () => {
		if (selectedIds.size === 0) {
			errorArea.textContent = "Select at least one model to continue.";
			errorArea.style.display = "";
			return;
		}
		errorArea.style.display = "none";
		continueBtn.disabled = true;
		continueBtn.textContent = "Saving\u2026";
		saveAndFinishProvider(provider, keyVal, endpointVal, modelVal, Array.from(selectedIds), !!skipSave);
	});
	btns.appendChild(continueBtn);

	wrapper.appendChild(btns);

	// Expose error area for saveAndFinishProvider to use
	wrapper._errorArea = errorArea;
	wrapper._resetSelection = () => {
		continueBtn.disabled = false;
		continueBtn.textContent = "Continue";
		renderCards(searchInp?.value.trim() || null);
	};

	m.body.appendChild(wrapper);
}

function saveAndFinishProvider(provider: ProviderInfo, keyVal: string | null, endpointVal: string | null, modelVal: string | null, selectedModelIds: string[] | null, skipSave: boolean): void {
	// selectedModelIds can be a single string (legacy callers) or an array
	const modelIds: string[] = Array.isArray(selectedModelIds) ? selectedModelIds : selectedModelIds ? [selectedModelIds] : [];

	const m = els();
	const saveAsCustomProvider = !skipSave && shouldUseCustomProviderForOpenAi(provider, endpointVal);

	const modelsForSave = saveAsCustomProvider ? modelIds.map(stripModelNamespace) : [...modelIds];
	const firstModelForSave = modelsForSave[0] || null;
	const effectiveModelVal = provider.keyOptional && firstModelForSave ? firstModelForSave : modelVal;

	function showError(msg: string): void {
		const wrapperEl = m.body.querySelector(".provider-key-form") as ModelSelectorWrapper | null;
		if (wrapperEl?._errorArea) {
			setFormError(wrapperEl._errorArea, msg);
			if (wrapperEl._resetSelection) wrapperEl._resetSelection();
		}
	}

	let savePromise: Promise<RpcResponse>;
	if (skipSave) {
		savePromise = Promise.resolve({ ok: true });
	} else if (saveAsCustomProvider) {
		const customPayload: Record<string, string | null> = { baseUrl: endpointVal, apiKey: keyVal };
		if (firstModelForSave) customPayload.model = firstModelForSave;
		savePromise = sendRpc("providers.add_custom", customPayload);
	} else {
		savePromise = saveProviderKey(provider.name, keyVal || "", endpointVal, effectiveModelVal);
	}

	savePromise
		.then(async (res: RpcResponse) => {
			if (!res?.ok) {
				showError(res?.error?.message || "Failed to save credentials.");
				return;
			}
			const savedProviderName = saveAsCustomProvider ? (res?.payload as AddCustomPayload)?.providerName || provider.name : provider.name;
			const successDisplayName = saveAsCustomProvider
				? (res?.payload as AddCustomPayload)?.displayName || provider.displayName
				: provider.displayName;

			let modelTimedOut = false;
			if (modelIds.length > 0) {
				// Test first model as a connectivity check
				const firstModelId = modelIds[0];
				const firstModelForTest = saveAsCustomProvider ? `${savedProviderName}::${modelsForSave[0]}` : firstModelId;
				const testResult: TestModelResult = await testModel(firstModelForTest);
				const modelServiceUnavailable = !testResult.ok && isModelServiceNotConfigured(testResult.error || "");
				modelTimedOut = !testResult.ok && isTimeoutError(testResult.error || "");
				if (!(testResult.ok || modelServiceUnavailable || modelTimedOut)) {
					showError(testResult.error || "Model test failed. Try another model.");
					return;
				}
				if (modelTimedOut) {
					console.warn(
						"models.test timed out for",
						firstModelForTest,
						"\u2014 saving models anyway (local servers may need longer to load)",
					);
				}

				// Save all selected models at once
				const saveModelsRes: RpcResponse = await sendRpc("providers.save_models", {
					provider: savedProviderName,
					models: modelsForSave,
				});
				if (!saveModelsRes?.ok) {
					showError(saveModelsRes?.error?.message || "Failed to save models.");
					return;
				}
				if (modelServiceUnavailable) {
					console.warn("models.test unavailable in provider settings, saved selected models without probe");
				}
				localStorage.setItem("moltis-model", firstModelForTest);
			}

			// Success
			m.body.textContent = "";
			const status = document.createElement("div");
			status.className = "provider-status";
			const countMsg = modelIds.length > 1 ? ` with ${modelIds.length} models` : "";
			status.textContent = `${successDisplayName} configured successfully${countMsg}!`;
			m.body.appendChild(status);
			if (modelTimedOut) {
				const slowHint = document.createElement("div");
				slowHint.className = "text-xs text-[var(--muted)] mt-1";
				slowHint.textContent = "Note: model was slow to respond. It may need a moment to finish loading.";
				m.body.appendChild(slowHint);
			}
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			setTimeout(closeProviderModal, modelTimedOut ? 3500 : 1500);
		})
		.catch((err: Error) => {
			showError(err?.message || "Failed to save credentials.");
		});
}

export function showOAuthFlow(provider: ProviderInfo): void {
	const m = els();
	m.title.textContent = provider.displayName;
	m.body.textContent = "";

	const wrapper = document.createElement("div");
	wrapper.className = "provider-key-form";

	const desc = document.createElement("div");
	desc.className = "text-xs text-[var(--muted)]";
	desc.textContent = `Click below to authenticate with ${provider.displayName} via OAuth.`;
	wrapper.appendChild(desc);

	const manualWrap = document.createElement("div");
	manualWrap.className = "flex flex-col gap-2 mt-2 hidden";

	const manualHint = document.createElement("div");
	manualHint.className = "text-xs text-[var(--muted)]";
	manualHint.textContent = "If localhost callback fails, paste the redirect URL (or code#state) below.";
	manualWrap.appendChild(manualHint);

	const manualInput = document.createElement("input");
	manualInput.type = "text";
	manualInput.className = "provider-key-input w-full";
	manualInput.placeholder = "http://localhost:1455/auth/callback?code=...&state=...";
	manualWrap.appendChild(manualInput);

	const manualBtns = document.createElement("div");
	manualBtns.className = "btn-row";
	const manualSubmitBtn = document.createElement("button");
	manualSubmitBtn.className = "provider-btn provider-btn-secondary";
	manualSubmitBtn.textContent = "Submit Callback";
	manualBtns.appendChild(manualSubmitBtn);
	manualWrap.appendChild(manualBtns);
	wrapper.appendChild(manualWrap);

	const btns = document.createElement("div");
	btns.className = "btn-row";

	const backBtn = document.createElement("button");
	backBtn.className = "provider-btn provider-btn-secondary";
	backBtn.textContent = "Back";
	backBtn.addEventListener("click", () => {
		clearOAuthStatusTimer();
		openProviderModal();
	});
	btns.appendChild(backBtn);

	const connectBtn = document.createElement("button");
	connectBtn.className = "provider-btn";
	connectBtn.textContent = "Connect";
	let oauthCompleted = false;

	function finishOAuthOnce(): void {
		if (oauthCompleted) return;
		oauthCompleted = true;
		clearOAuthStatusTimer();
		showOAuthModelSelector(provider);
	}

	function setManualSubmitting(submitting: boolean): void {
		manualSubmitBtn.disabled = submitting;
		manualInput.disabled = submitting;
		manualSubmitBtn.textContent = submitting ? "Submitting..." : "Submit Callback";
	}

	manualSubmitBtn.addEventListener("click", () => {
		const callback = manualInput.value.trim();
		if (!callback) {
			desc.classList.add("text-error");
			desc.textContent = "Paste the callback URL (or code#state) to continue.";
			return;
		}
		setManualSubmitting(true);
		completeProviderOAuth(provider.name, callback)
			.then((res: RpcResponse) => {
				if (res?.ok) {
					connectBtn.textContent = "Connected";
					desc.classList.remove("text-error");
					desc.textContent = `${provider.displayName} connected successfully!`;
					finishOAuthOnce();
					return;
				}
				desc.classList.add("text-error");
				desc.textContent = res?.error?.message || "Failed to complete OAuth callback.";
			})
			.catch((error: Error) => {
				desc.classList.add("text-error");
				desc.textContent = error?.message || "Failed to complete OAuth callback.";
			})
			.finally(() => {
				setManualSubmitting(false);
			});
	});

	connectBtn.addEventListener("click", () => {
		connectBtn.disabled = true;
		connectBtn.textContent = "Starting...";
		startProviderOAuth(provider.name).then((result) => {
			if (result.status === "already") {
				connectBtn.textContent = "Connected";
				desc.classList.remove("text-error");
				desc.textContent = `${provider.displayName} is already connected (imported credentials found).`;
				finishOAuthOnce();
			} else if (result.status === "browser") {
				window.open(result.authUrl, "_blank");
				connectBtn.textContent = "Waiting for auth...";
				manualWrap.classList.remove("hidden");
				pollOAuthStatus(provider, finishOAuthOnce);
			} else if (result.status === "device") {
				connectBtn.textContent = "Waiting for auth...";
				desc.classList.remove("text-error");
				desc.textContent = "";
				manualWrap.classList.add("hidden");
				const linkEl = document.createElement("a");
				linkEl.href = result.verificationUrl || "";
				linkEl.target = "_blank";
				linkEl.className = "oauth-link";
				linkEl.textContent = result.verificationUrl || "";
				const codeEl = document.createElement("strong");
				codeEl.textContent = result.userCode || "";
				desc.appendChild(document.createTextNode("Go to "));
				desc.appendChild(linkEl);
				desc.appendChild(document.createTextNode(" and enter code: "));
				desc.appendChild(codeEl);
				pollOAuthStatus(provider, finishOAuthOnce);
			} else {
				clearOAuthStatusTimer();
				connectBtn.disabled = false;
				connectBtn.textContent = "Connect";
				manualWrap.classList.add("hidden");
				desc.textContent = result.error || "Failed to start OAuth";
				desc.classList.add("text-error");
			}
		});
	});
	btns.appendChild(connectBtn);
	wrapper.appendChild(btns);
	m.body.appendChild(wrapper);
}

function pollOAuthStatus(provider: ProviderInfo, onAuthenticated: () => void): void {
	const m = els();
	let attempts = 0;
	const maxAttempts = 60;
	clearOAuthStatusTimer();
	oauthStatusTimer = setInterval(() => {
		attempts++;
		if (attempts > maxAttempts) {
			clearOAuthStatusTimer();
			m.body.textContent = "";
			const timeout = document.createElement("div");
			timeout.className = "text-xs text-[var(--error)]";
			timeout.textContent = "OAuth timed out. Please try again.";
			m.body.appendChild(timeout);
			return;
		}
		sendRpc("providers.oauth.status", { provider: provider.name }).then((res: RpcResponse) => {
			if (res?.ok && res.payload && (res.payload as Record<string, unknown>).authenticated) {
				clearOAuthStatusTimer();
				if (typeof onAuthenticated === "function") {
					onAuthenticated();
					return;
				}
				showOAuthModelSelector(provider);
			}
		});
	}, 2000);
}

function showOAuthModelSelector(provider: ProviderInfo): void {
	sendRpc<ModelEntry[]>("models.list", {}).then((modelsRes: RpcResponse<ModelEntry[]>) => {
		const allModels: ModelEntry[] = modelsRes?.ok ? (modelsRes.payload as ModelEntry[]) || [] : [];
		const needle = provider.name.replace(/-/g, "").toLowerCase();
		const provModels = allModels.filter((entry: ModelEntry) => entry.provider?.toLowerCase().replace(/-/g, "").includes(needle));

		if (provModels.length > 0) {
			const mapped: ModelEntry[] = provModels.map((entry: ModelEntry) => ({
				id: entry.id,
				displayName: entry.displayName || entry.id,
				provider: entry.provider,
				supportsTools: entry.supportsTools,
			}));
			showModelSelector(provider, mapped, null, null, null, true);
		} else {
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			const modal = els();
			modal.body.textContent = "";
			const status = document.createElement("div");
			status.className = "provider-status";
			status.textContent = `${provider.displayName} connected successfully!`;
			modal.body.appendChild(status);
			setTimeout(closeProviderModal, 1500);
		}
	});
}

// ── Model selector for existing providers (multi-select) ──

export function openModelSelectorForProvider(providerName: string, providerDisplayName: string): void {
	const m = els();
	m.modal.classList.remove("hidden");
	m.title.textContent = `${providerDisplayName} \u2014 Preferred Models`;
	m.body.textContent = "Loading models...";

	Promise.all([sendRpc<ModelEntry[]>("models.list", {}), sendRpc<ProviderInfo[]>("providers.available", {})]).then(([modelsRes, providersRes]: [RpcResponse<ModelEntry[]>, RpcResponse<ProviderInfo[]>]) => {
		const allModels: ModelEntry[] = modelsRes?.ok ? (modelsRes.payload as ModelEntry[]) || [] : [];
		const needle = providerName.replace(/-/g, "").toLowerCase();
		const provModels = allModels.filter((entry: ModelEntry) => entry.provider?.toLowerCase().replace(/-/g, "").includes(needle));

		if (provModels.length === 0) {
			m.body.textContent = "";
			const wrapper = document.createElement("div");
			wrapper.className = "provider-key-form";
			const msg = document.createElement("div");
			msg.className = "text-xs text-[var(--muted)] py-4 text-center";
			msg.textContent = "No models available yet. Try running Detect All Models first.";
			wrapper.appendChild(msg);
			const btns = document.createElement("div");
			btns.className = "btn-row mt-3";
			const closeBtn = document.createElement("button");
			closeBtn.className = "provider-btn provider-btn-secondary";
			closeBtn.textContent = "Close";
			closeBtn.addEventListener("click", closeProviderModal);
			btns.appendChild(closeBtn);
			wrapper.appendChild(btns);
			m.body.appendChild(wrapper);
			return;
		}

		// Get saved preferred models for this provider.
		const savedModels: Set<string> = new Set();
		if (providersRes?.ok) {
			const providerMeta = ((providersRes.payload as ProviderInfo[]) || []).find((p: ProviderInfo) => p.name === providerName);
			if (providerMeta?.models) {
				for (const sm of providerMeta.models) savedModels.add(sm);
			}
		}

		const mapped: ModelEntry[] = provModels.map((entry: ModelEntry) => ({
			id: entry.id,
			displayName: entry.displayName || entry.id,
			provider: entry.provider,
			supportsTools: entry.supportsTools,
			createdAt: entry.createdAt || 0,
		}));
		showMultiModelSelector(providerName, providerDisplayName, mapped, savedModels);
	});
}

function showMultiModelSelector(providerName: string, providerDisplayName: string, models: ModelEntry[], savedModels: Set<string>): void {
	const m = els();
	m.title.textContent = `${providerDisplayName} \u2014 Preferred Models`;
	m.body.textContent = "";

	const selectedIds: Set<string> = new Set(savedModels);

	// Track per-model probe state: "probing" | "ok" | { error: string }
	const probeResults: Map<string, string | ProbeResult> = new Map();

	function probeModel(modelId: string): void {
		if (probeResults.has(modelId)) return;
		probeResults.set(modelId, "probing");
		renderCards(searchInp?.value.trim() || null);
		testModel(modelId).then((result: TestModelResult) => {
			if (isModelServiceNotConfigured(result.error || "")) {
				// Model service not ready -- don't flag as broken.
				probeResults.delete(modelId);
			} else if (!result.ok && isTimeoutError(result.error || "")) {
				// Timeout -- model may still work, local servers need time to load.
				probeResults.set(modelId, { error: "Slow to respond (may still work)", timeout: true });
			} else {
				probeResults.set(modelId, result.ok ? "ok" : { error: humanizeProbeError(result.error || "Unsupported") as string });
			}
			renderCards(searchInp?.value.trim() || null);
		});
	}

	const wrapper = document.createElement("div");
	wrapper.className = "provider-key-form flex flex-col min-h-0 flex-1";

	const label = document.createElement("div");
	label.className = "text-xs font-medium text-[var(--text-strong)] mb-1 shrink-0";
	label.textContent = "Select models to pin at the top of the dropdown";
	wrapper.appendChild(label);

	const hint = document.createElement("div");
	hint.className = "text-xs text-[var(--muted)] mb-2 shrink-0";
	hint.textContent = "Selected models appear first in the session model selector.";
	wrapper.appendChild(hint);

	// Search input when >5 models
	let searchInp: HTMLInputElement | null = null;
	if (models.length > 5) {
		searchInp = document.createElement("input");
		searchInp.type = "text";
		searchInp.className = "provider-key-input w-full text-xs mb-2 shrink-0";
		searchInp.placeholder = "Search models\u2026";
		wrapper.appendChild(searchInp);
	}

	const list = document.createElement("div");
	list.className = "flex flex-col gap-1 overflow-y-auto flex-1 min-h-0";
	wrapper.appendChild(list);

	const statusArea = document.createElement("div");
	statusArea.className = "text-xs text-[var(--muted)] mt-2 shrink-0";
	wrapper.appendChild(statusArea);

	function updateStatus(): void {
		const count = selectedIds.size;
		statusArea.textContent = count === 0 ? "No models selected" : `${count} model${count > 1 ? "s" : ""} selected`;
	}

	function sortModelsForSelection(items: ModelEntry[]): ModelEntry[] {
		return [...items].sort((a: ModelEntry, b: ModelEntry) => {
			const aSel = selectedIds.has(a.id) ? 0 : 1;
			const bSel = selectedIds.has(b.id) ? 0 : 1;
			if (aSel !== bSel) return aSel - bSel;
			const aTime = a.createdAt || 0;
			const bTime = b.createdAt || 0;
			if (aTime !== bTime) return bTime - aTime;
			const aVer = modelVersionScore(a.id);
			const bVer = modelVersionScore(b.id);
			if (aVer !== bVer) return bVer - aVer;
			return (a.displayName || a.id).localeCompare(b.displayName || b.id);
		});
	}

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: card rendering with probe badges
	function renderCards(filter: string | null): void {
		list.textContent = "";
		let filtered = models;
		if (filter) {
			const q = filter.toLowerCase();
			filtered = models.filter(
				(entry: ModelEntry) => entry.displayName.toLowerCase().includes(q) || entry.id.toLowerCase().includes(q),
			);
		}
		if (filtered.length === 0) {
			const empty = document.createElement("div");
			empty.className = "text-xs text-[var(--muted)] py-4 text-center";
			empty.textContent = "No models match your search.";
			list.appendChild(empty);
			return;
		}
		const sorted = sortModelsForSelection(filtered);
		for (const mdl of sorted) {
			const card = document.createElement("div");
			card.className = `model-card ${selectedIds.has(mdl.id) ? "selected" : ""}`;

			const header = document.createElement("div");
			header.className = "flex items-center justify-between";

			const nameSpan = document.createElement("span");
			nameSpan.className = "text-sm font-medium text-[var(--text)] truncate";
			nameSpan.textContent = mdl.displayName;
			header.appendChild(nameSpan);

			const badges = document.createElement("div");
			badges.className = "flex gap-2";
			if (mdl.supportsTools) {
				const toolsBadge = document.createElement("span");
				toolsBadge.className = "recommended-badge";
				toolsBadge.textContent = "Tools";
				badges.appendChild(toolsBadge);
			}
			const probe = probeResults.get(mdl.id);
			if (probe === "probing") {
				const probeBadge = document.createElement("span");
				probeBadge.className = "tier-badge";
				probeBadge.textContent = "Probing\u2026";
				badges.appendChild(probeBadge);
			} else if (probe && probe !== "ok") {
				const probeObj = probe as ProbeResult;
				const unsupBadge = document.createElement("span");
				unsupBadge.className = probeObj.timeout ? "tier-badge" : "provider-item-badge warning";
				unsupBadge.textContent = probeObj.timeout ? "Slow" : "Unsupported";
				badges.appendChild(unsupBadge);
			}
			header.appendChild(badges);
			card.appendChild(header);

			const idLine = document.createElement("div");
			idLine.className = "text-xs text-[var(--muted)] mt-1 font-mono";
			idLine.textContent = mdl.id;
			card.appendChild(idLine);

			if (probe && probe !== "ok" && probe !== "probing" && (probe as ProbeResult).error) {
				const errorLine = document.createElement("div");
				errorLine.className = "text-xs font-medium text-[var(--danger,#ef4444)] mt-0.5";
				errorLine.textContent = (probe as ProbeResult).error || "";
				card.appendChild(errorLine);
			}

			if (mdl.createdAt) {
				const dateLine = document.createElement("time");
				dateLine.className = "text-xs text-[var(--muted)] mt-0.5 opacity-60 block";
				dateLine.setAttribute("data-epoch-ms", String(mdl.createdAt * 1000));
				dateLine.setAttribute("data-format", "year-month");
				card.appendChild(dateLine);
			}

			// Closure to capture mdl
			((modelId: string) => {
				card.addEventListener("click", () => {
					if (selectedIds.has(modelId)) {
						selectedIds.delete(modelId);
					} else {
						selectedIds.add(modelId);
						probeModel(modelId);
					}
					renderCards(searchInp?.value.trim() || null);
					updateStatus();
				});
			})(mdl.id);

			list.appendChild(card);
		}
	}

	renderCards(null);
	updateStatus();

	if (searchInp) {
		searchInp.addEventListener("input", () => {
			renderCards(searchInp?.value.trim());
		});
	}

	const errorArea = document.createElement("div");
	errorArea.className = "alert-error-text text-[var(--error)] whitespace-pre-line shrink-0";
	errorArea.style.display = "none";
	wrapper.appendChild(errorArea);

	// Buttons -- always visible at the bottom
	const btns = document.createElement("div");
	btns.className = "btn-row mt-3 shrink-0";

	const cancelBtn = document.createElement("button");
	cancelBtn.className = "provider-btn provider-btn-secondary";
	cancelBtn.textContent = "Cancel";
	cancelBtn.addEventListener("click", closeProviderModal);
	btns.appendChild(cancelBtn);

	const saveBtn = document.createElement("button");
	saveBtn.className = "provider-btn";
	saveBtn.textContent = "Save";
	saveBtn.addEventListener("click", () => {
		saveBtn.disabled = true;
		saveBtn.textContent = "Saving\u2026";
		errorArea.style.display = "none";

		sendRpc("providers.save_models", { provider: providerName, models: Array.from(selectedIds) })
			.then((res: RpcResponse) => {
				if (!res?.ok) {
					saveBtn.disabled = false;
					saveBtn.textContent = "Save";
					errorArea.textContent = res?.error?.message || "Failed to save model preferences.";
					errorArea.style.display = "";
					return;
				}
				fetchModels();
				if (S.refreshProvidersPage) S.refreshProvidersPage();
				closeProviderModal();
			})
			.catch((err: Error) => {
				saveBtn.disabled = false;
				saveBtn.textContent = "Save";
				errorArea.textContent = err?.message || "Failed to save model preferences.";
				errorArea.style.display = "";
			});
	});
	btns.appendChild(saveBtn);

	wrapper.appendChild(btns);
	m.body.appendChild(wrapper);
}

// ── Local model flow ──────────────────────────────────────

export function showLocalModelFlow(provider: ProviderInfo): void {
	const m = els();
	m.title.textContent = provider.displayName;
	m.body.textContent = "Loading system info...";

	// Fetch system info first
	sendRpc<SystemInfo>("providers.local.system_info", {}).then((sysRes: RpcResponse<SystemInfo>) => {
		if (!sysRes?.ok) {
			m.body.textContent = sysRes?.error?.message || "Failed to get system info";
			return;
		}
		const sysInfo = sysRes.payload as SystemInfo;

		// Fetch available models
		sendRpc<ModelsData>("providers.local.models", {}).then((modelsRes: RpcResponse<ModelsData>) => {
			if (!modelsRes?.ok) {
				m.body.textContent = modelsRes?.error?.message || "Failed to get models";
				return;
			}
			const modelsData = modelsRes.payload as ModelsData;
			renderLocalModelSelection(provider, sysInfo, modelsData);
		});
	});
}

// Store the selected backend for model configuration
let selectedBackend: string | null = null;

function renderLocalModelSelection(provider: ProviderInfo, sysInfo: SystemInfo, modelsData: ModelsData): void {
	const m = els();
	m.body.textContent = "";

	// Initialize selected backend to recommended
	selectedBackend = sysInfo.recommendedBackend || "GGUF";

	const wrapper = document.createElement("div") as ModelSelectorWrapper;
	wrapper.className = "provider-key-form";

	// System info section
	const sysSection = document.createElement("div");
	sysSection.className = "flex flex-col gap-2 mb-4";

	const sysTitle = document.createElement("div");
	sysTitle.className = "text-xs font-medium text-[var(--text-strong)]";
	sysTitle.textContent = "System Info";
	sysSection.appendChild(sysTitle);

	const sysDetails = document.createElement("div");
	sysDetails.className = "flex gap-3 text-xs text-[var(--muted)]";

	const ramSpan = document.createElement("span");
	ramSpan.textContent = `RAM: ${sysInfo.totalRamGb}GB`;
	sysDetails.appendChild(ramSpan);

	const tierSpan = document.createElement("span");
	tierSpan.textContent = `Tier: ${sysInfo.memoryTier}`;
	sysDetails.appendChild(tierSpan);

	if (sysInfo.hasGpu) {
		const gpuSpan = document.createElement("span");
		gpuSpan.className = "text-[var(--ok)]";
		gpuSpan.textContent = "GPU available";
		sysDetails.appendChild(gpuSpan);
	}

	sysSection.appendChild(sysDetails);
	wrapper.appendChild(sysSection);

	// Backend selector (show on Apple Silicon where both GGUF and MLX are options)
	const backends = sysInfo.availableBackends || [];
	if (sysInfo.isAppleSilicon && backends.length > 0) {
		const backendSection = document.createElement("div");
		backendSection.className = "flex flex-col gap-2 mb-4";

		const backendLabel = document.createElement("div");
		backendLabel.className = "text-xs font-medium text-[var(--text-strong)]";
		backendLabel.textContent = "Inference Backend";
		backendSection.appendChild(backendLabel);

		const backendCards = document.createElement("div");
		backendCards.className = "flex flex-col gap-2";

		// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: backend card rendering with many conditions
		backends.forEach((b: BackendInfo) => {
			const card = document.createElement("div");
			card.className = "backend-card";
			if (!b.available) card.className += " disabled";
			if (b.id === selectedBackend) card.className += " selected";
			card.dataset.backendId = b.id;

			const header = document.createElement("div");
			header.className = "flex items-center justify-between";

			const name = document.createElement("span");
			name.className = "backend-name text-sm font-medium text-[var(--text)]";
			name.textContent = b.name;
			header.appendChild(name);

			const badges = document.createElement("div");
			badges.className = "flex gap-2";

			if (b.id === sysInfo.recommendedBackend && b.available) {
				const recBadge = document.createElement("span");
				recBadge.className = "recommended-badge";
				recBadge.textContent = "Recommended";
				badges.appendChild(recBadge);
			}

			if (!b.available) {
				const unavailBadge = document.createElement("span");
				unavailBadge.className = "tier-badge";
				unavailBadge.textContent = "Not installed";
				badges.appendChild(unavailBadge);
			}

			header.appendChild(badges);
			card.appendChild(header);

			const desc = document.createElement("div");
			desc.className = "text-xs text-[var(--muted)] mt-1";
			desc.textContent = b.description;
			card.appendChild(desc);

			// Show install instructions for unavailable backends
			if (!b.available && b.id === "MLX") {
				const cmds = b.installCommands || ["pip install mlx-lm"];
				const tpl = S.$<HTMLTemplateElement>("tpl-install-hint")!;
				const hintEl = (tpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
				const labelEl = hintEl.querySelector("[data-install-label]") as HTMLElement;
				const container = hintEl.querySelector("[data-install-commands]") as HTMLElement;

				labelEl.textContent = cmds.length === 1 ? "Install with:" : "Install with any of:";

				const cmdTpl = S.$<HTMLTemplateElement>("tpl-install-cmd")!;
				cmds.forEach((c: string) => {
					const cmdEl = (cmdTpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
					cmdEl.textContent = c;
					container.appendChild(cmdEl);
				});

				card.appendChild(hintEl);
			}

			if (b.available) {
				card.addEventListener("click", () => {
					// Deselect all cards
					backendCards.querySelectorAll(".backend-card").forEach((c: Element) => {
						c.classList.remove("selected");
					});
					// Select this card
					card.classList.add("selected");
					selectedBackend = b.id;
					// Re-render models for new backend
					if (wrapper._renderModelsForBackend) {
						wrapper._renderModelsForBackend(b.id);
					}
					// Update filename input visibility
					if (wrapper._updateFilenameVisibility) {
						wrapper._updateFilenameVisibility(b.id);
					}
				});
			}

			backendCards.appendChild(card);
		});

		backendSection.appendChild(backendCards);
		wrapper.appendChild(backendSection);
	} else if (sysInfo.backendNote) {
		// Non-Apple Silicon - just show info
		const backendDiv = document.createElement("div");
		backendDiv.className = "text-xs text-[var(--muted)] mb-4";
		// Safe: backendNote comes from server system info, not user input
		backendDiv.textContent = `Backend: ${sysInfo.backendNote}`;
		wrapper.appendChild(backendDiv);
	}

	// Models section
	const modelsTitle = document.createElement("div");
	modelsTitle.className = "text-xs font-medium text-[var(--text-strong)] mb-2";
	modelsTitle.textContent = "Select a Model";
	wrapper.appendChild(modelsTitle);

	const modelsList = document.createElement("div");
	modelsList.className = "flex flex-col gap-2";
	modelsList.id = "local-model-list";

	// Helper to render models filtered by backend
	function renderModelsForBackend(backend: string): void {
		modelsList.textContent = "";
		const recommended = modelsData.recommended || [];
		const filtered = recommended.filter((mdl: LocalModelInfo) => mdl.backend === backend);
		if (filtered.length === 0) {
			const empty = document.createElement("div");
			empty.className = "text-xs text-[var(--muted)] py-4 text-center";
			empty.textContent = `No models available for ${backend}`;
			modelsList.appendChild(empty);
			return;
		}
		filtered.forEach((model: LocalModelInfo) => {
			const card = createModelCard(model, provider, sysInfo.totalRamGb);
			modelsList.appendChild(card);
		});
	}

	// Initial render with selected backend
	renderModelsForBackend(selectedBackend);

	// Store render function for backend card click handlers
	wrapper._renderModelsForBackend = renderModelsForBackend;

	wrapper.appendChild(modelsList);

	// HuggingFace search section
	const searchSection = document.createElement("div");
	searchSection.className = "flex flex-col gap-2 mt-4 pt-4 border-t border-[var(--border)]";

	const searchLabel = document.createElement("div");
	searchLabel.className = "text-xs font-medium text-[var(--text-strong)]";
	searchLabel.textContent = "Search HuggingFace";
	searchSection.appendChild(searchLabel);

	const searchRow = document.createElement("div");
	searchRow.className = "flex gap-2";

	const searchInput = document.createElement("input");
	searchInput.type = "text";
	searchInput.placeholder = "Search models...";
	searchInput.className = "provider-input flex-1";
	searchRow.appendChild(searchInput);

	const searchBtn = document.createElement("button");
	searchBtn.className = "provider-btn provider-btn-secondary";
	searchBtn.textContent = "Search";
	searchRow.appendChild(searchBtn);

	searchSection.appendChild(searchRow);

	const searchResults = document.createElement("div");
	searchResults.className = "flex flex-col gap-2 max-h-48 overflow-y-auto";
	searchResults.id = "hf-search-results";
	searchSection.appendChild(searchResults);

	// Search handler
	const doSearch = async (): Promise<void> => {
		const query = searchInput.value.trim();
		if (!query) return;
		searchBtn.disabled = true;
		searchBtn.textContent = "Searching...";
		searchResults.textContent = "";
		const res = await sendRpc<{ results: HfSearchResult[] }>("providers.local.search_hf", {
			query: query,
			backend: selectedBackend,
			limit: 15,
		});
		searchBtn.disabled = false;
		searchBtn.textContent = "Search";
		if (!(res?.ok && (res.payload as { results?: HfSearchResult[] })?.results?.length)) {
			const noResults = document.createElement("div");
			noResults.className = "text-xs text-[var(--muted)] py-2";
			noResults.textContent = "No results found";
			searchResults.appendChild(noResults);
			return;
		}
		(res.payload as { results: HfSearchResult[] }).results.forEach((result: HfSearchResult) => {
			const card = createHfSearchResultCard(result, provider);
			searchResults.appendChild(card);
		});
	};

	searchBtn.addEventListener("click", doSearch);
	searchInput.addEventListener("keydown", (e: KeyboardEvent) => {
		if (e.key === "Enter" && !e.isComposing) doSearch();
	});

	// Auto-search with debounce when user stops typing
	let searchTimeout: ReturnType<typeof setTimeout> | null = null;
	searchInput.addEventListener("input", () => {
		if (searchTimeout) clearTimeout(searchTimeout);
		const query = searchInput.value.trim();
		if (query.length >= 2) {
			searchTimeout = setTimeout(doSearch, 500);
		}
	});

	wrapper.appendChild(searchSection);

	// Custom repo section
	const customSection = document.createElement("div");
	customSection.className = "flex flex-col gap-2 mt-4 pt-4 border-t border-[var(--border)]";

	const customLabel = document.createElement("div");
	customLabel.className = "text-xs font-medium text-[var(--text-strong)]";
	customLabel.textContent = "Or enter HuggingFace repo URL";
	customSection.appendChild(customLabel);

	const customRow = document.createElement("div");
	customRow.className = "flex gap-2";

	const customInput = document.createElement("input");
	customInput.type = "text";
	customInput.placeholder = selectedBackend === "MLX" ? "mlx-community/Model-Name" : "TheBloke/Model-GGUF";
	customInput.className = "provider-input flex-1";
	customRow.appendChild(customInput);

	const customBtn = document.createElement("button");
	customBtn.className = "provider-btn";
	customBtn.textContent = "Use";
	customRow.appendChild(customBtn);

	customSection.appendChild(customRow);

	// GGUF filename input (only for GGUF backend)
	const filenameRow = document.createElement("div");
	filenameRow.className = "flex gap-2";
	filenameRow.style.display = selectedBackend === "GGUF" ? "flex" : "none";

	const filenameInput = document.createElement("input");
	filenameInput.type = "text";
	filenameInput.placeholder = "model-file.gguf (required for GGUF)";
	filenameInput.className = "provider-input flex-1";
	filenameRow.appendChild(filenameInput);

	customSection.appendChild(filenameRow);

	// Update filename visibility when backend changes
	wrapper._updateFilenameVisibility = (backend: string): void => {
		filenameRow.style.display = backend === "GGUF" ? "flex" : "none";
		customInput.placeholder = backend === "MLX" ? "mlx-community/Model-Name" : "TheBloke/Model-GGUF";
	};

	// Custom repo handler
	customBtn.addEventListener("click", async () => {
		const repo = customInput.value.trim();
		if (!repo) return;

		const params: Record<string, string | null> = {
			hfRepo: repo,
			backend: selectedBackend,
		};
		if (selectedBackend === "GGUF") {
			const filename = filenameInput.value.trim();
			if (!filename) {
				filenameInput.focus();
				return;
			}
			params.hfFilename = filename;
		}

		customBtn.disabled = true;
		customBtn.textContent = "Configuring...";
		const res = await sendRpc<{ modelId: string }>("providers.local.configure_custom", params);
		customBtn.disabled = false;
		customBtn.textContent = "Use";

		if (res?.ok) {
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			showModelDownloadProgress({ id: (res.payload as { modelId: string }).modelId, displayName: repo }, provider);
		} else {
			const err = res?.error?.message || "Failed to configure model";
			const errEl = document.createElement("div");
			errEl.className = "text-xs text-[var(--error)] py-2";
			errEl.textContent = err;
			searchResults.textContent = "";
			searchResults.appendChild(errEl);
		}
	});

	wrapper.appendChild(customSection);

	// Back button
	const btns = document.createElement("div");
	btns.className = "btn-row mt-4";

	const backBtn = document.createElement("button");
	backBtn.className = "provider-btn provider-btn-secondary";
	backBtn.textContent = "Back";
	backBtn.addEventListener("click", openProviderModal);
	btns.appendChild(backBtn);
	wrapper.appendChild(btns);

	m.body.appendChild(wrapper);
}

// Create a card for HuggingFace search result
function createHfSearchResultCard(model: HfSearchResult, provider: ProviderInfo): HTMLElement {
	const card = document.createElement("div");
	card.className = "model-card";

	const header = document.createElement("div");
	header.className = "flex items-center justify-between";

	const name = document.createElement("span");
	name.className = "text-sm font-medium text-[var(--text)]";
	name.textContent = model.displayName;
	header.appendChild(name);

	const stats = document.createElement("div");
	stats.className = "flex gap-2 text-xs text-[var(--muted)]";
	if (model.downloads) {
		const dl = document.createElement("span");
		dl.textContent = `\u2193${formatDownloads(model.downloads)}`;
		stats.appendChild(dl);
	}
	if (model.likes) {
		const likes = document.createElement("span");
		likes.textContent = `\u2665${model.likes}`;
		stats.appendChild(likes);
	}
	header.appendChild(stats);

	card.appendChild(header);

	const repo = document.createElement("div");
	repo.className = "text-xs text-[var(--muted)] mt-1";
	repo.textContent = model.id;
	card.appendChild(repo);

	card.addEventListener("click", async () => {
		// Prevent multiple clicks
		if (card.dataset.configuring) return;
		card.dataset.configuring = "true";

		const params: Record<string, string> = {
			hfRepo: model.id,
			backend: model.backend,
		};
		// For GGUF, we'd need to fetch the file list - for now, prompt user
		if (model.backend === "GGUF") {
			const filename = prompt("Enter the GGUF filename (e.g., model-q4_k_m.gguf):");
			if (!filename) {
				delete card.dataset.configuring;
				return;
			}
			params.hfFilename = filename;
		}
		card.style.opacity = "0.5";
		card.style.pointerEvents = "none";

		// Show configuring state in modal
		const modalEls = els();
		modalEls.body.textContent = "";
		const statusWrapper = document.createElement("div");
		statusWrapper.className = "provider-key-form";
		const statusText = document.createElement("div");
		statusText.className = "text-sm text-[var(--text)]";
		statusText.textContent = `Configuring ${model.displayName}...`;
		statusWrapper.appendChild(statusText);
		modalEls.body.appendChild(statusWrapper);

		const res = await sendRpc<{ modelId: string }>("providers.local.configure_custom", params);
		if (res?.ok) {
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			showModelDownloadProgress({ id: (res.payload as { modelId: string }).modelId, displayName: model.displayName }, provider);
		} else {
			const err = res?.error?.message || "Failed to configure model";
			statusText.className = "text-sm text-[var(--error)]";
			statusText.textContent = err;
		}
	});

	return card;
}

// Format download count (e.g., 1234567 -> "1.2M")
function formatDownloads(n: number): string {
	if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
	if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
	return n.toString();
}

function createModelCard(model: LocalModelInfo, provider: ProviderInfo, totalRamGb: number): HTMLElement {
	const card = document.createElement("div");
	card.className = "model-card";
	const detectedRamGb = Number.isFinite(totalRamGb) ? totalRamGb : 0;
	const hasEnoughRam = detectedRamGb >= model.minRamGb;

	const header = document.createElement("div");
	header.className = "flex items-center justify-between";

	const name = document.createElement("span");
	name.className = "text-sm font-medium text-[var(--text)]";
	name.textContent = model.displayName;
	header.appendChild(name);

	const badges = document.createElement("div");
	badges.className = "flex gap-2";

	const ramBadge = document.createElement("span");
	ramBadge.className = "tier-badge";
	ramBadge.textContent = `${model.minRamGb}GB`;
	badges.appendChild(ramBadge);

	if (model.suggested && hasEnoughRam) {
		const suggestedBadge = document.createElement("span");
		suggestedBadge.className = "recommended-badge";
		suggestedBadge.textContent = "Recommended";
		badges.appendChild(suggestedBadge);
	}

	if (!hasEnoughRam) {
		const insufficientBadge = document.createElement("span");
		insufficientBadge.className = "tier-badge";
		insufficientBadge.textContent = "Insufficient RAM";
		badges.appendChild(insufficientBadge);
	}

	header.appendChild(badges);
	card.appendChild(header);

	const meta = document.createElement("div");
	meta.className = "text-xs text-[var(--muted)] mt-1";
	meta.textContent = `Context: ${(model.contextWindow / 1000).toFixed(0)}k tokens`;
	card.appendChild(meta);

	if (!hasEnoughRam) {
		card.classList.add("disabled");
		const warning = document.createElement("div");
		warning.className = "text-xs text-[var(--error)] mt-1";
		warning.textContent = `You do not have enough RAM for this model (${detectedRamGb}GB detected, ${model.minRamGb}GB required).`;
		card.appendChild(warning);
		return card;
	}

	card.addEventListener("click", () => selectLocalModel(model, provider));

	return card;
}

export function showModelDownloadProgress(model: { id: string; displayName: string }, provider: ProviderInfo): void {
	const m = els();
	m.modal.classList.remove("hidden");
	m.body.textContent = "";

	const wrapper = document.createElement("div");
	wrapper.className = "provider-key-form";

	const status = document.createElement("div");
	status.className = "text-sm text-[var(--text)]";
	status.textContent = `Configuring ${model.displayName}...`;
	wrapper.appendChild(status);

	const progress = document.createElement("div");
	progress.className = "download-progress mt-4";

	const progressBar = document.createElement("div");
	progressBar.className = "download-progress-bar";
	progressBar.style.width = "0%";
	progress.appendChild(progressBar);

	const progressText = document.createElement("div");
	progressText.className = "text-xs text-[var(--muted)] mt-2";
	progress.appendChild(progressText);

	wrapper.appendChild(progress);
	m.body.appendChild(wrapper);

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: download progress handler with many states
	const off = onEvent("local-llm.download", (payload: unknown) => {
		const p = payload as LocalLlmDownloadPayload;
		if (p.modelId !== model.id) return;

		if (p.error) {
			status.textContent = p.error;
			status.className = "text-sm text-[var(--error)]";
			off();
			return;
		}

		if (p.complete) {
			status.textContent = `${model.displayName} downloaded successfully!`;
			status.className = "provider-status";
			progressBar.style.width = "100%";
			progressText.textContent = "";
			off();
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			setTimeout(closeProviderModal, 1500);
			return;
		}

		if (p.progress != null) {
			progressBar.style.width = `${p.progress.toFixed(1)}%`;
			status.textContent = `Downloading ${model.displayName}...`;
		}
		if (p.downloaded != null) {
			const downloadedMb = (p.downloaded / (1024 * 1024)).toFixed(1);
			if (p.total != null) {
				const totalMb = (p.total / (1024 * 1024)).toFixed(1);
				progressText.textContent = `${downloadedMb} MB / ${totalMb} MB`;
			} else {
				progressText.textContent = `${downloadedMb} MB downloaded`;
			}
		}
	});

	pollLocalStatus(model, provider, status, progress, off);
}

function selectLocalModel(model: LocalModelInfo, provider: ProviderInfo): void {
	const m = els();
	m.body.textContent = "";

	const wrapper = document.createElement("div");
	wrapper.className = "provider-key-form";

	const status = document.createElement("div");
	status.className = "text-sm text-[var(--text)]";
	status.textContent = `Configuring ${model.displayName}...`;
	wrapper.appendChild(status);

	const progress = document.createElement("div");
	progress.className = "download-progress mt-4";

	const progressBar = document.createElement("div");
	progressBar.className = "download-progress-bar";
	progressBar.style.width = "0%";
	progress.appendChild(progressBar);

	const progressText = document.createElement("div");
	progressText.className = "text-xs text-[var(--muted)] mt-2";
	progress.appendChild(progressText);

	wrapper.appendChild(progress);
	m.body.appendChild(wrapper);

	// Subscribe to download progress events
	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: download progress handler with many states
	const off = onEvent("local-llm.download", (payload: unknown) => {
		const p = payload as LocalLlmDownloadPayload;
		if (p.modelId !== model.id) return;

		if (p.error) {
			status.textContent = p.error;
			status.className = "text-sm text-[var(--error)]";
			off();
			return;
		}

		if (p.complete) {
			status.textContent = `${model.displayName} downloaded successfully!`;
			status.className = "provider-status";
			progressBar.style.width = "100%";
			progressText.textContent = "";
			off();
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			setTimeout(closeProviderModal, 1500);
			return;
		}

		// Update progress
		if (p.progress != null) {
			progressBar.style.width = `${p.progress.toFixed(1)}%`;
			status.textContent = `Downloading ${model.displayName}...`;
		}
		if (p.downloaded != null) {
			const downloadedMb = (p.downloaded / (1024 * 1024)).toFixed(1);
			if (p.total != null) {
				const totalMb = (p.total / (1024 * 1024)).toFixed(1);
				progressText.textContent = `${downloadedMb} MB / ${totalMb} MB`;
			} else {
				progressText.textContent = `${downloadedMb} MB downloaded`;
			}
		}
	});

	sendRpc("providers.local.configure", { modelId: model.id, backend: selectedBackend }).then((res: RpcResponse) => {
		if (!res?.ok) {
			status.textContent = res?.error?.message || "Failed to configure model";
			status.className = "text-sm text-[var(--error)]";
			off(); // Unsubscribe from events
			return;
		}

		// Start polling for status as a fallback (in case WebSocket events are missed)
		pollLocalStatus(model, provider, status, progress, off);
	});
}

function pollLocalStatus(model: { id: string; displayName: string }, _provider: ProviderInfo, statusEl: HTMLElement, progressEl: HTMLElement, offEvent: (() => void) | null): void {
	let attempts = 0;
	const maxAttempts = 300; // 10 minutes with 2s interval
	let completed = false;
	const timer = setInterval(() => {
		if (completed) {
			clearInterval(timer);
			return;
		}
		attempts++;
		if (attempts > maxAttempts) {
			clearInterval(timer);
			if (offEvent) offEvent();
			statusEl.textContent = "Configuration timed out. Please try again.";
			statusEl.className = "text-sm text-[var(--error)]";
			return;
		}

		// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: status polling with many state transitions
		sendRpc<{ status: string; error?: string }>("providers.local.status", {}).then((res: RpcResponse<{ status: string; error?: string }>) => {
			if (!res?.ok) return;
			const st = res.payload as { status: string; error?: string };

			if (st.status === "ready" || st.status === "loaded") {
				completed = true;
				clearInterval(timer);
				if (offEvent) offEvent();
				statusEl.textContent = `${model.displayName} configured successfully!`;
				statusEl.className = "provider-status";
				progressEl.style.display = "none";
				fetchModels();
				if (S.refreshProvidersPage) S.refreshProvidersPage();
				setTimeout(closeProviderModal, 1500);
			} else if (st.status === "error") {
				completed = true;
				clearInterval(timer);
				if (offEvent) offEvent();
				statusEl.textContent = st.error || "Configuration failed";
				statusEl.className = "text-sm text-[var(--error)]";
			}
			// Don't update progress here - let WebSocket events handle it
		});
	}, 2000);
}
