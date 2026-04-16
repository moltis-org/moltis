// ── Reasoning effort toggle ──────────────────────────────────
//
// Adds a "brain" combo next to the model selector that lets users
// pick Low / Medium / High reasoning effort for models that support
// extended thinking.  The selected effort is appended as a
// `@reasoning-*` suffix on the model ID sent to the backend — no
// backend changes required.

import { effect } from "@preact/signals";
import { modelStore } from "./stores/model-store.js";

var EFFORTS = [
	{ value: "", label: "Off" },
	{ value: "low", label: "Low" },
	{ value: "medium", label: "Medium" },
	{ value: "high", label: "High" },
];

var reasoningCombo = null;
var reasoningComboBtn = null;
var reasoningComboLabel = null;
var reasoningDropdown = null;
var reasoningDropdownList = null;
var disposeVisibility = null;

function effortLabel(effort) {
	var found = EFFORTS.find((e) => e.value === effort);
	return found ? found.label : "Off";
}

function renderOptions() {
	if (!reasoningDropdownList) return;
	reasoningDropdownList.textContent = "";
	var current = modelStore.reasoningEffort.value;
	for (var opt of EFFORTS) {
		var el = document.createElement("div");
		el.className = "model-dropdown-item";
		if (opt.value === current) el.classList.add("selected");
		var label = document.createElement("span");
		label.className = "model-item-label";
		label.textContent = opt.label;
		el.appendChild(label);
		el.addEventListener("click", selectEffort.bind(null, opt.value));
		reasoningDropdownList.appendChild(el);
	}
}

function selectEffort(effort) {
	modelStore.setReasoningEffort(effort);
	if (reasoningComboLabel) reasoningComboLabel.textContent = effortLabel(effort);
	closeDropdown();
}

function openDropdown() {
	if (!reasoningDropdown) return;
	renderOptions();
	reasoningDropdown.classList.remove("hidden");
}

function closeDropdown() {
	if (!reasoningDropdown) return;
	reasoningDropdown.classList.add("hidden");
}

function handleOutsideClick(e) {
	if (reasoningCombo && !reasoningCombo.contains(e.target)) {
		closeDropdown();
	}
}

export function bindReasoningToggle() {
	reasoningCombo = document.getElementById("reasoningCombo");
	reasoningComboBtn = document.getElementById("reasoningComboBtn");
	reasoningComboLabel = document.getElementById("reasoningComboLabel");
	reasoningDropdown = document.getElementById("reasoningDropdown");
	reasoningDropdownList = document.getElementById("reasoningDropdownList");
	if (!(reasoningCombo && reasoningComboBtn && reasoningDropdownList)) return;

	reasoningComboBtn.addEventListener("click", () => {
		if (reasoningDropdown.classList.contains("hidden")) {
			openDropdown();
		} else {
			closeDropdown();
		}
	});

	document.addEventListener("click", handleOutsideClick);

	// Reactively show/hide the combo based on model reasoning support
	disposeVisibility = effect(() => {
		var show = modelStore.supportsReasoning.value;
		reasoningCombo.classList.toggle("hidden", !show);
		// Reset effort when switching to a non-reasoning model
		if (!show && modelStore.reasoningEffort.value) {
			modelStore.setReasoningEffort("");
		}
		if (reasoningComboLabel) {
			reasoningComboLabel.textContent = effortLabel(modelStore.reasoningEffort.value);
		}
	});
}

/** Restore reasoning toggle state from a session's stored model ID. */
export function restoreReasoningFromModelId(modelId) {
	var parsed = modelStore.parseReasoningSuffix(modelId);
	if (parsed.effort) {
		modelStore.setReasoningEffort(parsed.effort);
	}
	if (reasoningComboLabel) {
		reasoningComboLabel.textContent = effortLabel(modelStore.reasoningEffort.value);
	}
	return parsed.baseId || modelId;
}

export function unbindReasoningToggle() {
	document.removeEventListener("click", handleOutsideClick);
	disposeVisibility?.();
	disposeVisibility = null;
	reasoningCombo = null;
	reasoningComboBtn = null;
	reasoningComboLabel = null;
	reasoningDropdown = null;
	reasoningDropdownList = null;
}
