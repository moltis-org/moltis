// ── Model store (signal-based) ──────────────────────────────
//
// Single source of truth for model data. Both Preact components
// (auto-subscribe) and imperative code (read .value) can use this.

import { computed, signal } from "@preact/signals";
import { sendRpc } from "../helpers.js";

// ── Signals ──────────────────────────────────────────────────
export var models = signal([]);
export var selectedModelId = signal(localStorage.getItem("moltis-model") || "");

export var selectedModel = computed(() => {
	var id = selectedModelId.value;
	return models.value.find((m) => m.id === id) || null;
});

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

/** Look up a model by id. */
export function getById(id) {
	return models.value.find((m) => m.id === id) || null;
}

export var modelStore = { models, selectedModelId, selectedModel, setAll, fetch, select, getById };
