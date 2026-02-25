// ── Projects (sidebar filter) ────────────────────────────────

import { updateNavCount } from "./nav-counts.js";
import { renderSessionProjectSelect } from "./project-combo.js";
import * as S from "./state.js";
import { projectStore } from "./stores/project-store.js";

var combo = S.$("projectFilterCombo");
var btn = S.$("projectFilterBtn");
var label = S.$("projectFilterLabel");
var dropdown = S.$("projectFilterDropdown");
var list = S.$("projectFilterList");
var searchInput = S.$("projectFilterSearch");
var kbIdx = -1;

export function fetchProjects() {
	projectStore.fetch().then(() => {
		var projects = projectStore.projects.value;
		// Dual-write to state.js for backward compat
		S.setProjects(projects);
		renderProjectSelect();
		renderSessionProjectSelect();
		updateNavCount("projects", projects.length);
	});
}

function selectFilter(id) {
	projectStore.setFilterId(id);
	// Dual-write to state.js for backward compat
	S.setProjectFilterId(id);
	var p = projectStore.getById(id);
	label.textContent = p ? p.label || p.id : "All sessions";
	closeDropdown();
	document.dispatchEvent(new CustomEvent("moltis:render-session-list"));
}

function closeDropdown() {
	dropdown.classList.add("hidden");
	if (searchInput) searchInput.value = "";
	kbIdx = -1;
}

function openDropdown() {
	dropdown.classList.remove("hidden");
	kbIdx = -1;
	renderList("");
	requestAnimationFrame(() => {
		if (searchInput) searchInput.focus();
	});
}

function renderList(query) {
	list.textContent = "";
	var q = (query || "").toLowerCase();
	var filterId = projectStore.projectFilterId.value;
	var allProjects = projectStore.projects.value;

	// "All sessions" option — always shown unless query excludes it
	if (!q || "all sessions".indexOf(q) !== -1) {
		var allEl = document.createElement("div");
		allEl.className = "model-dropdown-item";
		if (!filterId) allEl.classList.add("selected");
		var allLabel = document.createElement("span");
		allLabel.className = "model-item-label";
		allLabel.textContent = "All sessions";
		allEl.appendChild(allLabel);
		allEl.addEventListener("click", () => selectFilter(""));
		list.appendChild(allEl);
	}

	var filtered = allProjects.filter((p) => {
		if (!q) return true;
		var name = (p.label || p.id).toLowerCase();
		return name.indexOf(q) !== -1 || p.id.toLowerCase().indexOf(q) !== -1;
	});

	filtered.forEach((p) => {
		var el = document.createElement("div");
		el.className = "model-dropdown-item";
		if (p.id === filterId) el.classList.add("selected");
		var itemLabel = document.createElement("span");
		itemLabel.className = "model-item-label";
		itemLabel.textContent = p.label || p.id;
		el.appendChild(itemLabel);
		el.addEventListener("click", () => selectFilter(p.id));
		list.appendChild(el);
	});

	if (list.children.length === 0) {
		var empty = document.createElement("div");
		empty.className = "model-dropdown-empty";
		empty.textContent = "No matching projects";
		list.appendChild(empty);
	}
}

function updateKbActive() {
	var items = list.querySelectorAll(".model-dropdown-item");
	items.forEach((el, i) => {
		el.classList.toggle("kb-active", i === kbIdx);
	});
	if (kbIdx >= 0 && items[kbIdx]) {
		items[kbIdx].scrollIntoView({ block: "nearest" });
	}
}

export function renderProjectSelect() {
	var wrapper = S.$("projectSelectWrapper");
	var allProjects = projectStore.projects.value;
	var filterId = projectStore.projectFilterId.value;
	if (allProjects.length === 0) {
		if (wrapper) wrapper.classList.add("hidden");
		if (filterId) {
			projectStore.setFilterId("");
			S.setProjectFilterId("");
		}
		label.textContent = "All sessions";
		return;
	}
	if (wrapper) wrapper.classList.remove("hidden");

	var p = projectStore.getById(filterId);
	label.textContent = p ? p.label || p.id : "All sessions";
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

	searchInput.addEventListener("keydown", (e) => {
		var items = list.querySelectorAll(".model-dropdown-item");
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
				items[kbIdx].click();
			} else if (items.length === 1) {
				items[0].click();
			}
		} else if (e.key === "Escape") {
			closeDropdown();
			btn.focus();
		}
	});
}

document.addEventListener("click", (e) => {
	if (combo && !combo.contains(e.target)) {
		closeDropdown();
	}
});
