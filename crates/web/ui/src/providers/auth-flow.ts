// ── Provider auth flows: OAuth, API key form, model selector ──

import { modelVersionScore, sendRpc } from "../helpers";
import { fetchModels } from "../models";
import { providerApiKeyHelp } from "../provider-key-help";
import { completeProviderOAuth, startProviderOAuth } from "../provider-oauth";
import type { TestModelResult } from "../provider-validation";
import {
	humanizeProbeError,
	isModelServiceNotConfigured,
	isTimeoutError,
	saveProviderKey,
	testModel,
	validateProviderKey,
} from "../provider-validation";
import * as S from "../state";
import type { RpcResponse } from "../types";
import {
	BYOM_PROVIDERS,
	bindValidationProgressEvents,
	clearOAuthStatusTimer,
	closeProviderModal,
	completeValidationProgress,
	createValidationProgress,
	createValidationRequestId,
	els,
	OPENAI_COMPATIBLE_PROVIDERS,
	openProviderModal,
	resetValidationProgress,
	setFormError,
	setOAuthStatusTimer,
	setValidationProgress,
	shouldUseCustomProviderForOpenAi,
	stripModelNamespace,
} from "./shared";
import type { AddCustomPayload, ModelEntry, ModelSelectorWrapper, ProbeResult, ProviderInfo } from "./types";

// ── API key form ─────────────────────────────────────────────

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

// ── Model selector (after auth) ──────────────────────────────

export function showModelSelector(
	provider: ProviderInfo,
	models: ModelEntry[],
	keyVal: string | null,
	endpointVal: string | null,
	modelVal: string | null,
	skipSave?: boolean,
): void {
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
		return models.filter(
			(mdl: ModelEntry) => mdl.displayName.toLowerCase().includes(q) || mdl.id.toLowerCase().includes(q),
		);
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
			filtered = models.filter(
				(mdl: ModelEntry) => mdl.displayName.toLowerCase().includes(q) || mdl.id.toLowerCase().includes(q),
			);
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

// ── Save and finish provider ─────────────────────────────────

function saveAndFinishProvider(
	provider: ProviderInfo,
	keyVal: string | null,
	endpointVal: string | null,
	modelVal: string | null,
	selectedModelIds: string[] | null,
	skipSave: boolean,
): void {
	// selectedModelIds can be a single string (legacy callers) or an array
	const modelIds: string[] = Array.isArray(selectedModelIds)
		? selectedModelIds
		: selectedModelIds
			? [selectedModelIds]
			: [];

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
			const savedProviderName = saveAsCustomProvider
				? (res?.payload as AddCustomPayload)?.providerName || provider.name
				: provider.name;
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

// ── OAuth flow ───────────────────────────────────────────────

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
	setOAuthStatusTimer(
		setInterval(() => {
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
		}, 2000),
	);
}

function showOAuthModelSelector(provider: ProviderInfo): void {
	sendRpc<ModelEntry[]>("models.list", {}).then((modelsRes: RpcResponse<ModelEntry[]>) => {
		const allModels: ModelEntry[] = modelsRes?.ok ? (modelsRes.payload as ModelEntry[]) || [] : [];
		const needle = provider.name.replace(/-/g, "").toLowerCase();
		const provModels = allModels.filter((entry: ModelEntry) =>
			entry.provider?.toLowerCase().replace(/-/g, "").includes(needle),
		);

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

// ── Model selector for existing providers (multi-select) ─────

export function openModelSelectorForProvider(providerName: string, providerDisplayName: string): void {
	const m = els();
	m.modal.classList.remove("hidden");
	m.title.textContent = `${providerDisplayName} \u2014 Preferred Models`;
	m.body.textContent = "Loading models...";

	Promise.all([sendRpc<ModelEntry[]>("models.list", {}), sendRpc<ProviderInfo[]>("providers.available", {})]).then(
		([modelsRes, providersRes]: [RpcResponse<ModelEntry[]>, RpcResponse<ProviderInfo[]>]) => {
			const allModels: ModelEntry[] = modelsRes?.ok ? (modelsRes.payload as ModelEntry[]) || [] : [];
			const needle = providerName.replace(/-/g, "").toLowerCase();
			const provModels = allModels.filter((entry: ModelEntry) =>
				entry.provider?.toLowerCase().replace(/-/g, "").includes(needle),
			);

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
				const providerMeta = ((providersRes.payload as ProviderInfo[]) || []).find(
					(p: ProviderInfo) => p.name === providerName,
				);
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
		},
	);
}

function showMultiModelSelector(
	providerName: string,
	providerDisplayName: string,
	models: ModelEntry[],
	savedModels: Set<string>,
): void {
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
				probeResults.set(
					modelId,
					result.ok ? "ok" : { error: humanizeProbeError(result.error || "Unsupported") as string },
				);
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
