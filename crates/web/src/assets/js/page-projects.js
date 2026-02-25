// ── Projects page (Preact + HTM + Signals) ──────────────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { sendRpc } from "./helpers.js";
import { fetchProjects } from "./projects.js";
import { registerPage } from "./router.js";
import { routes } from "./routes.js";
import * as S from "./state.js";
import { projects as projectsSig } from "./stores/project-store.js";
import { ConfirmDialog, requestConfirm } from "./ui.js";

var completions = signal([]);
var editingProject = signal(null);
var detecting = signal(false);
var clearing = signal(false);

function PathInput(props) {
	var inputRef = useRef(null);
	var timerRef = useRef(null);

	function onInput() {
		clearTimeout(timerRef.current);
		timerRef.current = setTimeout(() => {
			var val = inputRef.current?.value || "";
			if (val.length < 2) {
				completions.value = [];
				return;
			}
			sendRpc("projects.complete_path", { partial: val }).then((res) => {
				if (!res?.ok) {
					completions.value = [];
					return;
				}
				completions.value = res.payload || [];
			});
		}, 200);
	}

	function selectPath(p) {
		if (inputRef.current) {
			inputRef.current.value = `${p}/`;
			inputRef.current.focus();
		}
		completions.value = [];
		onInput();
	}

	return html`<div class="project-dir-group">
    <div class="text-xs text-[var(--muted)] mb-1">Directory</div>
    <div class="flex gap-2 items-center">
    <input ref=${inputRef} type="text" class="provider-key-input flex-1"
      placeholder="/path/to/project" style="font-family:var(--font-mono);"
      onInput=${onInput} />
    <button class="provider-btn"
      onClick=${() => {
				var dir = inputRef.current?.value.trim();
				if (!dir) return;
				props.onAdd(dir).then(() => {
					if (inputRef.current) inputRef.current.value = "";
				});
			}}>Add</button>
    </div>
    ${
			completions.value.length > 0 &&
			html`
      <div class="project-completion" style="display:block;">
        ${completions.value.map(
					(p) => html`
          <div key=${p} class="project-completion-item" onClick=${() => selectPath(p)}>${p}</div>
        `,
				)}
      </div>
    `
		}
  </div>`;
}

var cachedImages = signal([]);

function fetchCachedImages() {
	fetch("/api/images/cached")
		.then((r) => (r.ok ? r.json() : { images: [] }))
		.then((data) => {
			cachedImages.value = data.images || [];
		})
		.catch(() => {
			cachedImages.value = [];
		});
}

function ProjectEditForm(props) {
	var p = props.project;
	var labelRef = useRef(null);
	var dirRef = useRef(null);
	var promptRef = useRef(null);
	var setupRef = useRef(null);
	var teardownRef = useRef(null);
	var prefixRef = useRef(null);
	var wtRef = useRef(null);
	var imageRef = useRef(null);

	useEffect(() => {
		fetchCachedImages();
	}, []);

	function onSave() {
		var updated = JSON.parse(JSON.stringify(p));
		updated.label = labelRef.current?.value.trim() || p.label;
		updated.directory = dirRef.current?.value.trim() || p.directory;
		updated.system_prompt = promptRef.current?.value.trim() || null;
		updated.setup_command = setupRef.current?.value.trim() || null;
		updated.teardown_command = teardownRef.current?.value.trim() || null;
		updated.branch_prefix = prefixRef.current?.value.trim() || null;
		updated.auto_worktree = wtRef.current?.checked;
		updated.sandbox_image = imageRef.current?.value.trim() || null;
		updated.updated_at = Date.now();
		sendRpc("projects.upsert", updated).then(() => {
			editingProject.value = null;
			fetchProjects();
		});
	}

	function field(label, ref, value, placeholder, mono) {
		return html`<div class="project-edit-group">
      <div class="text-xs text-[var(--muted)] project-edit-label">${label}</div>
      <input ref=${ref} type="text" class="provider-key-input"
        value=${value || ""} placeholder=${placeholder || ""}
        style=${mono ? "font-family:var(--font-mono);width:100%;" : "width:100%;"} />
    </div>`;
	}

	return html`<div class="project-edit-form">
    ${field("Label", labelRef, p.label, "Project name")}
    ${field("Directory", dirRef, p.directory, "/path/to/project", true)}
    <div class="project-edit-group">
      <div class="text-xs text-[var(--muted)] project-edit-label">System prompt (optional)</div>
      <textarea ref=${promptRef} class="provider-key-input"
        placeholder="Extra instructions for the LLM when working on this project..."
        style="width:100%;min-height:60px;resize-y;font-size:.8rem;">${p.system_prompt || ""}</textarea>
    </div>
    ${field("Setup command", setupRef, p.setup_command, "e.g. pnpm install", true)}
    ${field("Teardown command", teardownRef, p.teardown_command, "e.g. docker compose down", true)}
    ${field("Branch prefix", prefixRef, p.branch_prefix, "default: moltis", true)}
    <div class="project-edit-group">
      <div class="text-xs text-[var(--muted)] project-edit-label">Sandbox image</div>
      <input ref=${imageRef} type="text" class="provider-key-input" list="project-image-list"
        value=${p.sandbox_image || ""} placeholder="Default (ubuntu:25.10)"
        style="width:100%;font-family:var(--font-mono);font-size:.8rem;" />
      <datalist id="project-image-list">
        ${cachedImages.value.map((img) => html`<option key=${img.tag} value=${img.tag} />`)}
      </datalist>
    </div>
    <div style="margin-bottom:10px;display:flex;align-items:center;gap:8px;">
      <input ref=${wtRef} type="checkbox" checked=${p.auto_worktree} />
      <span class="text-xs text-[var(--text)]">Auto-create git worktree per session</span>
    </div>
    <div style="display:flex;gap:8px;">
      <button class="provider-btn" onClick=${onSave}>Save</button>
      <button class="provider-btn provider-btn-secondary" onClick=${() => {
				editingProject.value = null;
			}}>Cancel</button>
    </div>
  </div>`;
}

function ProjectCard(props) {
	var p = props.project;

	function onDelete() {
		sendRpc("projects.delete", { id: p.id }).then(() => fetchProjects());
	}

	return html`<div class="provider-item" style="margin-bottom:6px;">
    <div style="flex:1;min-width:0;">
      <div class="flex items-center gap-2">
        <div class="provider-item-name">${p.label || p.id}</div>
        ${p.detected && html`<span class="provider-item-badge api-key">auto</span>`}
        ${p.auto_worktree && html`<span class="provider-item-badge oauth">worktree</span>`}
        ${p.setup_command && html`<span class="provider-item-badge api-key">setup</span>`}
        ${p.teardown_command && html`<span class="provider-item-badge api-key">teardown</span>`}
        ${p.branch_prefix && html`<span class="provider-item-badge oauth">${p.branch_prefix}/*</span>`}
        ${p.sandbox_image && html`<span class="provider-item-badge api-key" title=${p.sandbox_image}>image</span>`}
      </div>
      <div style="font-size:.72rem;color:var(--muted);font-family:var(--font-mono);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;margin-top:2px;">
        ${p.directory}
      </div>
      ${
				p.system_prompt &&
				html`<div style="font-size:.7rem;color:var(--muted);margin-top:2px;font-style:italic;">
        System prompt: ${p.system_prompt.substring(0, 80)}${p.system_prompt.length > 80 ? "..." : ""}
      </div>`
			}
    </div>
    <div style="display:flex;gap:4px;flex-shrink:0;">
      <button class="session-action-btn" title="Edit project" onClick=${() => {
				editingProject.value = p.id;
			}}>edit</button>
      <button class="session-action-btn session-delete" title="Remove project" onClick=${onDelete}>x</button>
    </div>
  </div>`;
}

function ProjectsPage() {
	useEffect(() => {
		sendRpc("projects.list", {}).then((res) => {
			if (res?.ok) S.setProjects(res.payload || []);
		});
	}, []);

	function onAdd(dir) {
		return sendRpc("projects.detect", { directories: [dir] }).then((res) => {
			if (res?.ok) {
				var detected = res.payload || [];
				if (detected.length === 0) {
					var slug = dir.split("/").filter(Boolean).pop() || "project";
					var now = Date.now();
					return sendRpc("projects.upsert", {
						id: slug.toLowerCase().replace(/[^a-z0-9-]/g, "-"),
						label: slug,
						directory: dir,
						auto_worktree: false,
						detected: false,
						created_at: now,
						updated_at: now,
					}).then(() => fetchProjects());
				}
				fetchProjects();
			}
		});
	}

	function onDetect() {
		detecting.value = true;
		sendRpc("projects.detect", { directories: [] }).then(() => {
			detecting.value = false;
			fetchProjects();
		});
	}

	function onClearAll() {
		if (clearing.value) return;
		requestConfirm(
			"Clear all repositories from Moltis? This only removes them from the list and does not delete files on disk.",
			{
				confirmLabel: "Clear all",
				danger: true,
			},
		).then((yes) => {
			if (!yes) return;
			var ids = projectsSig.value.map((p) => p.id);
			if (ids.length === 0) return;
			clearing.value = true;
			var chain = Promise.resolve();
			for (const id of ids) {
				chain = chain.then(() => sendRpc("projects.delete", { id: id }));
			}
			chain
				.then(() => fetchProjects())
				.finally(() => {
					clearing.value = false;
				});
		});
	}

	var list = projectsSig.value;
	var clearDisabled = clearing.value || detecting.value || list.length === 0;

	return html`
    <div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
      <div class="flex items-center gap-3">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">Repositories</h2>
        <button class="provider-btn provider-btn-secondary"
          onClick=${onDetect} disabled=${detecting.value}
          title="Scan common locations for git repositories and add them as projects">
          ${detecting.value ? "Detecting\u2026" : "Auto-detect"}
        </button>
        <button
          class="provider-btn provider-btn-danger"
          onClick=${onClearAll}
          disabled=${clearDisabled}
          title="Remove all repository entries from Moltis without deleting files on disk"
        >
          ${clearing.value ? "Clearing\u2026" : "Clear All"}
        </button>
      </div>
      <p class="text-xs text-[var(--muted)] max-w-form">
        Clear All only removes repository entries from Moltis, it does not delete anything from disk.
      </p>
      <p class="text-sm text-[var(--muted)]" style="max-width:600px;margin:0;">
        Projects bind sessions to a codebase directory. When a session is linked to a project, context files (CLAUDE.md, AGENTS.md) are loaded automatically and a custom system prompt can be injected. Enable auto-worktree to give each session its own git branch for isolated work.
      </p>
      <p class="text-sm text-[var(--muted)]" style="max-width:600px;margin:0;">
        <strong class="text-[var(--text)]">Auto-detect</strong> scans common directories under your home folder (<code class="font-mono text-xs">~/Projects</code>, <code class="font-mono text-xs">~/Developer</code>, <code class="font-mono text-xs">~/src</code>, <code class="font-mono text-xs">~/code</code>, <code class="font-mono text-xs">~/repos</code>, <code class="font-mono text-xs">~/workspace</code>, <code class="font-mono text-xs">~/dev</code>, <code class="font-mono text-xs">~/git</code>) and Superset worktrees (<code class="font-mono text-xs">~/.superset/worktrees</code>) for git repositories and adds them as projects.
      </p>
      <div class="project-form-row">
        <${PathInput} onAdd=${onAdd} />
      </div>
      <div style="max-width:600px;margin-top:8px;">
        ${
					list.length === 0 &&
					html`
          <div class="text-sm text-[var(--muted)]" style="padding:12px 0;">
            No projects configured. Add a directory above or use auto-detect.
          </div>
        `
				}
        ${list.map((p) =>
					editingProject.value === p.id
						? html`<${ProjectEditForm} key=${p.id} project=${p} />`
						: html`<${ProjectCard} key=${p.id} project=${p} />`,
				)}
      </div>
      <${ConfirmDialog} />
    </div>
  `;
}

registerPage(
	routes.projects,
	function initProjects(container) {
		container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
		editingProject.value = null;
		completions.value = [];
		detecting.value = false;
		render(html`<${ProjectsPage} />`, container);
	},
	function teardownProjects() {
		var container = S.$("pageContent");
		if (container) render(null, container);
	},
);
