// ── Model selector ──────────────────────────────────────────

import { sendRpc } from "./helpers.js";
import { showModelNotice } from "./page-chat.js";
import * as S from "./state.js";

function setSessionModel(sessionKey, modelId) {
	sendRpc("sessions.patch", { key: sessionKey, model: modelId });
}

export { setSessionModel };

function updateModelComboLabel(model) {
	if (S.modelComboLabel) S.modelComboLabel.textContent = model.displayName || model.id;
}

export function fetchModels() {
	sendRpc("models.list", {}).then((res) => {
		if (!res?.ok) return;
		S.setModels(res.payload || []);
		if (S.models.length === 0) return;
		var saved = localStorage.getItem("moltis-model") || "";
		var found = S.models.find((m) => m.id === saved);
		var model = found || S.models[0];
		S.setSelectedModelId(model.id);
		updateModelComboLabel(model);
		if (!found) localStorage.setItem("moltis-model", S.selectedModelId);
	});
}

export function selectModel(m) {
	S.setSelectedModelId(m.id);
	updateModelComboLabel(m);
	localStorage.setItem("moltis-model", m.id);
	setSessionModel(S.activeSessionKey, m.id);
	closeModelDropdown();
	// Show notice if model doesn't support tools
	showModelNotice(m);
}

export function openModelDropdown() {
	if (!S.modelDropdown) return;
	S.modelDropdown.classList.remove("hidden");
	S.modelSearchInput.value = "";
	S.setModelIdx(-1);
	renderModelList("");
	requestAnimationFrame(() => {
		if (S.modelSearchInput) S.modelSearchInput.focus();
	});
}

export function closeModelDropdown() {
	if (!S.modelDropdown) return;
	S.modelDropdown.classList.add("hidden");
	if (S.modelSearchInput) S.modelSearchInput.value = "";
	S.setModelIdx(-1);
}

export function renderModelList(query) {
	if (!S.modelDropdownList) return;
	S.modelDropdownList.textContent = "";
	var q = query.toLowerCase();
	var filtered = S.models.filter((m) => {
		var label = (m.displayName || m.id).toLowerCase();
		var provider = (m.provider || "").toLowerCase();
		return !q || label.indexOf(q) !== -1 || provider.indexOf(q) !== -1 || m.id.toLowerCase().indexOf(q) !== -1;
	});
	if (filtered.length === 0) {
		var empty = document.createElement("div");
		empty.className = "model-dropdown-empty";
		empty.textContent = "No matching models";
		S.modelDropdownList.appendChild(empty);
		return;
	}
	filtered.forEach((m) => {
		var el = document.createElement("div");
		el.className = "model-dropdown-item";
		if (m.id === S.selectedModelId) el.classList.add("selected");
		var label = document.createElement("span");
		label.className = "model-item-label";
		label.textContent = m.displayName || m.id;
		el.appendChild(label);
		if (m.provider) {
			var prov = document.createElement("span");
			prov.className = "model-item-provider";
			prov.textContent = m.provider;
			el.appendChild(prov);
		}
		el.addEventListener("click", () => {
			selectModel(m);
		});
		S.modelDropdownList.appendChild(el);
	});
}

function updateModelActive() {
	if (!S.modelDropdownList) return;
	var items = S.modelDropdownList.querySelectorAll(".model-dropdown-item");
	items.forEach((el, i) => {
		el.classList.toggle("kb-active", i === S.modelIdx);
	});
	if (S.modelIdx >= 0 && items[S.modelIdx]) {
		items[S.modelIdx].scrollIntoView({ block: "nearest" });
	}
}

export function bindModelComboEvents() {
	if (!(S.modelComboBtn && S.modelSearchInput && S.modelDropdownList && S.modelCombo)) return;

	S.modelComboBtn.addEventListener("click", () => {
		if (S.modelDropdown.classList.contains("hidden")) {
			openModelDropdown();
		} else {
			closeModelDropdown();
		}
	});

	S.modelSearchInput.addEventListener("input", () => {
		S.setModelIdx(-1);
		renderModelList(S.modelSearchInput.value.trim());
	});

	S.modelSearchInput.addEventListener("keydown", (e) => {
		var items = S.modelDropdownList.querySelectorAll(".model-dropdown-item");
		if (e.key === "ArrowDown") {
			e.preventDefault();
			S.setModelIdx(Math.min(S.modelIdx + 1, items.length - 1));
			updateModelActive();
		} else if (e.key === "ArrowUp") {
			e.preventDefault();
			S.setModelIdx(Math.max(S.modelIdx - 1, 0));
			updateModelActive();
		} else if (e.key === "Enter") {
			e.preventDefault();
			if (S.modelIdx >= 0 && items[S.modelIdx]) {
				items[S.modelIdx].click();
			} else if (items.length === 1) {
				items[0].click();
			}
		} else if (e.key === "Escape") {
			closeModelDropdown();
			S.modelComboBtn.focus();
		}
	});
}

document.addEventListener("click", (e) => {
	if (S.modelCombo && !S.modelCombo.contains(e.target)) {
		closeModelDropdown();
	}
});
