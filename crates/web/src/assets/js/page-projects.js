import { html, Component, render } from "https://esm.sh/htm/preact/standalone.module.js";
import { S } from "./state.js";
import { sendRpc } from "./helpers.js";
import { routes } from "./router.js";

var completions = signal([]);
var editingProject = signal(null);
var detecting = signal(false);
var clearing = signal(false);
var _projectsContainer = null;

export function initProjects(container) {
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	_projectsContainer = container;
	editingProject.value = null;
	completions.value = [];
	detecting.value = false;
	clearing.value = false;
	render(html`<${ProjectsPage} />`, container);
}

export function teardownProjects() {
	if (_projectsContainer) render(null, _projectsContainer);
	_projectsContainer = null;
}

registerPage(
	routes.projects,
	initProjects,
	teardownProjects,
);

function ProjectsPage() {
	return html`
    <div style="display:flex;flex-direction:column;gap:12px;padding:20px;overflow-y:auto;height:100%;">
      <div style="display:flex;align-items:center;justify-content:space-between;">
        <h2 style="font-size:18px;font-weight:600;margin:0;">Repositories</h2>
        <div style="display:flex;gap:8px;">
          <button
            class="provider-btn-secondary"
            style="font-size:12px;padding:4px 12px;"
            onClick=${() => onDetect()}
            disabled=${detecting.value}
          >
            ${detecting.value ? "Detecting..." : "Auto-detect"}
          </button>
          <button
            class="provider-btn-danger"
            style="font-size:12px;padding:4px 12px;"
            onClick=${() => onClearAll()}
          >
            Clear All
          </button>
        </div>
      </div>

      <p style="font-size:13px;color:var(--text-secondary);margin:0;">
        Bind repositories to projects. Projects automatically set the working directory,
        load project-specific context (CLAUDE.md, .cursorrules), and can create
        auto-managed git worktrees for feature branches.
      </p>

      <div style="display:flex;gap:8px;">
        <input
          type="text"
          placeholder="Directory path..."
          style="flex:1;padding:6px 10px;border:1px solid var(--border);border-radius:6px;background:var(--input-bg);color:var(--text);font-size:13px;"
          value=${S.projectInput.value}
          onInput=${(e) => (S.projectInput.value = e.target.value)}
          onKeyDown=${(e) => { if (e.key === "Enter") onAdd(); }}
        />
        <button class="provider-btn" style="font-size:12px;padding:6px 16px;" onClick=${() => onAdd()}>
          Add
        </button>
      </div>

      ${completions.value.length === 0
        ? html`<p style="font-size:13px;color:var(--text-secondary);margin-top:8px;">
            No projects configured. Add a directory above or use auto-detect.
          </p>`
        : completions.value.map((project) => html`
          <${ProjectCard} key=${project.id} project=${project} />
        `)
      }

      <${ConfirmDialog} />
    </div>
  `;
}