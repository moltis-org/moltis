// ── Projects (sidebar filter) ────────────────────────────────

import { sendRpc } from "./helpers.js";
import { updateNavCount } from "./nav-counts.js";
import { renderSessionProjectSelect } from "./project-combo.js";
import * as S from "./state.js";

var combo = S.$("projectFilterCombo");
var btn = S.$("projectFilterBtn");
var label = S.$("projectFilterLabel");
var dropdown = S.$("projectFilterDropdown");
var list = S.$("projectFilterList");

export function fetchProjects() {
	sendRpc("projects.list", {}).then((res) => {
		if (!res?.ok) return;
		var projects = res.payload || [];
		S.setProjects(projects);
		renderProjectSelect();
		renderSessionProjectSelect();
		updateNavCount("projects", projects.length);
	});
}

function selectFilter(id) {
	S.setProjectFilterId(id);
	localStorage.setItem("moltis-project-filter", id);
	var p = S.projects.find((x) => x.id === id);
	label.textContent = p ? p.label || p.id : "All sessions";
	closeDropdown();
	document.dispatchEvent(new CustomEvent("moltis:render-session-list"));
}

function closeDropdown() {
	dropdown.classList.add("hidden");
}

function openDropdown() {
	dropdown.classList.remove("hidden");
	renderList();
}

function renderList() {
	list.textContent = "";

	// "All sessions" option
	var allEl = document.createElement("div");
	allEl.className = "model-dropdown-item";
	if (!S.projectFilterId) allEl.classList.add("selected");
	var allLabel = document.createElement("span");
	allLabel.className = "model-item-label";
	allLabel.textContent = "All sessions";
	allEl.appendChild(allLabel);
	allEl.addEventListener("click", () => selectFilter(""));
	list.appendChild(allEl);

	S.projects.forEach((p) => {
		var el = document.createElement("div");
		el.className = "model-dropdown-item";
		if (p.id === S.projectFilterId) el.classList.add("selected");
		var itemLabel = document.createElement("span");
		itemLabel.className = "model-item-label";
		itemLabel.textContent = p.label || p.id;
		el.appendChild(itemLabel);
		el.addEventListener("click", () => selectFilter(p.id));
		list.appendChild(el);
	});
}

export function renderProjectSelect() {
	var wrapper = S.$("projectSelectWrapper");
	if (S.projects.length === 0) {
		if (wrapper) wrapper.classList.add("hidden");
		if (S.projectFilterId) {
			S.setProjectFilterId("");
			localStorage.removeItem("moltis-project-filter");
		}
		label.textContent = "All sessions";
		return;
	}
	if (wrapper) wrapper.classList.remove("hidden");

	// Restore label from saved filter
	var p = S.projects.find((x) => x.id === S.projectFilterId);
	label.textContent = p ? p.label || p.id : "All sessions";
}

btn.addEventListener("click", () => {
	if (dropdown.classList.contains("hidden")) {
		openDropdown();
	} else {
		closeDropdown();
	}
});

document.addEventListener("click", (e) => {
	if (combo && !combo.contains(e.target)) {
		closeDropdown();
	}
});
