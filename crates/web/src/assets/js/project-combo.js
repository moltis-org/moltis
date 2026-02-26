// ── Session project combo (in chat header) ──────────────────

import { sendRpc } from "./helpers.js";
import { t } from "./i18n.js";
import * as S from "./state.js";

export function openProjectDropdown() {
	if (!S.projectDropdown) return;
	S.projectDropdown.classList.remove("hidden");
	renderProjectDropdownList();
}

export function closeProjectDropdown() {
	if (!S.projectDropdown) return;
	S.projectDropdown.classList.add("hidden");
}

export function renderProjectDropdownList() {
	if (!S.projectDropdownList) return;
	S.projectDropdownList.textContent = "";
	// "No project" option
	var none = document.createElement("div");
	none.className = `model-dropdown-item${S.activeProjectId ? "" : " selected"}`;
	var noneLabel = document.createElement("span");
	noneLabel.className = "model-item-label";
	noneLabel.textContent = t("common:sessions.noProject");
	none.appendChild(noneLabel);
	none.addEventListener("click", () => {
		selectProject("", t("common:sessions.noProject"));
	});
	S.projectDropdownList.appendChild(none);
	(S.projects || []).forEach((p) => {
		var el = document.createElement("div");
		el.className = `model-dropdown-item${p.id === S.activeProjectId ? " selected" : ""}`;
		var lbl = document.createElement("span");
		lbl.className = "model-item-label";
		lbl.textContent = p.label || p.id;
		el.appendChild(lbl);
		el.addEventListener("click", () => {
			selectProject(p.id, p.label || p.id);
		});
		S.projectDropdownList.appendChild(el);
	});
}

export function selectProject(id, label) {
	S.setActiveProjectId(id);
	localStorage.setItem("moltis-project", S.activeProjectId);
	if (S.projectComboLabel) S.projectComboLabel.textContent = label;
	closeProjectDropdown();
	if (S.connected && S.activeSessionKey) {
		sendRpc("sessions.patch", { key: S.activeSessionKey, projectId: id });
	}
}

export function updateSessionProjectSelect(projectId) {
	if (!S.projectComboLabel) return;
	if (!projectId) {
		S.projectComboLabel.textContent = t("common:sessions.noProject");
		return;
	}
	var proj = (S.projects || []).find((p) => p.id === projectId);
	S.projectComboLabel.textContent = proj ? proj.label || proj.id : projectId;
}

export function renderSessionProjectSelect() {
	updateSessionProjectSelect(S.activeProjectId);
}

export function bindProjectComboEvents() {
	if (!(S.projectComboBtn && S.projectCombo)) return;
	S.projectComboBtn.addEventListener("click", () => {
		if (S.projectDropdown.classList.contains("hidden")) {
			openProjectDropdown();
		} else {
			closeProjectDropdown();
		}
	});
}

document.addEventListener("click", (e) => {
	if (S.projectCombo && !S.projectCombo.contains(e.target)) {
		closeProjectDropdown();
	}
});

window.addEventListener("moltis:locale-changed", () => {
	updateSessionProjectSelect(S.activeProjectId);
	if (S.projectDropdown && !S.projectDropdown.classList.contains("hidden")) {
		renderProjectDropdownList();
	}
});
