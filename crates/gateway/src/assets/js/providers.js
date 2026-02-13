// ── Provider modal ──────────────────────────────────────

import { onEvent } from "./events.js";
import { sendRpc } from "./helpers.js";
import { ensureProviderModal } from "./modals.js";
import { fetchModels } from "./models.js";
import { providerApiKeyHelp } from "./provider-key-help.js";
import { startProviderOAuth } from "./provider-oauth.js";
import { isModelServiceNotConfigured, testModel, validateProviderKey } from "./provider-validation.js";
import * as S from "./state.js";

var _els = null;

function els() {
	if (!_els) {
		ensureProviderModal();
		_els = {
			modal: S.$("providerModal"),
			body: S.$("providerModalBody"),
			title: S.$("providerModalTitle"),
			close: S.$("providerModalClose"),
		};
		_els.close.addEventListener("click", closeProviderModal);
		_els.modal.addEventListener("click", (e) => {
			if (e.target === _els.modal) closeProviderModal();
		});
	}
	return _els;
}

// Re-export for backwards compat with page-providers.js
export function getProviderModal() {
	return els().modal;
}

// Providers that support custom endpoint configuration
var OPENAI_COMPATIBLE_PROVIDERS = [
	"openai",
	"mistral",
	"openrouter",
	"cerebras",
	"minimax",
	"moonshot",
	"venice",
	"ollama",
];

var BYOM_PROVIDERS = ["openrouter", "venice"];

export function openProviderModal() {
	var m = els();
	m.modal.classList.remove("hidden");
	m.title.textContent = "Add LLM";
	m.body.textContent = "Loading...";
	sendRpc("providers.available", {}).then((res) => {
		if (!res?.ok) {
			m.body.textContent = "Failed to load LLM providers.";
			return;
		}
		var providers = res.payload || [];

		providers.sort((a, b) => {
			var aOrder = Number.isFinite(a.uiOrder) ? a.uiOrder : Number.MAX_SAFE_INTEGER;
			var bOrder = Number.isFinite(b.uiOrder) ? b.uiOrder : Number.MAX_SAFE_INTEGER;
			if (aOrder !== bOrder) return aOrder - bOrder;
			return a.displayName.localeCompare(b.displayName);
		});

		m.body.textContent = "";
		providers.forEach((p) => {
			var item = document.createElement("div");
			// Don't gray out configured providers - users can add multiple
			item.className = "provider-item";
			var name = document.createElement("span");
			name.className = "provider-item-name";
			name.textContent = p.displayName;
			item.appendChild(name);

			var badges = document.createElement("div");
			badges.className = "badge-row";

			if (p.configured) {
				var check = document.createElement("span");
				check.className = "provider-item-badge configured";
				check.textContent = "configured";
				badges.appendChild(check);
			}

			var badge = document.createElement("span");
			badge.className = `provider-item-badge ${p.authType}`;
			if (p.authType === "oauth") {
				badge.textContent = "OAuth";
			} else if (p.authType === "local") {
				badge.textContent = "Local";
			} else {
				badge.textContent = "API Key";
			}
			badges.appendChild(badge);
			item.appendChild(badges);

			item.addEventListener("click", () => {
				if (p.authType === "api-key") showApiKeyForm(p);
				else if (p.authType === "oauth") showOAuthFlow(p);
				else if (p.authType === "local") showLocalModelFlow(p);
			});
			m.body.appendChild(item);
		});
	});
}

export function closeProviderModal() {
	els().modal.classList.add("hidden");
}

function setFormError(errorPanel, message) {
	if (!errorPanel) return;
	if (!message) {
		errorPanel.style.display = "none";
		errorPanel.textContent = "";
		return;
	}
	errorPanel.textContent = `Error: ${message}`;
	errorPanel.style.display = "";
}

export function showApiKeyForm(provider) {
	var m = els();
	m.title.textContent = provider.displayName;
	m.body.textContent = "";

	var form = document.createElement("div");
	form.className = "provider-key-form";

	// Check if this provider supports custom endpoint
	var supportsEndpoint = OPENAI_COMPATIBLE_PROVIDERS.includes(provider.name);

	// API Key field
	var keyLabel = document.createElement("label");
	keyLabel.className = "text-xs text-[var(--muted)]";
	keyLabel.textContent = "API Key";
	form.appendChild(keyLabel);

	var keyInp = document.createElement("input");
	keyInp.className = "provider-key-input";
	keyInp.type = "password";
	keyInp.placeholder = provider.keyOptional ? "(optional)" : "sk-...";
	form.appendChild(keyInp);

	var errorPanel = document.createElement("div");
	errorPanel.className = "alert-error-text text-[var(--error)] whitespace-pre-line";
	errorPanel.style.display = "none";
	form.appendChild(errorPanel);

	var keyHelp = providerApiKeyHelp(provider);
	if (keyHelp) {
		var keyHelpLine = document.createElement("div");
		keyHelpLine.className = "text-xs text-[var(--muted)] mt-1";
		if (keyHelp.url) {
			keyHelpLine.append(`${keyHelp.text} `);
			var keyLink = document.createElement("a");
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
	var endpointInp = null;
	if (supportsEndpoint) {
		var endpointLabel = document.createElement("label");
		endpointLabel.className = "text-xs text-[var(--muted)]";
		endpointLabel.style.marginTop = "8px";
		endpointLabel.textContent = "Endpoint (optional)";
		form.appendChild(endpointLabel);

		endpointInp = document.createElement("input");
		endpointInp.className = "provider-key-input";
		endpointInp.type = "text";
		endpointInp.placeholder = provider.defaultBaseUrl || "https://api.example.com/v1";
		form.appendChild(endpointInp);

		var hint = document.createElement("div");
		hint.className = "text-xs text-[var(--muted)]";
		hint.style.marginTop = "2px";
		hint.textContent = "Leave empty to use the default endpoint.";
		form.appendChild(hint);
	}

	// Model field for bring-your-own-model providers
	var modelInp = null;
	var needsModel = BYOM_PROVIDERS.includes(provider.name);
	if (needsModel) {
		var modelLabel = document.createElement("label");
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

	var btns = document.createElement("div");
	btns.className = "btn-row";
	btns.style.marginTop = "12px";

	var backBtn = document.createElement("button");
	backBtn.className = "provider-btn provider-btn-secondary";
	backBtn.textContent = "Back";
	backBtn.addEventListener("click", openProviderModal);
	btns.appendChild(backBtn);

	var saveBtn = document.createElement("button");
	saveBtn.className = "provider-btn";
	saveBtn.textContent = "Save & Validate";
	saveBtn.addEventListener("click", () => {
		var key = keyInp.value.trim();
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
		saveBtn.textContent = "Validating...";
		setFormError(errorPanel, null);

		var keyVal = key || provider.name;
		var endpointVal = endpointInp?.value.trim() || null;
		var modelVal = modelInp?.value.trim() || null;

		validateProviderKey(provider.name, keyVal, endpointVal, modelVal)
			.then((result) => {
				if (!result.valid) {
					saveBtn.disabled = false;
					saveBtn.textContent = "Save & Validate";
					setFormError(errorPanel, result.error || "Validation failed. Please check your credentials.");
					return;
				}

				// BYOM providers already tested the specific model — save directly.
				if (needsModel) {
					saveAndFinishProvider(provider, keyVal, endpointVal, modelVal, null, false);
					return;
				}

				// Regular providers — show model selector.
				showModelSelector(provider, result.models || [], keyVal, endpointVal, modelVal);
			})
			.catch((err) => {
				saveBtn.disabled = false;
				saveBtn.textContent = "Save & Validate";
				setFormError(errorPanel, err?.message || "Validation failed.");
			});
	});
	btns.appendChild(saveBtn);
	form.appendChild(btns);
	m.body.appendChild(form);
	keyInp.focus();
}

function showModelSelector(provider, models, keyVal, endpointVal, modelVal, skipSave) {
	var m = els();
	m.title.textContent = `${provider.displayName} — Select Model`;
	m.body.textContent = "";

	var wrapper = document.createElement("div");
	wrapper.className = "provider-key-form";

	var label = document.createElement("div");
	label.className = "text-xs font-medium text-[var(--text-strong)] mb-2";
	label.textContent = "Choose a model to use";
	wrapper.appendChild(label);

	// Search input when >5 models
	var searchInp = null;
	if (models.length > 5) {
		searchInp = document.createElement("input");
		searchInp.type = "text";
		searchInp.className = "provider-key-input w-full text-xs mb-2";
		searchInp.placeholder = "Search models\u2026";
		wrapper.appendChild(searchInp);
	}

	var list = document.createElement("div");
	list.className = "flex flex-col gap-2 max-h-56 overflow-y-auto";
	wrapper.appendChild(list);

	var errorArea = document.createElement("div");
	errorArea.className = "alert-error-text text-[var(--error)] whitespace-pre-line";
	errorArea.style.display = "none";
	wrapper.appendChild(errorArea);

	var selectedCard = null;

	function renderCards(filter) {
		list.textContent = "";
		var filtered = models;
		if (filter) {
			var q = filter.toLowerCase();
			filtered = models.filter((mdl) => mdl.displayName.toLowerCase().includes(q) || mdl.id.toLowerCase().includes(q));
		}
		if (filtered.length === 0) {
			var empty = document.createElement("div");
			empty.className = "text-xs text-[var(--muted)] py-4 text-center";
			empty.textContent = "No models match your search.";
			list.appendChild(empty);
			return;
		}
		filtered.forEach((mdl) => {
			var card = document.createElement("div");
			card.className = "model-card";

			var header = document.createElement("div");
			header.className = "flex items-center justify-between";

			var name = document.createElement("span");
			name.className = "text-sm font-medium text-[var(--text)]";
			name.textContent = mdl.displayName;
			header.appendChild(name);

			var badges = document.createElement("div");
			badges.className = "flex gap-2";

			if (mdl.supportsTools) {
				var toolsBadge = document.createElement("span");
				toolsBadge.className = "recommended-badge";
				toolsBadge.textContent = "Tools";
				badges.appendChild(toolsBadge);
			}

			header.appendChild(badges);
			card.appendChild(header);

			var idLine = document.createElement("div");
			idLine.className = "text-xs text-[var(--muted)] mt-1 font-mono";
			idLine.textContent = mdl.id;
			card.appendChild(idLine);

			card.addEventListener("click", () => {
				if (selectedCard) return; // prevent double-click
				// Deselect all, select this one
				for (var c of list.querySelectorAll(".model-card")) c.classList.remove("selected");
				card.classList.add("selected");
				selectedCard = card;

				// Show testing state
				var testBadge = document.createElement("span");
				testBadge.className = "tier-badge";
				testBadge.textContent = "Testing\u2026";
				badges.appendChild(testBadge);
				errorArea.style.display = "none";

				saveAndFinishProvider(provider, keyVal, endpointVal, modelVal, mdl.id, !!skipSave);
			});

			list.appendChild(card);
		});
	}

	renderCards(null);

	if (searchInp) {
		searchInp.addEventListener("input", () => {
			selectedCard = null;
			renderCards(searchInp.value.trim());
		});
	}

	// Buttons
	var btns = document.createElement("div");
	btns.className = "btn-row mt-3";

	var backBtn = document.createElement("button");
	backBtn.className = "provider-btn provider-btn-secondary";
	backBtn.textContent = "Back";
	backBtn.addEventListener("click", () => {
		if (skipSave) {
			// OAuth flow — go back to provider list
			openProviderModal();
		} else {
			showApiKeyForm(provider);
		}
	});
	btns.appendChild(backBtn);
	wrapper.appendChild(btns);

	// Expose error area for saveAndFinishProvider to use
	wrapper._errorArea = errorArea;
	wrapper._resetSelection = () => {
		selectedCard = null;
		renderCards(searchInp?.value.trim() || null);
	};

	m.body.appendChild(wrapper);
}

function saveAndFinishProvider(provider, keyVal, endpointVal, modelVal, selectedModelId, skipSave) {
	var m = els();
	var effectiveModelVal = provider.keyOptional && selectedModelId ? selectedModelId : modelVal;

	function showError(msg) {
		var wrapper = m.body.querySelector(".provider-key-form");
		if (wrapper?._errorArea) {
			setFormError(wrapper._errorArea, msg);
			if (wrapper._resetSelection) wrapper._resetSelection();
		}
	}

	var savePromise = skipSave
		? Promise.resolve({ ok: true })
		: sendRpc("providers.save_key", buildSavePayload(provider.name, keyVal, endpointVal, effectiveModelVal));

	savePromise
		.then(async (res) => {
			if (!res?.ok) {
				showError(res?.error?.message || "Failed to save credentials.");
				return;
			}

			if (selectedModelId) {
				var testResult = await testModel(selectedModelId);
				var modelServiceUnavailable = !testResult.ok && isModelServiceNotConfigured(testResult.error || "");
				if (!(testResult.ok || modelServiceUnavailable)) {
					showError(testResult.error || "Model test failed. Try another model.");
					return;
				}
				await sendRpc("providers.save_model", { provider: provider.name, model: selectedModelId });
				if (modelServiceUnavailable) {
					console.warn("models.test unavailable in provider settings, saved selected model without probe");
				}
				localStorage.setItem("moltis-model", selectedModelId);
			}

			// Success
			m.body.textContent = "";
			var status = document.createElement("div");
			status.className = "provider-status";
			status.textContent = `${provider.displayName} configured successfully!`;
			m.body.appendChild(status);
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			setTimeout(closeProviderModal, 1500);
		})
		.catch((err) => {
			showError(err?.message || "Failed to save credentials.");
		});
}

function buildSavePayload(providerName, keyVal, endpointVal, modelVal) {
	var payload = { provider: providerName, apiKey: keyVal };
	if (endpointVal) payload.baseUrl = endpointVal;
	if (modelVal) payload.model = modelVal;
	return payload;
}

export function showOAuthFlow(provider) {
	var m = els();
	m.title.textContent = provider.displayName;
	m.body.textContent = "";

	var wrapper = document.createElement("div");
	wrapper.className = "provider-key-form";

	var desc = document.createElement("div");
	desc.className = "text-xs text-[var(--muted)]";
	desc.textContent = `Click below to authenticate with ${provider.displayName} via OAuth.`;
	wrapper.appendChild(desc);

	var btns = document.createElement("div");
	btns.className = "btn-row";

	var backBtn = document.createElement("button");
	backBtn.className = "provider-btn provider-btn-secondary";
	backBtn.textContent = "Back";
	backBtn.addEventListener("click", openProviderModal);
	btns.appendChild(backBtn);

	var connectBtn = document.createElement("button");
	connectBtn.className = "provider-btn";
	connectBtn.textContent = "Connect";
	connectBtn.addEventListener("click", () => {
		connectBtn.disabled = true;
		connectBtn.textContent = "Starting...";
		startProviderOAuth(provider.name).then((result) => {
			if (result.status === "already") {
				connectBtn.textContent = "Connected";
				desc.classList.remove("text-error");
				desc.textContent = `${provider.displayName} is already connected (imported credentials found).`;
				showOAuthModelSelector(provider);
			} else if (result.status === "browser") {
				window.open(result.authUrl, "_blank");
				connectBtn.textContent = "Waiting for auth...";
				pollOAuthStatus(provider);
			} else if (result.status === "device") {
				connectBtn.textContent = "Waiting for auth...";
				desc.classList.remove("text-error");
				desc.textContent = "";
				var linkEl = document.createElement("a");
				linkEl.href = result.verificationUrl;
				linkEl.target = "_blank";
				linkEl.className = "oauth-link";
				linkEl.textContent = result.verificationUrl;
				var codeEl = document.createElement("strong");
				codeEl.textContent = result.userCode;
				desc.appendChild(document.createTextNode("Go to "));
				desc.appendChild(linkEl);
				desc.appendChild(document.createTextNode(" and enter code: "));
				desc.appendChild(codeEl);
				pollOAuthStatus(provider);
			} else {
				connectBtn.disabled = false;
				connectBtn.textContent = "Connect";
				desc.textContent = result.error || "Failed to start OAuth";
				desc.classList.add("text-error");
			}
		});
	});
	btns.appendChild(connectBtn);
	wrapper.appendChild(btns);
	m.body.appendChild(wrapper);
}

function pollOAuthStatus(provider) {
	var m = els();
	var attempts = 0;
	var maxAttempts = 60;
	var timer = setInterval(() => {
		attempts++;
		if (attempts > maxAttempts) {
			clearInterval(timer);
			m.body.textContent = "";
			var timeout = document.createElement("div");
			timeout.className = "text-xs text-[var(--error)]";
			timeout.textContent = "OAuth timed out. Please try again.";
			m.body.appendChild(timeout);
			return;
		}
		sendRpc("providers.oauth.status", { provider: provider.name }).then((res) => {
			if (res?.ok && res.payload && res.payload.authenticated) {
				clearInterval(timer);
				showOAuthModelSelector(provider);
			}
		});
	}, 2000);
}

function showOAuthModelSelector(provider) {
	sendRpc("models.list", {}).then((modelsRes) => {
		var allModels = modelsRes?.ok ? modelsRes.payload || [] : [];
		var needle = provider.name.replace(/-/g, "").toLowerCase();
		var provModels = allModels.filter((entry) => entry.provider?.toLowerCase().replace(/-/g, "").includes(needle));

		if (provModels.length > 0) {
			var mapped = provModels.map((entry) => ({
				id: entry.id,
				displayName: entry.displayName || entry.id,
				provider: entry.provider,
				supportsTools: entry.supportsTools,
			}));
			showModelSelector(provider, mapped, null, null, null, true);
		} else {
			// No models found yet — trigger detection in background and show success.
			sendRpc("models.detect_supported", {
				background: true,
				reason: "provider_connected",
				provider: provider.name,
			});
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			var modal = els();
			modal.body.textContent = "";
			var status = document.createElement("div");
			status.className = "provider-status";
			status.textContent = `${provider.displayName} connected successfully!`;
			modal.body.appendChild(status);
			setTimeout(closeProviderModal, 1500);
		}
	});
}

// ── Model selector for existing providers (multi-select) ──

export function openModelSelectorForProvider(providerName, providerDisplayName) {
	var m = els();
	m.modal.classList.remove("hidden");
	m.title.textContent = `${providerDisplayName} — Preferred Models`;
	m.body.textContent = "Loading models...";

	Promise.all([sendRpc("models.list", {}), sendRpc("providers.available", {})]).then(([modelsRes, providersRes]) => {
		var allModels = modelsRes?.ok ? modelsRes.payload || [] : [];
		var needle = providerName.replace(/-/g, "").toLowerCase();
		var provModels = allModels.filter((entry) => entry.provider?.toLowerCase().replace(/-/g, "").includes(needle));

		if (provModels.length === 0) {
			m.body.textContent = "";
			var wrapper = document.createElement("div");
			wrapper.className = "provider-key-form";
			var msg = document.createElement("div");
			msg.className = "text-xs text-[var(--muted)] py-4 text-center";
			msg.textContent = "No models available yet. Try running Detect All Models first.";
			wrapper.appendChild(msg);
			var btns = document.createElement("div");
			btns.className = "btn-row mt-3";
			var closeBtn = document.createElement("button");
			closeBtn.className = "provider-btn provider-btn-secondary";
			closeBtn.textContent = "Close";
			closeBtn.addEventListener("click", closeProviderModal);
			btns.appendChild(closeBtn);
			wrapper.appendChild(btns);
			m.body.appendChild(wrapper);
			return;
		}

		// Get saved preferred models for this provider.
		var savedModels = new Set();
		if (providersRes?.ok) {
			var providerMeta = (providersRes.payload || []).find((p) => p.name === providerName);
			if (providerMeta?.models) {
				for (var sm of providerMeta.models) savedModels.add(sm);
			}
		}

		var mapped = provModels.map((entry) => ({
			id: entry.id,
			displayName: entry.displayName || entry.id,
			provider: entry.provider,
			supportsTools: entry.supportsTools,
			createdAt: entry.createdAt || 0,
		}));
		showMultiModelSelector(providerName, providerDisplayName, mapped, savedModels);
	});
}

function showMultiModelSelector(providerName, providerDisplayName, models, savedModels) {
	var m = els();
	m.title.textContent = `${providerDisplayName} — Preferred Models`;
	m.body.textContent = "";

	var selectedIds = new Set(savedModels);

	var wrapper = document.createElement("div");
	wrapper.className = "provider-key-form flex flex-col min-h-0 flex-1";

	var label = document.createElement("div");
	label.className = "text-xs font-medium text-[var(--text-strong)] mb-1 shrink-0";
	label.textContent = "Select models to pin at the top of the dropdown";
	wrapper.appendChild(label);

	var hint = document.createElement("div");
	hint.className = "text-xs text-[var(--muted)] mb-2 shrink-0";
	hint.textContent = "Selected models appear first in the session model selector.";
	wrapper.appendChild(hint);

	// Search input when >5 models
	var searchInp = null;
	if (models.length > 5) {
		searchInp = document.createElement("input");
		searchInp.type = "text";
		searchInp.className = "provider-key-input w-full text-xs mb-2 shrink-0";
		searchInp.placeholder = "Search models\u2026";
		wrapper.appendChild(searchInp);
	}

	var list = document.createElement("div");
	list.className = "flex flex-col gap-1 overflow-y-auto flex-1 min-h-0";
	wrapper.appendChild(list);

	var statusArea = document.createElement("div");
	statusArea.className = "text-xs text-[var(--muted)] mt-2 shrink-0";
	wrapper.appendChild(statusArea);

	function updateStatus() {
		var count = selectedIds.size;
		statusArea.textContent = count === 0 ? "No models selected" : `${count} model${count > 1 ? "s" : ""} selected`;
	}

	function modelSortKey(m) {
		return { selected: selectedIds.has(m.id) ? 0 : 1, time: m.createdAt || 0, name: m.displayName || m.id };
	}

	function sortModelsForSelection(items) {
		return [...items].sort((a, b) => {
			var ka = modelSortKey(a);
			var kb = modelSortKey(b);
			return ka.selected - kb.selected || kb.time - ka.time || ka.name.localeCompare(kb.name);
		});
	}

	function renderCards(filter) {
		list.textContent = "";
		var filtered = models;
		if (filter) {
			var q = filter.toLowerCase();
			filtered = models.filter(
				(entry) => entry.displayName.toLowerCase().includes(q) || entry.id.toLowerCase().includes(q),
			);
		}
		if (filtered.length === 0) {
			var empty = document.createElement("div");
			empty.className = "text-xs text-[var(--muted)] py-4 text-center";
			empty.textContent = "No models match your search.";
			list.appendChild(empty);
			return;
		}
		var sorted = sortModelsForSelection(filtered);
		for (var mdl of sorted) {
			var card = document.createElement("div");
			card.className = `model-card ${selectedIds.has(mdl.id) ? "selected" : ""}`;

			var header = document.createElement("div");
			header.className = "flex items-center justify-between";

			var nameSpan = document.createElement("span");
			nameSpan.className = "text-sm font-medium text-[var(--text)] truncate";
			nameSpan.textContent = mdl.displayName;
			header.appendChild(nameSpan);

			var badges = document.createElement("div");
			badges.className = "flex gap-2";
			if (mdl.supportsTools) {
				var toolsBadge = document.createElement("span");
				toolsBadge.className = "recommended-badge";
				toolsBadge.textContent = "Tools";
				badges.appendChild(toolsBadge);
			}
			header.appendChild(badges);
			card.appendChild(header);

			var idLine = document.createElement("div");
			idLine.className = "text-xs text-[var(--muted)] mt-1 font-mono";
			idLine.textContent = mdl.id;
			card.appendChild(idLine);

			// Closure to capture mdl
			((modelId) => {
				card.addEventListener("click", () => {
					if (selectedIds.has(modelId)) {
						selectedIds.delete(modelId);
					} else {
						selectedIds.add(modelId);
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
			renderCards(searchInp.value.trim());
		});
	}

	var errorArea = document.createElement("div");
	errorArea.className = "alert-error-text text-[var(--error)] whitespace-pre-line shrink-0";
	errorArea.style.display = "none";
	wrapper.appendChild(errorArea);

	// Buttons — always visible at the bottom
	var btns = document.createElement("div");
	btns.className = "btn-row mt-3 shrink-0";

	var cancelBtn = document.createElement("button");
	cancelBtn.className = "provider-btn provider-btn-secondary";
	cancelBtn.textContent = "Cancel";
	cancelBtn.addEventListener("click", closeProviderModal);
	btns.appendChild(cancelBtn);

	var saveBtn = document.createElement("button");
	saveBtn.className = "provider-btn";
	saveBtn.textContent = "Save";
	saveBtn.addEventListener("click", () => {
		saveBtn.disabled = true;
		saveBtn.textContent = "Saving\u2026";
		errorArea.style.display = "none";

		sendRpc("providers.save_models", { provider: providerName, models: Array.from(selectedIds) })
			.then((res) => {
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
			.catch((err) => {
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

export function showLocalModelFlow(provider) {
	var m = els();
	m.title.textContent = provider.displayName;
	m.body.textContent = "Loading system info...";

	// Fetch system info first
	sendRpc("providers.local.system_info", {}).then((sysRes) => {
		if (!sysRes?.ok) {
			m.body.textContent = sysRes?.error?.message || "Failed to get system info";
			return;
		}
		var sysInfo = sysRes.payload;

		// Fetch available models
		sendRpc("providers.local.models", {}).then((modelsRes) => {
			if (!modelsRes?.ok) {
				m.body.textContent = modelsRes?.error?.message || "Failed to get models";
				return;
			}
			var modelsData = modelsRes.payload;
			renderLocalModelSelection(provider, sysInfo, modelsData);
		});
	});
}

// Store the selected backend for model configuration
var selectedBackend = null;

function renderLocalModelSelection(provider, sysInfo, modelsData) {
	var m = els();
	m.body.textContent = "";

	// Initialize selected backend to recommended
	selectedBackend = sysInfo.recommendedBackend || "GGUF";

	var wrapper = document.createElement("div");
	wrapper.className = "provider-key-form";

	// System info section
	var sysSection = document.createElement("div");
	sysSection.className = "flex flex-col gap-2 mb-4";

	var sysTitle = document.createElement("div");
	sysTitle.className = "text-xs font-medium text-[var(--text-strong)]";
	sysTitle.textContent = "System Info";
	sysSection.appendChild(sysTitle);

	var sysDetails = document.createElement("div");
	sysDetails.className = "flex gap-3 text-xs text-[var(--muted)]";

	var ramSpan = document.createElement("span");
	ramSpan.textContent = `RAM: ${sysInfo.totalRamGb}GB`;
	sysDetails.appendChild(ramSpan);

	var tierSpan = document.createElement("span");
	tierSpan.textContent = `Tier: ${sysInfo.memoryTier}`;
	sysDetails.appendChild(tierSpan);

	if (sysInfo.hasGpu) {
		var gpuSpan = document.createElement("span");
		gpuSpan.className = "text-[var(--ok)]";
		gpuSpan.textContent = "GPU available";
		sysDetails.appendChild(gpuSpan);
	}

	sysSection.appendChild(sysDetails);
	wrapper.appendChild(sysSection);

	// Backend selector (show on Apple Silicon where both GGUF and MLX are options)
	var backends = sysInfo.availableBackends || [];
	if (sysInfo.isAppleSilicon && backends.length > 0) {
		var backendSection = document.createElement("div");
		backendSection.className = "flex flex-col gap-2 mb-4";

		var backendLabel = document.createElement("div");
		backendLabel.className = "text-xs font-medium text-[var(--text-strong)]";
		backendLabel.textContent = "Inference Backend";
		backendSection.appendChild(backendLabel);

		var backendCards = document.createElement("div");
		backendCards.className = "flex flex-col gap-2";

		// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: backend card rendering with many conditions
		backends.forEach((b) => {
			var card = document.createElement("div");
			card.className = "backend-card";
			if (!b.available) card.className += " disabled";
			if (b.id === selectedBackend) card.className += " selected";
			card.dataset.backendId = b.id;

			var header = document.createElement("div");
			header.className = "flex items-center justify-between";

			var name = document.createElement("span");
			name.className = "backend-name text-sm font-medium text-[var(--text)]";
			name.textContent = b.name;
			header.appendChild(name);

			var badges = document.createElement("div");
			badges.className = "flex gap-2";

			if (b.id === sysInfo.recommendedBackend && b.available) {
				var recBadge = document.createElement("span");
				recBadge.className = "recommended-badge";
				recBadge.textContent = "Recommended";
				badges.appendChild(recBadge);
			}

			if (!b.available) {
				var unavailBadge = document.createElement("span");
				unavailBadge.className = "tier-badge";
				unavailBadge.textContent = "Not installed";
				badges.appendChild(unavailBadge);
			}

			header.appendChild(badges);
			card.appendChild(header);

			var desc = document.createElement("div");
			desc.className = "text-xs text-[var(--muted)] mt-1";
			desc.textContent = b.description;
			card.appendChild(desc);

			// Show install instructions for unavailable backends
			if (!b.available && b.id === "MLX") {
				var cmds = b.installCommands || ["pip install mlx-lm"];
				var tpl = document.getElementById("tpl-install-hint");
				var hint = tpl.content.cloneNode(true).firstElementChild;
				var label = hint.querySelector("[data-install-label]");
				var container = hint.querySelector("[data-install-commands]");

				label.textContent = cmds.length === 1 ? "Install with:" : "Install with any of:";

				var cmdTpl = document.getElementById("tpl-install-cmd");
				cmds.forEach((c) => {
					var cmdEl = cmdTpl.content.cloneNode(true).firstElementChild;
					cmdEl.textContent = c;
					container.appendChild(cmdEl);
				});

				card.appendChild(hint);
			}

			if (b.available) {
				card.addEventListener("click", () => {
					// Deselect all cards
					backendCards.querySelectorAll(".backend-card").forEach((c) => {
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
		var backendDiv = document.createElement("div");
		backendDiv.className = "text-xs text-[var(--muted)] mb-4";
		backendDiv.innerHTML = `<span class="font-medium">Backend:</span> ${sysInfo.backendNote}`;
		wrapper.appendChild(backendDiv);
	}

	// Models section
	var modelsTitle = document.createElement("div");
	modelsTitle.className = "text-xs font-medium text-[var(--text-strong)] mb-2";
	modelsTitle.textContent = "Select a Model";
	wrapper.appendChild(modelsTitle);

	var modelsList = document.createElement("div");
	modelsList.className = "flex flex-col gap-2";
	modelsList.id = "local-model-list";

	// Helper to render models filtered by backend
	function renderModelsForBackend(backend) {
		modelsList.innerHTML = "";
		var recommended = modelsData.recommended || [];
		var filtered = recommended.filter((mdl) => mdl.backend === backend);
		if (filtered.length === 0) {
			var empty = document.createElement("div");
			empty.className = "text-xs text-[var(--muted)] py-4 text-center";
			empty.textContent = `No models available for ${backend}`;
			modelsList.appendChild(empty);
			return;
		}
		filtered.forEach((model) => {
			var card = createModelCard(model, provider, sysInfo.totalRamGb);
			modelsList.appendChild(card);
		});
	}

	// Initial render with selected backend
	renderModelsForBackend(selectedBackend);

	// Store render function for backend card click handlers
	wrapper._renderModelsForBackend = renderModelsForBackend;

	wrapper.appendChild(modelsList);

	// HuggingFace search section
	var searchSection = document.createElement("div");
	searchSection.className = "flex flex-col gap-2 mt-4 pt-4 border-t border-[var(--border)]";

	var searchLabel = document.createElement("div");
	searchLabel.className = "text-xs font-medium text-[var(--text-strong)]";
	searchLabel.textContent = "Search HuggingFace";
	searchSection.appendChild(searchLabel);

	var searchRow = document.createElement("div");
	searchRow.className = "flex gap-2";

	var searchInput = document.createElement("input");
	searchInput.type = "text";
	searchInput.placeholder = "Search models...";
	searchInput.className = "provider-input flex-1";
	searchRow.appendChild(searchInput);

	var searchBtn = document.createElement("button");
	searchBtn.className = "provider-btn provider-btn-secondary";
	searchBtn.textContent = "Search";
	searchRow.appendChild(searchBtn);

	searchSection.appendChild(searchRow);

	var searchResults = document.createElement("div");
	searchResults.className = "flex flex-col gap-2 max-h-48 overflow-y-auto";
	searchResults.id = "hf-search-results";
	searchSection.appendChild(searchResults);

	// Search handler
	var doSearch = async () => {
		var query = searchInput.value.trim();
		if (!query) return;
		searchBtn.disabled = true;
		searchBtn.textContent = "Searching...";
		searchResults.innerHTML = "";
		var res = await sendRpc("providers.local.search_hf", {
			query: query,
			backend: selectedBackend,
			limit: 15,
		});
		searchBtn.disabled = false;
		searchBtn.textContent = "Search";
		if (!(res?.ok && res.payload?.results?.length)) {
			searchResults.innerHTML = '<div class="text-xs text-[var(--muted)] py-2">No results found</div>';
			return;
		}
		res.payload.results.forEach((result) => {
			var card = createHfSearchResultCard(result, provider);
			searchResults.appendChild(card);
		});
	};

	searchBtn.addEventListener("click", doSearch);
	searchInput.addEventListener("keydown", (e) => {
		if (e.key === "Enter") doSearch();
	});

	// Auto-search with debounce when user stops typing
	var searchTimeout = null;
	searchInput.addEventListener("input", () => {
		if (searchTimeout) clearTimeout(searchTimeout);
		var query = searchInput.value.trim();
		if (query.length >= 2) {
			searchTimeout = setTimeout(doSearch, 500);
		}
	});

	wrapper.appendChild(searchSection);

	// Custom repo section
	var customSection = document.createElement("div");
	customSection.className = "flex flex-col gap-2 mt-4 pt-4 border-t border-[var(--border)]";

	var customLabel = document.createElement("div");
	customLabel.className = "text-xs font-medium text-[var(--text-strong)]";
	customLabel.textContent = "Or enter HuggingFace repo URL";
	customSection.appendChild(customLabel);

	var customRow = document.createElement("div");
	customRow.className = "flex gap-2";

	var customInput = document.createElement("input");
	customInput.type = "text";
	customInput.placeholder = selectedBackend === "MLX" ? "mlx-community/Model-Name" : "TheBloke/Model-GGUF";
	customInput.className = "provider-input flex-1";
	customRow.appendChild(customInput);

	var customBtn = document.createElement("button");
	customBtn.className = "provider-btn";
	customBtn.textContent = "Use";
	customRow.appendChild(customBtn);

	customSection.appendChild(customRow);

	// GGUF filename input (only for GGUF backend)
	var filenameRow = document.createElement("div");
	filenameRow.className = "flex gap-2";
	filenameRow.style.display = selectedBackend === "GGUF" ? "flex" : "none";

	var filenameInput = document.createElement("input");
	filenameInput.type = "text";
	filenameInput.placeholder = "model-file.gguf (required for GGUF)";
	filenameInput.className = "provider-input flex-1";
	filenameRow.appendChild(filenameInput);

	customSection.appendChild(filenameRow);

	// Update filename visibility when backend changes
	wrapper._updateFilenameVisibility = (backend) => {
		filenameRow.style.display = backend === "GGUF" ? "flex" : "none";
		customInput.placeholder = backend === "MLX" ? "mlx-community/Model-Name" : "TheBloke/Model-GGUF";
	};

	// Custom repo handler
	customBtn.addEventListener("click", async () => {
		var repo = customInput.value.trim();
		if (!repo) return;

		var params = {
			hfRepo: repo,
			backend: selectedBackend,
		};
		if (selectedBackend === "GGUF") {
			var filename = filenameInput.value.trim();
			if (!filename) {
				filenameInput.focus();
				return;
			}
			params.hfFilename = filename;
		}

		customBtn.disabled = true;
		customBtn.textContent = "Configuring...";
		var res = await sendRpc("providers.local.configure_custom", params);
		customBtn.disabled = false;
		customBtn.textContent = "Use";

		if (res?.ok) {
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			showModelDownloadProgress({ id: res.payload.modelId, displayName: repo }, provider);
		} else {
			var err = res?.error?.message || "Failed to configure model";
			searchResults.innerHTML = `<div class="text-xs text-[var(--error)] py-2">${err}</div>`;
		}
	});

	wrapper.appendChild(customSection);

	// Back button
	var btns = document.createElement("div");
	btns.className = "btn-row mt-4";

	var backBtn = document.createElement("button");
	backBtn.className = "provider-btn provider-btn-secondary";
	backBtn.textContent = "Back";
	backBtn.addEventListener("click", openProviderModal);
	btns.appendChild(backBtn);
	wrapper.appendChild(btns);

	m.body.appendChild(wrapper);
}

// Create a card for HuggingFace search result
function createHfSearchResultCard(model, _provider) {
	var card = document.createElement("div");
	card.className = "model-card";

	var header = document.createElement("div");
	header.className = "flex items-center justify-between";

	var name = document.createElement("span");
	name.className = "text-sm font-medium text-[var(--text)]";
	name.textContent = model.displayName;
	header.appendChild(name);

	var stats = document.createElement("div");
	stats.className = "flex gap-2 text-xs text-[var(--muted)]";
	if (model.downloads) {
		var dl = document.createElement("span");
		dl.textContent = `↓${formatDownloads(model.downloads)}`;
		stats.appendChild(dl);
	}
	if (model.likes) {
		var likes = document.createElement("span");
		likes.textContent = `♥${model.likes}`;
		stats.appendChild(likes);
	}
	header.appendChild(stats);

	card.appendChild(header);

	var repo = document.createElement("div");
	repo.className = "text-xs text-[var(--muted)] mt-1";
	repo.textContent = model.id;
	card.appendChild(repo);

	card.addEventListener("click", async () => {
		// Prevent multiple clicks
		if (card.dataset.configuring) return;
		card.dataset.configuring = "true";

		var params = {
			hfRepo: model.id,
			backend: model.backend,
		};
		// For GGUF, we'd need to fetch the file list - for now, prompt user
		if (model.backend === "GGUF") {
			var filename = prompt("Enter the GGUF filename (e.g., model-q4_k_m.gguf):");
			if (!filename) {
				delete card.dataset.configuring;
				return;
			}
			params.hfFilename = filename;
		}
		card.style.opacity = "0.5";
		card.style.pointerEvents = "none";

		// Show configuring state in modal
		var m = els();
		m.body.innerHTML = "";
		var status = document.createElement("div");
		status.className = "provider-key-form";
		status.innerHTML = `<div class="text-sm text-[var(--text)]">Configuring ${model.displayName}...</div>`;
		m.body.appendChild(status);

		var res = await sendRpc("providers.local.configure_custom", params);
		if (res?.ok) {
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			status.innerHTML = `<div class="provider-status">${model.displayName} configured!</div>`;
			setTimeout(closeProviderModal, 1500);
		} else {
			var err = res?.error?.message || "Failed to configure model";
			status.innerHTML = `<div class="text-sm text-[var(--error)]">${err}</div>`;
		}
	});

	return card;
}

// Format download count (e.g., 1234567 -> "1.2M")
function formatDownloads(n) {
	if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
	if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
	return n.toString();
}

function createModelCard(model, provider, totalRamGb) {
	var card = document.createElement("div");
	card.className = "model-card";
	var detectedRamGb = Number.isFinite(totalRamGb) ? totalRamGb : 0;
	var hasEnoughRam = detectedRamGb >= model.minRamGb;

	var header = document.createElement("div");
	header.className = "flex items-center justify-between";

	var name = document.createElement("span");
	name.className = "text-sm font-medium text-[var(--text)]";
	name.textContent = model.displayName;
	header.appendChild(name);

	var badges = document.createElement("div");
	badges.className = "flex gap-2";

	var ramBadge = document.createElement("span");
	ramBadge.className = "tier-badge";
	ramBadge.textContent = `${model.minRamGb}GB`;
	badges.appendChild(ramBadge);

	if (model.suggested && hasEnoughRam) {
		var suggestedBadge = document.createElement("span");
		suggestedBadge.className = "recommended-badge";
		suggestedBadge.textContent = "Recommended";
		badges.appendChild(suggestedBadge);
	}

	if (!hasEnoughRam) {
		var insufficientBadge = document.createElement("span");
		insufficientBadge.className = "tier-badge";
		insufficientBadge.textContent = "Insufficient RAM";
		badges.appendChild(insufficientBadge);
	}

	header.appendChild(badges);
	card.appendChild(header);

	var meta = document.createElement("div");
	meta.className = "text-xs text-[var(--muted)] mt-1";
	meta.textContent = `Context: ${(model.contextWindow / 1000).toFixed(0)}k tokens`;
	card.appendChild(meta);

	if (!hasEnoughRam) {
		card.classList.add("disabled");
		var warning = document.createElement("div");
		warning.className = "text-xs text-[var(--error)] mt-1";
		warning.textContent = `You do not have enough RAM for this model (${detectedRamGb}GB detected, ${model.minRamGb}GB required).`;
		card.appendChild(warning);
		return card;
	}

	card.addEventListener("click", () => selectLocalModel(model, provider));

	return card;
}

function selectLocalModel(model, provider) {
	var m = els();
	m.body.textContent = "";

	var wrapper = document.createElement("div");
	wrapper.className = "provider-key-form";

	var status = document.createElement("div");
	status.className = "text-sm text-[var(--text)]";
	status.textContent = `Configuring ${model.displayName}...`;
	wrapper.appendChild(status);

	var progress = document.createElement("div");
	progress.className = "download-progress mt-4";

	var progressBar = document.createElement("div");
	progressBar.className = "download-progress-bar";
	progressBar.style.width = "0%";
	progress.appendChild(progressBar);

	var progressText = document.createElement("div");
	progressText.className = "text-xs text-[var(--muted)] mt-2";
	progress.appendChild(progressText);

	wrapper.appendChild(progress);
	m.body.appendChild(wrapper);

	// Subscribe to download progress events
	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: download progress handler with many states
	var off = onEvent("local-llm.download", (payload) => {
		if (payload.modelId !== model.id) return;

		if (payload.error) {
			status.textContent = payload.error;
			status.className = "text-sm text-[var(--error)]";
			off();
			return;
		}

		if (payload.complete) {
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
		if (payload.progress != null) {
			progressBar.style.width = `${payload.progress.toFixed(1)}%`;
			status.textContent = `Downloading ${model.displayName}...`;
		}
		if (payload.downloaded != null) {
			var downloadedMb = (payload.downloaded / (1024 * 1024)).toFixed(1);
			if (payload.total != null) {
				var totalMb = (payload.total / (1024 * 1024)).toFixed(1);
				progressText.textContent = `${downloadedMb} MB / ${totalMb} MB`;
			} else {
				progressText.textContent = `${downloadedMb} MB downloaded`;
			}
		}
	});

	sendRpc("providers.local.configure", { modelId: model.id, backend: selectedBackend }).then((res) => {
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

function pollLocalStatus(model, _provider, statusEl, progressEl, offEvent) {
	var attempts = 0;
	var maxAttempts = 300; // 10 minutes with 2s interval
	var completed = false;
	var timer = setInterval(() => {
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
		sendRpc("providers.local.status", {}).then((res) => {
			if (!res?.ok) return;
			var st = res.payload;

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
