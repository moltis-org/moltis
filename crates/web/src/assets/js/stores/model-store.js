// ── Model store (signal-based) ──────────────────────────────
//
// Single source of truth for model data. Both Preact components
// (auto-subscribe) and imperative code (read .value) can use this.

import { computed, signal } from "@preact/signals";
import { sendRpc } from "../helpers.js";

export var REASONING_SEP = "@reasoning-";

// ── Signals ──────────────────────────────────────────────────
export var models = signal([]);
export var selectedModelId = signal(localStorage.getItem("moltis-model") || "");
export var reasoningEffort = signal(localStorage.getItem("moltis-reasoning-effort") || "");

export var selectedModel = computed(() => {
	var id = selectedModelId.value;
	return models.value.find((m) => m.id === id) || null;
});

/** True when the currently selected model supports extended thinking. */
export var supportsReasoning = computed(() => {
	var m = selectedModel.value;
	return !!(m && m.supportsReasoning);
});

/** Model ID with @reasoning-* suffix when effort is active. */
export var effectiveModelId = computed(() => {
	var id = selectedModelId.value;
	if (!id) return "";
	var effort = reasoningEffort.value;
	if (effort && supportsReasoning.value) return id + REASONING_SEP + effort;
	return id;
});

// ── Helpers ──────────────────────────────────────────────────

/** Parse a model ID that may contain a @reasoning-* suffix.
 *  Returns { baseId, effort } where effort is "" if no suffix. */
export function parseReasoningSuffix(modelId) {
	if (!modelId) return { baseId: "", effort: "" };
	var idx = modelId.indexOf(REASONING_SEP);
	if (idx === -1) return { baseId: modelId, effort: "" };
	return { baseId: modelId.substring(0, idx), effort: modelId.substring(idx + REASONING_SEP.length) };
}

/** True if a model ID is a @reasoning-* virtual variant. */
export function isReasoningVariant(modelId) {
	return modelId.indexOf(REASONING_SEP) !== -1;
}

// ── Methods ──────────────────────────────────────────────────

/** Replace the full model list (e.g. after fetch or bootstrap). */
export function setAll(arr) {
	models.value = arr || [];
}

/** Fetch models from the server via RPC. */
export function fetch() {
	return sendRpc("models.list", {}).then((res) => {
		if (!res?.ok) return;
		setAll(res.payload || []);
		if (models.value.length === 0) return;
		var saved = localStorage.getItem("moltis-model") || "";
		// If the saved model has a reasoning suffix, strip it and restore the effort
		var parsed = parseReasoningSuffix(saved);
		if (parsed.effort) {
			saved = parsed.baseId;
			setReasoningEffort(parsed.effort);
			localStorage.setItem("moltis-model", saved);
		}
		var found = models.value.find((m) => m.id === saved);
		var model = found || models.value[0];
		select(model.id);
		if (!found) localStorage.setItem("moltis-model", model.id);
	});
}

/** Select a model by id. Persists to localStorage. */
export function select(id) {
	selectedModelId.value = id;
}

/** Set the reasoning effort level. Empty string means off. */
export function setReasoningEffort(effort) {
	reasoningEffort.value = effort || "";
	localStorage.setItem("moltis-reasoning-effort", effort || "");
}

/** Look up a model by id. */
export function getById(id) {
	return models.value.find((m) => m.id === id) || null;
}

export var modelStore = {
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
