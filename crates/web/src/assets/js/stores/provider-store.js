// ── Provider store (signal-based) ────────────────────────────
//
// Single source of truth for provider configuration data.
// Centralizes signals previously local to page-providers.js.

import { signal } from "@preact/signals";

// ── Signals ──────────────────────────────────────────────────
export var configuredModels = signal([]);
export var providerMeta = signal(new Map());
export var loading = signal(false);
export var detectingModels = signal(false);
export var detectSummary = signal(null);
export var detectError = signal("");
export var detectProgress = signal(null);
export var deletingProvider = signal("");
export var providerActionError = signal("");

// ── Methods ──────────────────────────────────────────────────

export function setConfiguredModels(arr) {
	configuredModels.value = arr || [];
}

export function setProviderMeta(map) {
	providerMeta.value = map;
}

export function setLoading(v) {
	loading.value = v;
}

export function resetDetection() {
	detectingModels.value = false;
	detectSummary.value = null;
	detectError.value = "";
	detectProgress.value = null;
}

export var providerStore = {
	configuredModels,
	providerMeta,
	loading,
	detectingModels,
	detectSummary,
	detectError,
	detectProgress,
	deletingProvider,
	providerActionError,
	setConfiguredModels,
	setProviderMeta,
	setLoading,
	resetDetection,
};
