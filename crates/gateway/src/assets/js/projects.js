// ── Projects (sidebar filter + modal) ────────────────────────

import { sendRpc } from "./helpers.js";
import { ensureProjectModal } from "./modals.js";
import { updateNavCount } from "./nav-counts.js";
import { renderSessionProjectSelect } from "./project-combo.js";
import * as S from "./state.js";

var projectSelect = S.$("projectSelect");
var manageProjectsBtn = S.$("manageProjectsBtn");

var _projectEls = null;

function projectEls() {
	if (!_projectEls) {
		ensureProjectModal();
		_projectEls = {
			modal: S.$("projectModal"),
			body: S.$("projectModalBody"),
			close: S.$("projectModalClose"),
		};
		_projectEls.close.addEventListener("click", () => {
			_projectEls.modal.classList.add("hidden");
		});
		_projectEls.modal.addEventListener("click", (e) => {
			if (e.target === _projectEls.modal) _projectEls.modal.classList.add("hidden");
		});
	}
	return _projectEls;
}

export function fetchProjects() {
	sendRpc("projects.list", {}).then((res) => {
		if (!res?.ok) return;
		var list = res.payload || [];
		S.setProjects(list);
		renderProjectSelect();
		renderSessionProjectSelect();
		updateNavCount("projects", list.length);
	});
}

export function renderProjectSelect() {
	var wrapper = S.$("projectSelectWrapper");
	if (S.projects.length === 0) {
		if (wrapper) wrapper.classList.add("hidden");
		// Reset filter when no projects exist
		if (S.projectFilterId) {
			S.setProjectFilterId("");
			localStorage.removeItem("moltis-project-filter");
		}
		return;
	}
	if (wrapper) wrapper.classList.remove("hidden");

	while (projectSelect.firstChild) projectSelect.removeChild(projectSelect.firstChild);
	var defaultOpt = document.createElement("option");
	defaultOpt.value = "";
	defaultOpt.textContent = "All sessions";
	projectSelect.appendChild(defaultOpt);

	S.projects.forEach((p) => {
		var opt = document.createElement("option");
		opt.value = p.id;
		opt.textContent = p.label || p.id;
		projectSelect.appendChild(opt);
	});
	projectSelect.value = S.projectFilterId || "";
}

projectSelect.addEventListener("change", () => {
	S.setProjectFilterId(projectSelect.value);
	localStorage.setItem("moltis-project-filter", S.projectFilterId);
	// renderSessionList is called from sessions.js — import would be circular,
	// so we dispatch a custom event instead.
	document.dispatchEvent(new CustomEvent("moltis:render-session-list"));
});

// ── Project modal ──────────────────────────────────────────
manageProjectsBtn.addEventListener("click", () => {
	renderProjectModal();
	projectEls().modal.classList.remove("hidden");
});

function renderProjectModal() {
	var projectModalBody = projectEls().body;
	while (projectModalBody.firstChild) projectModalBody.removeChild(projectModalBody.firstChild);

	var detectBtn = document.createElement("button");
	detectBtn.className = "provider-btn provider-btn-secondary";
	detectBtn.textContent = "Auto-detect projects";
	detectBtn.style.marginBottom = "8px";
	detectBtn.addEventListener("click", () => {
		detectBtn.disabled = true;
		detectBtn.textContent = "Detecting...";
		sendRpc("projects.detect", { directories: [] }).then((res) => {
			detectBtn.disabled = false;
			detectBtn.textContent = "Auto-detect projects";
			if (res?.ok) {
				fetchProjects();
				renderProjectModal();
			}
		});
	});
	projectModalBody.appendChild(detectBtn);

	var addForm = document.createElement("div");
	addForm.className = "provider-key-form";
	addForm.style.marginBottom = "12px";

	var dirLabel = document.createElement("div");
	dirLabel.className = "text-xs text-[var(--muted)]";
	dirLabel.textContent = "Add project by directory path:";
	addForm.appendChild(dirLabel);

	var dirWrap = document.createElement("div");
	dirWrap.style.position = "relative";

	var dirInput = document.createElement("input");
	dirInput.type = "text";
	dirInput.className = "provider-key-input";
	dirInput.placeholder = "/path/to/project";
	dirInput.style.fontFamily = "var(--font-mono)";
	dirWrap.appendChild(dirInput);

	var completionList = document.createElement("div");
	completionList.style.cssText =
		"position:absolute;left:0;right:0;top:100%;background:var(--surface);border:1px solid var(--border);border-radius:4px;max-height:150px;overflow-y:auto;z-index:20;display:none;";
	dirWrap.appendChild(completionList);
	addForm.appendChild(dirWrap);

	var addBtnRow = document.createElement("div");
	addBtnRow.style.display = "flex";
	addBtnRow.style.gap = "8px";

	var addBtn = document.createElement("button");
	addBtn.className = "provider-btn";
	addBtn.textContent = "Add project";
	addBtn.addEventListener("click", () => {
		var dir = dirInput.value.trim();
		if (!dir) return;
		addBtn.disabled = true;
		sendRpc("projects.detect", { directories: [dir] }).then((res) => {
			addBtn.disabled = false;
			if (res?.ok) {
				var detected = res.payload || [];
				if (detected.length === 0) {
					var slug = dir.split("/").filter(Boolean).pop() || "project";
					var now = Date.now();
					sendRpc("projects.upsert", {
						id: slug.toLowerCase().replace(/[^a-z0-9-]/g, "-"),
						label: slug,
						directory: dir,
						auto_worktree: false,
						detected: false,
						created_at: now,
						updated_at: now,
					}).then(() => {
						fetchProjects();
						renderProjectModal();
					});
				} else {
					fetchProjects();
					renderProjectModal();
				}
			}
		});
	});
	addBtnRow.appendChild(addBtn);
	addForm.appendChild(addBtnRow);
	projectModalBody.appendChild(addForm);

	var completeTimer = null;
	dirInput.addEventListener("input", () => {
		clearTimeout(completeTimer);
		completeTimer = setTimeout(() => {
			var val = dirInput.value;
			if (val.length < 2) {
				completionList.style.display = "none";
				return;
			}
			sendRpc("projects.complete_path", { partial: val }).then((res) => {
				if (!res?.ok) {
					completionList.style.display = "none";
					return;
				}
				var paths = res.payload || [];
				while (completionList.firstChild) completionList.removeChild(completionList.firstChild);
				if (paths.length === 0) {
					completionList.style.display = "none";
					return;
				}
				paths.forEach((p) => {
					var item = document.createElement("div");
					item.textContent = p;
					item.style.cssText =
						"padding:6px 10px;cursor:pointer;font-size:.78rem;font-family:var(--font-mono);color:var(--text);transition:background .1s;";
					item.addEventListener("mouseenter", () => {
						item.style.background = "var(--bg-hover)";
					});
					item.addEventListener("mouseleave", () => {
						item.style.background = "";
					});
					item.addEventListener("click", () => {
						dirInput.value = `${p}/`;
						completionList.style.display = "none";
						dirInput.focus();
						dirInput.dispatchEvent(new Event("input"));
					});
					completionList.appendChild(item);
				});
				completionList.style.display = "block";
			});
		}, 200);
	});

	var sep = document.createElement("div");
	sep.style.cssText = "border-top:1px solid var(--border);margin:4px 0 8px;";
	projectModalBody.appendChild(sep);

	if (S.projects.length === 0) {
		var empty = document.createElement("div");
		empty.className = "text-xs text-[var(--muted)]";
		empty.textContent = "No projects configured yet.";
		projectModalBody.appendChild(empty);
	} else {
		S.projects.forEach((p) => {
			var row = document.createElement("div");
			row.className = "provider-item";

			var info = document.createElement("div");
			info.style.flex = "1";
			info.style.minWidth = "0";

			var name = document.createElement("div");
			name.className = "provider-item-name";
			name.textContent = p.label || p.id;
			info.appendChild(name);

			var dir = document.createElement("div");
			dir.style.cssText =
				"font-size:.7rem;color:var(--muted);font-family:var(--font-mono);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;";
			dir.textContent = p.directory;
			info.appendChild(dir);

			row.appendChild(info);

			var actions = document.createElement("div");
			actions.style.cssText = "display:flex;gap:4px;flex-shrink:0;";

			if (p.detected) {
				var badge = document.createElement("span");
				badge.className = "provider-item-badge api-key";
				badge.textContent = "auto";
				actions.appendChild(badge);
			}

			var delBtn = document.createElement("button");
			delBtn.className = "session-action-btn session-delete";
			delBtn.textContent = "x";
			delBtn.title = "Remove project";
			delBtn.addEventListener("click", (e) => {
				e.stopPropagation();
				sendRpc("projects.delete", { id: p.id }).then(() => {
					fetchProjects();
					renderProjectModal();
				});
			});
			actions.appendChild(delBtn);

			row.appendChild(actions);

			row.addEventListener("click", () => {
				S.setActiveProjectId(p.id);
				localStorage.setItem("moltis-project", S.activeProjectId);
				renderProjectSelect();
				projectEls().modal.classList.add("hidden");
			});

			projectModalBody.appendChild(row);
		});
	}
}
