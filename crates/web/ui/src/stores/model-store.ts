// ── Model store (signal-based) ──────────────────────────────
//
// Single source of truth for model data. Both Preact components
// (auto-subscribe) and imperative code (read .value) can use this.

import { computed, signal } from "@preact/signals";
import { sendRpc } from "../helpers";
import type { ModelInfo, ReasoningSuffix, RpcResponse } from "../types";

export const REASONING_SEP = "@reasoning-";

// ── Signals ──────────────────────────────────────────────────
export const models = signal<ModelInfo[]>([]);
export const selectedModelId = signal<string>(localStorage.getItem("moltis-model") || "");
export const reasoningEffort = signal<string>(localStorage.getItem("moltis-reasoning-effort") || "");

export const selectedModel = computed<ModelInfo | null>(() => {
	const id = selectedModelId.value;
	return models.value.find((m) => m.id === id) || null;
});

/** True when the currently selected model supports extended thinking. */
export const supportsReasoning = computed<boolean>(() => {
	const m = selectedModel.value;
	return !!m?.supportsReasoning;
});

/** Model ID with @reasoning-* suffix when effort is active. */
export const effectiveModelId = computed<string>(() => {
	const id = selectedModelId.value;
	if (!id) return "";
	const effort = reasoningEffort.value;
	if (effort && supportsReasoning.value) return id + REASONING_SEP + effort;
	return id;
});

// ── Helpers ──────────────────────────────────────────────────

/** Parse a model ID that may contain a @reasoning-* suffix.
 *  Returns { baseId, effort } where effort is "" if no suffix. */
export function parseReasoningSuffix(modelId: string): ReasoningSuffix {
	if (!modelId) return { baseId: "", effort: "" };
	const idx = modelId.indexOf(REASONING_SEP);
	if (idx === -1) return { baseId: modelId, effort: "" };
	return { baseId: modelId.substring(0, idx), effort: modelId.substring(idx + REASONING_SEP.length) };
}

/** True if a model ID is a @reasoning-* virtual variant. */
export function isReasoningVariant(modelId: string): boolean {
	return modelId.indexOf(REASONING_SEP) !== -1;
}

// ── Methods ──────────────────────────────────────────────────

/** Replace the full model list (e.g. after fetch or bootstrap). */
export function setAll(arr: ModelInfo[]): void {
	models.value = arr || [];
}

/** Fetch models from the server via RPC. */
export function fetch(): Promise<void> {
	return sendRpc("models.list", {}).then((r) => {
		const res = r as RpcResponse<ModelInfo[]>;
		if (!res?.ok) return;
		setAll(res.payload || []);
		if (models.value.length === 0) return;
		let saved = localStorage.getItem("moltis-model") || "";
		// If the saved model has a reasoning suffix, strip it and restore the effort
		const parsed = parseReasoningSuffix(saved);
		if (parsed.effort) {
			saved = parsed.baseId;
			setReasoningEffort(parsed.effort);
			localStorage.setItem("moltis-model", saved);
		}
		const found = models.value.find((m) => m.id === saved);
		const model = found || models.value[0];
		select(model.id);
		if (!found) localStorage.setItem("moltis-model", model.id);
	});
}

/** Select a model by id. Persists to localStorage. */
export function select(id: string): void {
	selectedModelId.value = id;
}

/** Set the reasoning effort level. Empty string means off. */
export function setReasoningEffort(effort: string): void {
	reasoningEffort.value = effort || "";
	localStorage.setItem("moltis-reasoning-effort", effort || "");
}

/** Look up a model by id. */
export function getById(id: string): ModelInfo | null {
	return models.value.find((m) => m.id === id) || null;
}

export const modelStore = {
	models,
	selectedModelId,
	selectedModel,
	reasoningEffort,
	supportsReasoning,
	effectiveModelId,
	parseReasoningSuffix,
	isReasoningVariant,
	setAll,
	fetch,
	select,
	setReasoningEffort,
	getById,
};
