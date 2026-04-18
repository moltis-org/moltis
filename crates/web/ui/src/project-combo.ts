// ── Session project combo (in chat header) ──────────────────

import { sendRpc } from "./helpers";
import { t } from "./i18n";
import * as S from "./state";
import type { ProjectInfo } from "./types";

export function openProjectDropdown(): void {
	if (!S.projectDropdown) return;
	S.projectDropdown.classList.remove("hidden");
	renderProjectDropdownList();
}

export function closeProjectDropdown(): void {
	if (!S.projectDropdown) return;
	S.projectDropdown.classList.add("hidden");
}

export function renderProjectDropdownList(): void {
	if (!S.projectDropdownList) return;
	S.projectDropdownList.textContent = "";
	// "No project" option
	const none = document.createElement("div");
	none.className = `model-dropdown-item${S.activeProjectId ? "" : " selected"}`;
	const noneLabel = document.createElement("span");
	noneLabel.className = "model-item-label";
	noneLabel.textContent = t("common:sessions.noProject");
	none.appendChild(noneLabel);
	none.addEventListener("click", () => {
		selectProject("", t("common:sessions.noProject"));
	});
	S.projectDropdownList.appendChild(none);
	((S.projects as Array<ProjectInfo & { label?: string }>) || []).forEach((p) => {
		const el = document.createElement("div");
		el.className = `model-dropdown-item${p.id === S.activeProjectId ? " selected" : ""}`;
		const lbl = document.createElement("span");
		lbl.className = "model-item-label";
		lbl.textContent = p.label || p.id;
		el.appendChild(lbl);
		el.addEventListener("click", () => {
			selectProject(p.id, p.label || p.id);
		});
		S.projectDropdownList?.appendChild(el);
	});
}

export function selectProject(id: string, label: string): void {
	S.setActiveProjectId(id);
	localStorage.setItem("moltis-project", S.activeProjectId);
	if (S.projectComboLabel) S.projectComboLabel.textContent = label;
	closeProjectDropdown();
	if (S.connected && S.activeSessionKey) {
		sendRpc("sessions.patch", { key: S.activeSessionKey, projectId: id });
	}
}

export function updateSessionProjectSelect(projectId: string): void {
	if (!S.projectComboLabel) return;
	if (!projectId) {
		S.projectComboLabel.textContent = t("common:sessions.noProject");
		return;
	}
	const proj = ((S.projects as Array<ProjectInfo & { label?: string }>) || []).find((p) => p.id === projectId);
	S.projectComboLabel.textContent = proj ? proj.label || proj.id : projectId;
}

export function renderSessionProjectSelect(): void {
	updateSessionProjectSelect(S.activeProjectId);
}

export function bindProjectComboEvents(): void {
	if (!(S.projectComboBtn && S.projectCombo)) return;
	S.projectComboBtn.addEventListener("click", () => {
		if (S.projectDropdown?.classList.contains("hidden")) {
			openProjectDropdown();
		} else {
			closeProjectDropdown();
		}
	});
}

document.addEventListener("click", (e: MouseEvent) => {
	if (S.projectCombo && !S.projectCombo.contains(e.target as Node)) {
		closeProjectDropdown();
	}
});

window.addEventListener("moltis:locale-changed", () => {
	updateSessionProjectSelect(S.activeProjectId);
	if (S.projectDropdown && !S.projectDropdown.classList.contains("hidden")) {
		renderProjectDropdownList();
	}
});
