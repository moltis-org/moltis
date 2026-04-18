// ── Projects (sidebar filter) ────────────────────────────────

import { t } from "./i18n";
import { updateNavCount } from "./nav-counts";
import { renderSessionProjectSelect } from "./project-combo";
import * as S from "./state";
import { projectStore } from "./stores/project-store";
import type { ProjectInfo } from "./types";

const combo = S.$("projectFilterCombo")!;
const btn = S.$("projectFilterBtn")!;
const label = S.$("projectFilterLabel")!;
const dropdown = S.$("projectFilterDropdown")!;
const list = S.$("projectFilterList")!;
const searchInput = S.$<HTMLInputElement>("projectFilterSearch");
let kbIdx = -1;

export function fetchProjects(): void {
	projectStore.fetch().then(() => {
		const projects = projectStore.projects.value;
		// Dual-write to state.js for backward compat
		S.setProjects(projects);
		renderProjectSelect();
		renderSessionProjectSelect();
		updateNavCount("projects", projects.length);
	});
}

function selectFilter(id: string): void {
	projectStore.setFilterId(id);
	// Dual-write to state.js for backward compat
	S.setProjectFilterId(id);
	const p = projectStore.getById(id);
	label.textContent = p ? (p as ProjectInfo & { label?: string }).label || p.id : t("common:sessions.allSessions");
	closeDropdown();
	document.dispatchEvent(new CustomEvent("moltis:render-session-list"));
}

function closeDropdown(): void {
	dropdown.classList.add("hidden");
	if (searchInput) searchInput.value = "";
	kbIdx = -1;
}

function openDropdown(): void {
	dropdown.classList.remove("hidden");
	kbIdx = -1;
	renderList("");
	requestAnimationFrame(() => {
		if (searchInput) searchInput.focus();
	});
}

function renderList(query: string): void {
	list.textContent = "";
	const q = (query || "").toLowerCase();
	const filterId = projectStore.projectFilterId.value;
	const allProjects = projectStore.projects.value;

	// "All sessions" option -- always shown unless query excludes it
	const allSessionsLabel = t("common:sessions.allSessions");
	if (!q || allSessionsLabel.toLowerCase().indexOf(q) !== -1) {
		const allEl = document.createElement("div");
		allEl.className = "model-dropdown-item";
		if (!filterId) allEl.classList.add("selected");
		const allLabel = document.createElement("span");
		allLabel.className = "model-item-label";
		allLabel.textContent = allSessionsLabel;
		allEl.appendChild(allLabel);
		allEl.addEventListener("click", () => selectFilter(""));
		list.appendChild(allEl);
	}

	const filtered = allProjects.filter((p) => {
		if (!q) return true;
		const name = ((p as ProjectInfo & { label?: string }).label || p.id).toLowerCase();
		return name.indexOf(q) !== -1 || p.id.toLowerCase().indexOf(q) !== -1;
	});

	filtered.forEach((p) => {
		const el = document.createElement("div");
		el.className = "model-dropdown-item";
		if (p.id === filterId) el.classList.add("selected");
		const itemLabel = document.createElement("span");
		itemLabel.className = "model-item-label";
		itemLabel.textContent = (p as ProjectInfo & { label?: string }).label || p.id;
		el.appendChild(itemLabel);
		el.addEventListener("click", () => selectFilter(p.id));
		list.appendChild(el);
	});

	if (list.children.length === 0) {
		const empty = document.createElement("div");
		empty.className = "model-dropdown-empty";
		empty.textContent = t("common:sessions.noMatchingProjects");
		list.appendChild(empty);
	}
}

function updateKbActive(): void {
	const items = list.querySelectorAll<HTMLElement>(".model-dropdown-item");
	items.forEach((el, i) => {
		el.classList.toggle("kb-active", i === kbIdx);
	});
	if (kbIdx >= 0 && items[kbIdx]) {
		items[kbIdx].scrollIntoView({ block: "nearest" });
	}
}

export function renderProjectSelect(): void {
	const wrapper = S.$("projectSelectWrapper");
	const allProjects = projectStore.projects.value;
	const filterId = projectStore.projectFilterId.value;
	if (allProjects.length === 0) {
		if (wrapper) wrapper.classList.add("hidden");
		if (filterId) {
			projectStore.setFilterId("");
			S.setProjectFilterId("");
		}
		label.textContent = t("common:sessions.allSessions");
		return;
	}
	if (wrapper) wrapper.classList.remove("hidden");

	const p = projectStore.getById(filterId);
	label.textContent = p ? (p as ProjectInfo & { label?: string }).label || p.id : t("common:sessions.allSessions");
}

btn.addEventListener("click", () => {
	if (dropdown.classList.contains("hidden")) {
		openDropdown();
	} else {
		closeDropdown();
	}
});

if (searchInput) {
	searchInput.addEventListener("input", () => {
		kbIdx = -1;
		renderList(searchInput.value.trim());
	});

	searchInput.addEventListener("keydown", (e: KeyboardEvent) => {
		const items = list.querySelectorAll<HTMLElement>(".model-dropdown-item");
		if (e.key === "ArrowDown") {
			e.preventDefault();
			kbIdx = Math.min(kbIdx + 1, items.length - 1);
			updateKbActive();
		} else if (e.key === "ArrowUp") {
			e.preventDefault();
			kbIdx = Math.max(kbIdx - 1, 0);
			updateKbActive();
		} else if (e.key === "Enter") {
			e.preventDefault();
			if (kbIdx >= 0 && items[kbIdx]) {
				(items[kbIdx] as HTMLElement).click();
			} else if (items.length === 1) {
				(items[0] as HTMLElement).click();
			}
		} else if (e.key === "Escape") {
			closeDropdown();
			btn.focus();
		}
	});
}

document.addEventListener("click", (e: MouseEvent) => {
	if (combo && !combo.contains(e.target as Node)) {
		closeDropdown();
	}
});

window.addEventListener("moltis:locale-changed", () => {
	renderProjectSelect();
	if (!dropdown.classList.contains("hidden")) {
		const query = searchInput ? searchInput.value.trim() : "";
		renderList(query);
	}
});
