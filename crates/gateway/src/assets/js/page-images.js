// ── Images page (Preact + HTM + Signals) ──────────────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { updateNavCount } from "./nav-counts.js";
import { registerPage } from "./router.js";
import { sandboxInfo } from "./signals.js";
import * as S from "./state.js";

var defaultImage = signal("");
var savingDefault = signal(false);
var images = signal([]);
var loading = signal(false);
var buildName = signal("");
var buildBase = signal("ubuntu:25.10");
var buildPackages = signal("");
var building = signal(false);
var buildStatus = signal("");
var buildWarning = signal("");
var pruning = signal(false);
var SANDBOX_DISABLED_HINT =
	"Sandboxes are disabled on cloud deploys without a container runtime. Install on a VM with Docker or Apple Container to enable this feature.";

function sandboxRuntimeAvailable() {
	return (sandboxInfo.value?.backend || "none") !== "none";
}

function fetchImages() {
	loading.value = true;
	fetch("/api/images/cached")
		.then((r) => (r.ok ? r.json() : { images: [] }))
		.then((data) => {
			images.value = data.images || [];
			updateNavCount("images", images.value.length);
		})
		.catch(() => {
			images.value = [];
		})
		.finally(() => {
			loading.value = false;
		});
}

function deleteImage(tag) {
	var encoded = encodeURIComponent(tag);
	fetch(`/api/images/cached/${encoded}`, { method: "DELETE" })
		.then((r) => {
			if (r.ok) fetchImages();
		})
		.catch(() => {
			/* ignore */
		});
}

function pruneAll() {
	pruning.value = true;
	fetch("/api/images/cached", { method: "DELETE" })
		.then((r) => {
			if (r.ok) fetchImages();
		})
		.catch(() => {
			/* ignore */
		})
		.finally(() => {
			pruning.value = false;
		});
}

function doBuild(name, base, pkgs) {
	buildStatus.value = "Building image\u2026";
	fetch("/api/images/build", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ name, base, packages: pkgs }),
	})
		.then((r) => r.json())
		.then((data) => {
			if (data.error) {
				buildStatus.value = `Error: ${data.error}`;
			} else {
				buildStatus.value = `Built: ${data.tag}`;
				buildName.value = "";
				buildPackages.value = "";
				fetchImages();
			}
		})
		.catch((e) => {
			buildStatus.value = `Error: ${e.message}`;
		})
		.finally(() => {
			building.value = false;
		});
}

function buildImage() {
	var name = buildName.value.trim();
	if (!name) return;
	var base = buildBase.value.trim() || "ubuntu:25.10";
	var pkgs = buildPackages.value
		.trim()
		.split(/[\s,]+/)
		.filter(Boolean);
	if (pkgs.length === 0) {
		buildStatus.value = "Please specify at least one package.";
		return;
	}
	building.value = true;
	buildWarning.value = "";
	buildStatus.value = "Checking packages in base image\u2026";

	fetch("/api/images/check-packages", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ base, packages: pkgs }),
	})
		.then((r) => (r.ok ? r.json() : null))
		.then((data) => {
			var found = data?.found || {};
			var present = pkgs.filter((p) => found[p]);
			var missing = pkgs.filter((p) => !found[p]);

			if (present.length > 0 && missing.length === 0) {
				// All packages already in base image
				building.value = false;
				buildWarning.value = `All requested packages are already present in ${base}: ${present.join(", ")}. No image build needed.`;
				buildStatus.value = "";
				return;
			}

			if (present.length > 0) {
				buildWarning.value = `Already in ${base}: ${present.join(", ")}. Only installing: ${missing.join(", ")}.`;
			}

			doBuild(name, base, missing.length > 0 ? missing : pkgs);
		})
		.catch(() => {
			// Check failed (e.g. image not pulled yet), proceed with full build
			doBuild(name, base, pkgs);
		});
}

var BACKEND_LABELS = {
	"apple-container": "Apple Container (VM-isolated)",
	docker: "Docker",
	cgroup: "cgroup (systemd-run)",
	none: "None (host execution)",
};

function backendRecommendation(info) {
	if (!info) return null;
	var os = info.os;
	var backend = info.backend;

	if (backend === "none") {
		if (os === "macos") {
			return {
				level: "warn",
				text: "No container runtime detected. Install Apple Container (macOS 26+) for VM-isolated sandboxing, or install Docker as an alternative.",
				link: "https://developer.apple.com/documentation/virtualization",
			};
		}
		if (os === "linux") {
			return {
				level: "warn",
				text: "No container runtime detected. Install Docker for sandboxed execution, or ensure systemd is available for cgroup isolation.",
			};
		}
		return {
			level: "warn",
			text: "No container runtime detected. Install Docker for sandboxed execution.",
		};
	}

	if (os === "macos" && backend === "docker") {
		return {
			level: "info",
			text: "Apple Container provides stronger VM-level isolation on macOS 26+. Install it for automatic use (moltis prefers it over Docker). Run: brew install container",
		};
	}

	if (os === "linux" && backend === "docker") {
		return {
			level: "info",
			text: "Docker is a good choice on Linux. For lighter-weight isolation without Docker overhead, systemd cgroup sandboxing is also supported.",
		};
	}

	return null;
}

function SandboxBanner() {
	var info = sandboxInfo.value;
	if (!info) return null;

	var label = BACKEND_LABELS[info.backend] || info.backend;
	var rec = backendRecommendation(info);

	var badgeColor =
		info.backend === "none" ? "var(--error)" : info.backend === "apple-container" ? "var(--accent)" : "var(--muted)";

	return html`<div>
    <div class="info-bar" style="margin-bottom:8px;">
      <span class="info-field">
        <span class="info-label">Container backend:</span>
        <span class="info-value-strong" style="color:${badgeColor};font-family:var(--font-mono)">${label}</span>
      </span>
    </div>
    ${
			rec &&
			html`
      <div class="${rec.level === "warn" ? "alert-warning-text" : "alert-info-text"}">
        <span class="${rec.level === "warn" ? "alert-label-warn" : "alert-label-info"}">
          ${rec.level === "warn" ? "Warning" : "Tip"}:
        </span>
        ${" "}${rec.text}
      </div>
    `
		}
  </div>`;
}

function DefaultImageSelector() {
	var info = sandboxInfo.value;
	var current = defaultImage.value || info?.default_image || "";
	var sandboxAvailable = sandboxRuntimeAvailable();

	function onSave() {
		var val = defaultImage.value.trim();
		savingDefault.value = true;
		fetch("/api/images/default", {
			method: "PUT",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ image: val || null }),
		})
			.then((r) => (r.ok ? r.json() : null))
			.then((data) => {
				if (data) defaultImage.value = data.image;
			})
			.catch(() => {
				/* ignore */
			})
			.finally(() => {
				savingDefault.value = false;
			});
	}

	return html`<div class="max-w-form">
    <h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">Default image</h3>
    <p class="text-xs text-[var(--muted)]" style="margin:0 0 8px;">
      Base image used for new sessions and projects unless overridden. Leave empty to use the built-in default (ubuntu:25.10).
    </p>
    <div style="display:flex;gap:8px;align-items:center;">
      <input type="text" class="provider-key-input" list="default-image-list"
        placeholder="ubuntu:25.10"
        style="flex:1;font-family:var(--font-mono);font-size:.8rem;"
        value=${current}
        onInput=${(e) => {
					defaultImage.value = e.target.value;
				}} />
      <button class="provider-btn" onClick=${onSave} disabled=${savingDefault.value || !sandboxAvailable}
			title=${sandboxAvailable ? null : SANDBOX_DISABLED_HINT}>
        ${savingDefault.value ? "Saving\u2026" : "Save"}
      </button>
    </div>
    <datalist id="default-image-list">
      ${images.value.map((img) => html`<option key=${img.tag} value=${img.tag} />`)}
    </datalist>
  </div>`;
}

function ImageRow(props) {
	var img = props.image;
	var sandboxAvailable = props.sandboxAvailable;
	return html`<div class="provider-item" style="margin-bottom:4px;">
    <div style="flex:1;min-width:0;">
      <div class="provider-item-name" style="font-family:var(--font-mono);font-size:.8rem;">${img.tag}</div>
      <div style="font-size:.7rem;color:var(--muted);margin-top:2px;display:flex;gap:12px;">
        <span>${img.size}</span>
        <span>${img.created}</span>
      </div>
    </div>
    <button class="session-action-btn session-delete" title=${sandboxAvailable ? "Delete image" : SANDBOX_DISABLED_HINT}
		disabled=${!sandboxAvailable}
      onClick=${() => deleteImage(img.tag)}>x</button>
  </div>`;
}

function ImagesPage() {
	useEffect(() => {
		fetchImages();
	}, []);

	return html`
    <div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
      ${
				!sandboxRuntimeAvailable() &&
				html`<div class="alert-warning-text"><span class="alert-label-warn">Warning:</span> ${" "}${SANDBOX_DISABLED_HINT}</div>`
			}
      <div class="flex items-center gap-3">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">Sandboxes</h2>
        <button class="text-xs text-[var(--muted)] border border-[var(--border)] px-2.5 py-1 rounded-md hover:text-[var(--text)] hover:border-[var(--border-strong)] transition-colors cursor-pointer bg-transparent"
			  onClick=${pruneAll} disabled=${pruning.value || !sandboxRuntimeAvailable()}
			  title=${sandboxRuntimeAvailable() ? "Prune all" : SANDBOX_DISABLED_HINT}>
          ${pruning.value ? "Pruning\u2026" : "Prune all"}
        </button>
      </div>
      <p class="text-sm text-[var(--muted)] leading-relaxed" class="max-w-form" style="margin:0;">
        Container images cached by moltis for sandbox execution. You can delete individual images or prune all. Build custom images from a base with apt packages.
        ${sandboxInfo.value?.backend === "apple-container" && html`<br /><br />Apple Container provides VM-isolated execution but does not support building images. Docker (or OrbStack) is required alongside Apple Container to build and cache custom images. Sandboxed commands run via Apple Container; image builds use Docker.`}
      </p>

      <${SandboxBanner} />

      <${DefaultImageSelector} />

      <!-- Cached images list -->
      <div class="max-w-form">
        ${loading.value && html`<div class="text-xs text-[var(--muted)]">Loading\u2026</div>`}
        ${!loading.value && images.value.length === 0 && html`<div class="text-xs text-[var(--muted)]" style="padding:12px 0;">No cached images.</div>`}
        ${images.value.map((img) => html`<${ImageRow} key=${img.tag} image=${img} sandboxAvailable=${sandboxRuntimeAvailable()} />`)}
      </div>

      <!-- Build custom image -->
      <div class="max-w-form" style="margin-top:8px;border-top:1px solid var(--border);padding-top:16px;">
        <h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:12px;">Build custom image</h3>
        <div class="project-edit-group" style="margin-bottom:8px;">
          <div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Image name</div>
          <input type="text" class="provider-key-input" placeholder="my-tools"
            style="width:100%;" value=${buildName.value}
            onInput=${(e) => {
							buildName.value = e.target.value;
						}} />
        </div>
        <div class="project-edit-group" style="margin-bottom:8px;">
          <div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Base image</div>
          <input type="text" class="provider-key-input" placeholder="ubuntu:25.10"
            style="width:100%;font-family:var(--font-mono);" value=${buildBase.value}
            onInput=${(e) => {
							buildBase.value = e.target.value;
						}} />
        </div>
        <div class="project-edit-group" style="margin-bottom:8px;">
          <div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Packages (space or newline separated)</div>
          <textarea class="provider-key-input"
            placeholder="ffmpeg python3-pip curl"
            style="width:100%;min-height:60px;resize:vertical;font-family:var(--font-mono);font-size:.8rem;"
            value=${buildPackages.value}
            onInput=${(e) => {
							buildPackages.value = e.target.value;
						}}></textarea>
        </div>
        <button class="provider-btn" onClick=${buildImage}
			  disabled=${building.value || !buildName.value.trim() || !buildPackages.value.trim() || !sandboxRuntimeAvailable()}
			  title=${sandboxRuntimeAvailable() ? "Build" : SANDBOX_DISABLED_HINT}>
          ${building.value ? "Building\u2026" : "Build"}
        </button>
        ${
					buildWarning.value &&
					html`<div class="alert-warning-text" style="margin-top:8px;">
          <span class="alert-label-warn">Warning:</span>${" "}${buildWarning.value}
        </div>`
				}
        ${
					buildStatus.value &&
					(buildStatus.value.startsWith("Error")
						? html`<div class="alert-error-text" style="margin-top:8px;"><pre>${buildStatus.value}</pre></div>`
						: html`<div class="text-xs" style="margin-top:8px;color:var(--muted);">${buildStatus.value}</div>`)
				}
      </div>
    </div>
  `;
}

registerPage(
	"/sandboxes",
	function initImages(container) {
		container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
		images.value = [];
		defaultImage.value = sandboxInfo.value?.default_image || "";
		buildStatus.value = "";
		buildWarning.value = "";
		render(html`<${ImagesPage} />`, container);
	},
	function teardownImages() {
		var container = S.$("pageContent");
		if (container) render(null, container);
	},
);
