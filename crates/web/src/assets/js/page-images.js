// ── Images page (Preact + HTM + Signals) ──────────────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { localizedApiErrorMessage } from "./helpers.js";
import { updateNavCount } from "./nav-counts.js";
import { sandboxInfo } from "./signals.js";

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
var containers = signal([]);
var loadingContainers = signal(false);
var diskUsage = signal(null);
var cleaningAll = signal(false);
var restarting = signal(false);
var containerError = signal("");
var sharedHomeEnabled = signal(false);
var sharedHomeMode = signal("off");
var sharedHomePath = signal("");
var sharedHomeConfiguredPath = signal("");
var sharedHomeLoading = signal(false);
var sharedHomeSaving = signal(false);
var sharedHomeMsg = signal("");
var sharedHomeErr = signal("");
var SANDBOX_DISABLED_HINT =
	"Sandboxes are disabled on cloud deploys without a container runtime. Install on a VM with Docker or Apple Container to enable this feature.";

function sandboxRuntimeAvailable() {
	return (sandboxInfo.value?.backend || "none") !== "none";
}

async function responseErrorMessage(response, fallback) {
	try {
		var payload = await response.json();
		return localizedApiErrorMessage(payload, fallback);
	} catch {
		try {
			var text = await response.text();
			return text || fallback;
		} catch {
			return fallback;
		}
	}
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
				buildStatus.value = `Error: ${localizedApiErrorMessage(data, "Failed to build image.")}`;
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

function fetchContainers() {
	loadingContainers.value = true;
	fetch("/api/sandbox/containers")
		.then((r) => (r.ok ? r.json() : { containers: [] }))
		.then((data) => {
			containers.value = data.containers || [];
			containerError.value = "";
		})
		.catch(() => {
			containers.value = [];
		})
		.finally(() => {
			loadingContainers.value = false;
		});
}

function stopContainer(name) {
	fetch(`/api/sandbox/containers/${encodeURIComponent(name)}/stop`, { method: "POST" })
		.then((r) => {
			if (r.ok) fetchContainers();
		})
		.catch(() => {
			/* ignore */
		});
}

function removeContainer(name) {
	fetch(`/api/sandbox/containers/${encodeURIComponent(name)}`, { method: "DELETE" })
		.then(async (r) => {
			if (!r.ok) {
				var msg = await responseErrorMessage(r, r.statusText);
				containerError.value = `Failed to delete ${name}: ${msg}`;
				return;
			}
			fetchContainers();
		})
		.catch((e) => {
			containerError.value = `Failed to delete ${name}: ${e.message}`;
		});
}

function fetchDiskUsage() {
	fetch("/api/sandbox/disk-usage")
		.then((r) => (r.ok ? r.json() : null))
		.then((data) => {
			diskUsage.value = data?.usage || null;
		})
		.catch(() => {
			diskUsage.value = null;
		});
}

function cleanAllContainers() {
	cleaningAll.value = true;
	fetch("/api/sandbox/containers/clean", { method: "POST" })
		.then(async (r) => {
			if (!r.ok) {
				var msg = await responseErrorMessage(r, r.statusText);
				containerError.value = `Failed to clean containers: ${msg}`;
				return;
			}
			fetchContainers();
			fetchDiskUsage();
		})
		.catch((e) => {
			containerError.value = `Failed to clean containers: ${e.message}`;
		})
		.finally(() => {
			cleaningAll.value = false;
		});
}

function restartDaemon() {
	restarting.value = true;
	fetch("/api/sandbox/daemon/restart", { method: "POST" })
		.then(async (r) => {
			if (!r.ok) {
				var msg = await responseErrorMessage(r, r.statusText);
				containerError.value = `Failed to restart daemon: ${msg}`;
				return;
			}
			fetchContainers();
			fetchDiskUsage();
		})
		.catch((e) => {
			containerError.value = `Failed to restart daemon: ${e.message}`;
		})
		.finally(() => {
			restarting.value = false;
		});
}

function applySharedHomeConfig(config) {
	var payload = config || {};
	sharedHomeEnabled.value = payload.enabled === true;
	sharedHomeMode.value = payload.mode || "off";
	sharedHomePath.value = payload.path || "";
	sharedHomeConfiguredPath.value = payload.configured_path || "";
}

function fetchSharedHomeConfig() {
	sharedHomeLoading.value = true;
	sharedHomeErr.value = "";
	sharedHomeMsg.value = "";
	fetch("/api/sandbox/shared-home")
		.then(async (r) => {
			if (!r.ok) {
				throw new Error(await responseErrorMessage(r, "Failed to load shared folder settings."));
			}
			return r.json();
		})
		.then((data) => {
			applySharedHomeConfig(data);
		})
		.catch((e) => {
			sharedHomeErr.value = e.message;
		})
		.finally(() => {
			sharedHomeLoading.value = false;
		});
}

function saveSharedHomeConfig() {
	sharedHomeSaving.value = true;
	sharedHomeErr.value = "";
	sharedHomeMsg.value = "";
	fetch("/api/sandbox/shared-home", {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({
			enabled: sharedHomeEnabled.value,
			path: sharedHomePath.value || "",
		}),
	})
		.then(async (r) => {
			if (!r.ok) {
				throw new Error(await responseErrorMessage(r, "Failed to save shared folder settings."));
			}
			return r.json();
		})
		.then((data) => {
			applySharedHomeConfig(data?.config || {});
			sharedHomeMsg.value = "Saved. Restart Moltis to apply shared folder changes.";
			if (sandboxInfo.value) {
				sandboxInfo.value = {
					...sandboxInfo.value,
					shared_home_enabled: sharedHomeEnabled.value,
					shared_home_dir: sharedHomePath.value,
				};
			}
		})
		.catch((e) => {
			sharedHomeErr.value = e.message;
		})
		.finally(() => {
			sharedHomeSaving.value = false;
		});
}

function formatBytes(bytes) {
	if (bytes == null) return "—";
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

var STATE_LABELS = {
	running: { label: "running", color: "var(--accent)" },
	stopped: { label: "stopped", color: "var(--muted)" },
	exited: { label: "exited", color: "var(--muted)" },
	unknown: { label: "unknown", color: "var(--muted)" },
};

var BACKEND_ICONS = {
	"apple-container": "\u{1F34E}",
	docker: "\u{1F433}",
};

/** Truncate long hash suffixes: "repo:abcdef...uvwxyz" */
function truncateHash(str) {
	var idx = str.lastIndexOf(":");
	if (idx !== -1) {
		var suffix = str.slice(idx + 1);
		if (suffix.length > 12) {
			return `${str.slice(0, idx + 1) + suffix.slice(0, 6)}\u2026${suffix.slice(-6)}`;
		}
	}
	if (str.length > 24 && str.indexOf(":") === -1) {
		return `${str.slice(0, 6)}\u2026${str.slice(-6)}`;
	}
	return str;
}

function ContainerRow(props) {
	var c = props.container;
	var sandboxAvailable = props.sandboxAvailable;
	var stateInfo = STATE_LABELS[c.state] || STATE_LABELS.unknown;
	var backendIcon = BACKEND_ICONS[c.backend] || "";
	var isRunning = c.state === "running";
	var resources = [];
	if (c.cpus) resources.push(`${c.cpus} CPU`);
	if (c.memory_mb) resources.push(`${c.memory_mb} MB`);

	return html`<div class="provider-item flex-col gap-1 mb-1" style="align-items:stretch;">
    <div class="flex items-center justify-between gap-2 w-full min-w-0">
      <span class="font-mono text-xs truncate flex-1 text-[var(--text-strong)]" title="${c.name}">${truncateHash(c.name)}</span>
      <span class="inline-flex items-center gap-1 text-xs px-1.5 py-0.5 rounded-full shrink-0"
        style="background:color-mix(in srgb, ${stateInfo.color} 15%, transparent);color:${stateInfo.color};">
        <span class="inline-block w-1.5 h-1.5 rounded-full" style="background:${stateInfo.color};"></span>
        ${stateInfo.label}
      </span>
    </div>
    <div class="flex items-center justify-between gap-2 w-full">
      <div class="flex items-center gap-2 text-xs text-[var(--muted)]">
        <span title="${c.backend}">${backendIcon}</span>
        <span class="font-mono" title="${c.image}">${truncateHash(c.image)}</span>
        ${resources.length > 0 && html`<span>${resources.join(" \u00b7 ")}</span>`}
      </div>
      <div class="flex items-center gap-1">
        ${
					isRunning &&
					html`<button class="text-xs px-2 py-0.5 rounded border border-[var(--border)] bg-transparent text-[var(--muted)] hover:text-[var(--text)] hover:border-[var(--border-strong)] transition-colors cursor-pointer"
          onClick=${() => stopContainer(c.name)}
          disabled=${!sandboxAvailable}
          title=${sandboxAvailable ? "Stop container" : SANDBOX_DISABLED_HINT}>Stop</button>`
				}
        <button class="text-xs text-white border border-[var(--error)] px-2 py-0.5 rounded bg-[var(--error)] hover:opacity-80 transition-colors cursor-pointer"
          onClick=${() => removeContainer(c.name)}
          disabled=${!sandboxAvailable}
          title=${sandboxAvailable ? "Delete container" : SANDBOX_DISABLED_HINT}>Delete</button>
      </div>
    </div>
  </div>`;
}

function DiskUsageBar() {
	var du = diskUsage.value;
	if (!du) return null;

	return html`<div class="text-xs text-[var(--muted)] flex flex-wrap gap-x-4 gap-y-1 mt-1 mb-2">
    <span>Containers: ${du.containers_total} total, ${du.containers_active} active \u00b7 ${formatBytes(du.containers_size_bytes)} (${formatBytes(du.containers_reclaimable_bytes)} reclaimable)</span>
    <span>Images: ${du.images_total} total, ${du.images_active} active \u00b7 ${formatBytes(du.images_size_bytes)}</span>
  </div>`;
}

function RunningContainersSection() {
	var sandboxAvailable = sandboxRuntimeAvailable();
	var list = containers.value;

	return html`<div class="max-w-form">
    <div class="flex items-center gap-3 mb-2">
      <h3 class="text-sm font-medium text-[var(--text-strong)]">Running Containers${list.length > 0 ? ` (${list.length})` : ""}</h3>
      <button class="text-xs text-[var(--muted)] border border-[var(--border)] px-2 py-0.5 rounded-md hover:text-[var(--text)] hover:border-[var(--border-strong)] transition-colors cursor-pointer bg-transparent"
        onClick=${restartDaemon}
        disabled=${restarting.value || !sandboxAvailable}
        title=${sandboxAvailable ? "Restart container daemon" : SANDBOX_DISABLED_HINT}>
        ${restarting.value ? "Restarting\u2026" : "Restart"}
      </button>
      <button class="text-xs text-[var(--muted)] border border-[var(--border)] px-2 py-0.5 rounded-md hover:text-[var(--text)] hover:border-[var(--border-strong)] transition-colors cursor-pointer bg-transparent"
        onClick=${() => {
					fetchContainers();
					fetchDiskUsage();
				}}
        disabled=${loadingContainers.value || !sandboxAvailable}
        title=${sandboxAvailable ? "Refresh" : SANDBOX_DISABLED_HINT}>
        ${loadingContainers.value ? "Loading\u2026" : "Refresh"}
      </button>
      ${
				list.length > 0 &&
				html`
        <button class="text-xs text-white border border-[var(--error)] px-2 py-0.5 rounded-md bg-[var(--error)] hover:opacity-80 transition-colors cursor-pointer"
          onClick=${cleanAllContainers}
          disabled=${cleaningAll.value || !sandboxAvailable}
          title=${sandboxAvailable ? "Stop and remove all containers" : SANDBOX_DISABLED_HINT}>
          ${cleaningAll.value ? "Cleaning\u2026" : "Clean All"}
        </button>
      `
			}
    </div>
    <${DiskUsageBar} />
    ${containerError.value && html`<div class="alert-error-text mb-2">${containerError.value}</div>`}
    ${loadingContainers.value && list.length === 0 && html`<div class="text-xs text-[var(--muted)]">Loading\u2026</div>`}
    ${!loadingContainers.value && list.length === 0 && html`<div class="text-xs text-[var(--muted)]" style="padding:4px 0;">No containers found.</div>`}
    ${list.map((c) => html`<${ContainerRow} key=${c.name} container=${c} sandboxAvailable=${sandboxAvailable} />`)}
  </div>`;
}

var BACKEND_LABELS = {
	"apple-container": "Apple Container (VM-isolated)",
	docker: "Docker",
	cgroup: "cgroup (systemd-run)",
	"restricted-host": "Restricted Host (env + rlimits)",
	wasm: "Wasmtime (WASM-isolated)",
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

	if (backend === "restricted-host") {
		return {
			level: "info",
			text: "Using restricted host execution (env clearing, rlimits). For stronger isolation, install Docker or Apple Container.",
		};
	}

	if (backend === "wasm") {
		return {
			level: "info",
			text: "Using WASM sandbox with filesystem isolation. For container-level isolation, install Docker or Apple Container.",
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
		info.backend === "none"
			? "var(--error)"
			: info.backend === "apple-container"
				? "var(--accent)"
				: info.backend === "wasm"
					? "var(--success)"
					: info.backend === "restricted-host"
						? "var(--warning, var(--muted))"
						: "var(--muted)";

	return html`<div class="max-w-form">
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
          ${rec.level === "warn" ? "Warning: " : "Tip: "}
        </span>${rec.text}
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

function SharedHomeSection() {
	var modeLabel = sharedHomeMode.value === "shared" ? "enabled" : `disabled (${sharedHomeMode.value})`;

	return html`<div class="max-w-form" style="border-top:1px solid var(--border);padding-top:16px;">
    <h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">Shared home folder</h3>
    <p class="text-xs text-[var(--muted)] leading-relaxed" style="margin:0 0 10px;">
      Controls where <code>/home/sandbox</code> is persisted when shared home mode is enabled.
    </p>
    <div class="text-xs text-[var(--muted)]" style="margin-bottom:10px;">
      Status: <span style="color:${sharedHomeMode.value === "shared" ? "var(--accent)" : "var(--muted)"}">${modeLabel}</span>
    </div>
    ${
			sharedHomeLoading.value
				? html`<div class="text-xs text-[var(--muted)]">Loading…</div>`
				: html`<div style="display:flex;flex-direction:column;gap:8px;">
          <label for="sandboxSharedHomeEnabled" class="text-xs text-[var(--text)]" style="display:flex;align-items:center;gap:8px;">
            <input
              id="sandboxSharedHomeEnabled"
              type="checkbox"
              checked=${sharedHomeEnabled.value}
              onInput=${(e) => {
								sharedHomeEnabled.value = e.target.checked;
							}}
            />
            <span>Enable shared home folder</span>
          </label>
          <label for="sandboxSharedHomePath" class="text-xs text-[var(--muted)]">Shared folder location</label>
          <input
            id="sandboxSharedHomePath"
            type="text"
            class="provider-key-input"
            placeholder="data_dir()/sandbox/home/shared"
            value=${sharedHomePath.value}
            onInput=${(e) => {
							sharedHomePath.value = e.target.value;
						}}
            style="font-family:var(--font-mono);font-size:.75rem;"
          />
          ${
						sharedHomeConfiguredPath.value
							? html`<div class="text-xs text-[var(--muted)]">Configured path: <code>${sharedHomeConfiguredPath.value}</code></div>`
							: html`<div class="text-xs text-[var(--muted)]">Configured path: <em>default</em></div>`
					}
          <div style="display:flex;gap:8px;align-items:center;">
            <button class="provider-btn" onClick=${saveSharedHomeConfig} disabled=${sharedHomeSaving.value}>
              ${sharedHomeSaving.value ? "Saving…" : "Save"}
            </button>
            ${
							sharedHomeErr.value
								? html`<span class="text-xs" style="color:var(--error);">${sharedHomeErr.value}</span>`
								: sharedHomeMsg.value
									? html`<span class="text-xs" style="color:var(--accent);">${sharedHomeMsg.value}</span>`
									: null
						}
          </div>
        </div>`
		}
  </div>`;
}

function ImageRow(props) {
	var img = props.image;
	var sandboxAvailable = props.sandboxAvailable;
	var kindLabel = img.kind === "sandbox" ? "sandbox" : "tool";
	var kindColor = img.kind === "sandbox" ? "var(--accent)" : "var(--muted)";
	return html`<div class="provider-item" style="margin-bottom:4px;">
    <div style="flex:1;min-width:0;">
      <div class="flex items-center gap-2">
        <span class="provider-item-name" style="font-family:var(--font-mono);font-size:.8rem;" title="${img.tag}">${truncateHash(img.tag)}</span>
        <span class="text-[0.65rem] px-1.5 py-0.5 rounded-full" style="background:color-mix(in srgb, ${kindColor} 15%, transparent);color:${kindColor};">${kindLabel}</span>
      </div>
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
		fetchContainers();
		fetchDiskUsage();
		fetchSharedHomeConfig();
	}, []);

	return html`
    <div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
      ${
				!sandboxRuntimeAvailable() &&
				html`<div class="alert-warning-text max-w-form"><span class="alert-label-warn">Warning: </span>${SANDBOX_DISABLED_HINT}</div>`
			}
      <div class="flex items-center gap-3">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">Sandboxes</h2>
        <button class="text-xs text-[var(--muted)] border border-[var(--border)] px-2.5 py-1 rounded-md hover:text-[var(--text)] hover:border-[var(--border-strong)] transition-colors cursor-pointer bg-transparent"
			  onClick=${pruneAll} disabled=${pruning.value || !sandboxRuntimeAvailable()}
			  title=${sandboxRuntimeAvailable() ? "Prune all" : SANDBOX_DISABLED_HINT}>
          ${pruning.value ? "Pruning\u2026" : "Prune all"}
        </button>
      </div>
      <p class="text-sm text-[var(--muted)] leading-relaxed max-w-form" style="margin:0;">
        Container images cached by moltis for sandbox execution. You can delete individual images or prune all. Build custom images from a base with apt packages.
        ${sandboxInfo.value?.backend === "apple-container" && html`<br /><br />Apple Container provides VM-isolated execution but does not support building images. Docker (or OrbStack) is required alongside Apple Container to build and cache custom images. Sandboxed commands run via Apple Container; image builds use Docker.`}
      </p>

      <${SandboxBanner} />

      <${RunningContainersSection} />

      <${DefaultImageSelector} />

      <${SharedHomeSection} />

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
          <span class="alert-label-warn">Warning: </span>${buildWarning.value}
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

var _imagesContainer = null;

export function initImages(container) {
	_imagesContainer = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	images.value = [];
	containers.value = [];
	diskUsage.value = null;
	defaultImage.value = sandboxInfo.value?.default_image || "";
	buildStatus.value = "";
	buildWarning.value = "";
	containerError.value = "";
	sharedHomeEnabled.value = false;
	sharedHomeMode.value = "off";
	sharedHomePath.value = "";
	sharedHomeConfiguredPath.value = "";
	sharedHomeLoading.value = false;
	sharedHomeSaving.value = false;
	sharedHomeMsg.value = "";
	sharedHomeErr.value = "";
	render(html`<${ImagesPage} />`, container);
}

export function teardownImages() {
	if (_imagesContainer) render(null, _imagesContainer);
	_imagesContainer = null;
}
