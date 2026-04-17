// ── Provider store (signal-based) ────────────────────────────
//
// Single source of truth for provider configuration data.
// Centralizes signals previously local to page-providers.js.

import { signal } from "@preact/signals";
import type { ConfiguredModel, DetectProgress, DetectSummary, ProviderMeta } from "../types";

// ── Signals ──────────────────────────────────────────────────
export const configuredModels = signal<ConfiguredModel[]>([]);
export const providerMeta = signal<Map<string, ProviderMeta>>(new Map());
export const loading = signal<boolean>(false);
export const detectingModels = signal<boolean>(false);
export const detectSummary = signal<DetectSummary | null>(null);
export const detectError = signal<string>("");
export const detectProgress = signal<DetectProgress | null>(null);
export const deletingProvider = signal<string>("");
export const providerActionError = signal<string>("");

// ── Methods ──────────────────────────────────────────────────

export function setConfiguredModels(arr: ConfiguredModel[]): void {
	configuredModels.value = arr || [];
}

export function setProviderMeta(map: Map<string, ProviderMeta>): void {
	providerMeta.value = map;
}

export function setLoading(v: boolean): void {
	loading.value = v;
}

export function resetDetection(): void {
	detectingModels.value = false;
	detectSummary.value = null;
	detectError.value = "";
	detectProgress.value = null;
}

export const providerStore = {
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
