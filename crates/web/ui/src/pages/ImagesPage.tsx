// ── Images page (Preact + Signals) ──────────────────────────

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { TabBar } from "../components/forms/Tabs";
import { localizedApiErrorMessage } from "../helpers";
import { updateNavCount } from "../nav-counts";
import { sandboxInfo } from "../signals";
import type { SandboxGonInfo } from "../types/gon";

// ── Types ────────────────────────────────────────────────────

interface CachedImage {
	tag: string;
	size: string;
	created: string;
	kind: string;
}

interface ContainerInfo {
	name: string;
	state: string;
	backend: string;
	image: string;
	cpus?: number;
	memory_mb?: number;
}

interface DiskUsageInfo {
	containers_total: number;
	containers_active: number;
	containers_size_bytes: number;
	containers_reclaimable_bytes: number;
	images_total: number;
	images_active: number;
	images_size_bytes: number;
}

interface SandboxInfoValue extends SandboxGonInfo {
	shared_home_enabled?: boolean;
	shared_home_dir?: string;
}

interface SharedHomeConfig {
	enabled?: boolean;
	mode?: string;
	path?: string;
	configured_path?: string;
}

interface RemoteBackendsConfig {
	vercel: {
		configured: boolean;
		from_env?: boolean;
		project_id?: string;
		team_id?: string;
		runtime: string;
		timeout_ms: number;
		vcpus: number;
	};
	daytona: {
		configured: boolean;
		from_env?: boolean;
		api_url: string;
		target?: string;
	};
}

// ── Signals ──────────────────────────────────────────────────

const defaultImage = signal("");
const savingDefault = signal(false);
const images = signal<CachedImage[]>([]);
const loading = signal(false);
const buildName = signal("");
const buildBase = signal("ubuntu:25.10");
const buildPackages = signal("");
const building = signal(false);
const buildStatus = signal("");
const buildWarning = signal("");
const pruning = signal(false);
const containers = signal<ContainerInfo[]>([]);
const loadingContainers = signal(false);
const diskUsage = signal<DiskUsageInfo | null>(null);
const cleaningAll = signal(false);
const restarting = signal(false);
const containerError = signal("");
const sharedHomeEnabled = signal(false);
const sharedHomeMode = signal("off");
const sharedHomePath = signal("");
const sharedHomeConfiguredPath = signal("");
const sharedHomeLoading = signal(false);
const sharedHomeSaving = signal(false);
const sharedHomeMsg = signal("");
const sharedHomeErr = signal("");
const remoteConfig = signal<RemoteBackendsConfig | null>(null);
const remoteLoading = signal(false);
const remoteSaving = signal("");
const remoteMsg = signal("");
const remoteErr = signal("");
const vercelToken = signal("");
const vercelProjectId = signal("");
const vercelTeamId = signal("");
const daytonaApiKey = signal("");
const daytonaApiUrl = signal("");
const activeTab = signal("general");
const SANDBOX_TABS = [
	{ id: "general", label: "General" },
	{ id: "vercel", label: "Vercel" },
	{ id: "daytona", label: "Daytona" },
	{ id: "containers", label: "Containers & Images" },
];
const SANDBOX_DISABLED_HINT =
	"No local container runtime detected. Install Docker, configure a remote backend (Vercel or Daytona), or deploy on a VM with Docker to enable sandboxes.";

function sandboxRuntimeAvailable(): boolean {
	return ((sandboxInfo.value as SandboxInfoValue | null)?.backend || "none") !== "none";
}

async function responseErrorMessage(response: Response, fallback: string): Promise<string> {
	try {
		const payload = await response.json();
		return localizedApiErrorMessage(payload, fallback);
	} catch {
		try {
			const text = await response.text();
			return text || fallback;
		} catch {
			return fallback;
		}
	}
}

function fetchImages(): void {
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

function deleteImage(tag: string): void {
	const encoded = encodeURIComponent(tag);
	fetch(`/api/images/cached/${encoded}`, { method: "DELETE" })
		.then((r) => {
			if (r.ok) fetchImages();
		})
		.catch(() => {
			/* ignore */
		});
}

function pruneAll(): void {
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

function doBuild(name: string, base: string, pkgs: string[]): void {
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
		.catch((e: Error) => {
			buildStatus.value = `Error: ${e.message}`;
		})
		.finally(() => {
			building.value = false;
		});
}

function buildImage(): void {
	const name = buildName.value.trim();
	if (!name) return;
	const base = buildBase.value.trim() || "ubuntu:25.10";
	const pkgs = buildPackages.value
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
			const found = data?.found || {};
			const present = pkgs.filter((p) => found[p]);
			const missing = pkgs.filter((p) => !found[p]);

			if (present.length > 0 && missing.length === 0) {
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
			doBuild(name, base, pkgs);
		});
}

function fetchContainers(): void {
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

function stopContainer(name: string): void {
	fetch(`/api/sandbox/containers/${encodeURIComponent(name)}/stop`, { method: "POST" })
		.then((r) => {
			if (r.ok) fetchContainers();
		})
		.catch(() => {
			/* ignore */
		});
}

function removeContainer(name: string): void {
	fetch(`/api/sandbox/containers/${encodeURIComponent(name)}`, { method: "DELETE" })
		.then(async (r) => {
			if (!r.ok) {
				const msg = await responseErrorMessage(r, r.statusText);
				containerError.value = `Failed to delete ${name}: ${msg}`;
				return;
			}
			fetchContainers();
		})
		.catch((e: Error) => {
			containerError.value = `Failed to delete ${name}: ${e.message}`;
		});
}

function fetchDiskUsage(): void {
	fetch("/api/sandbox/disk-usage")
		.then((r) => (r.ok ? r.json() : null))
		.then((data) => {
			diskUsage.value = data?.usage || null;
		})
		.catch(() => {
			diskUsage.value = null;
		});
}

function cleanAllContainers(): void {
	cleaningAll.value = true;
	fetch("/api/sandbox/containers/clean", { method: "POST" })
		.then(async (r) => {
			if (!r.ok) {
				const msg = await responseErrorMessage(r, r.statusText);
				containerError.value = `Failed to clean containers: ${msg}`;
				return;
			}
			fetchContainers();
			fetchDiskUsage();
		})
		.catch((e: Error) => {
			containerError.value = `Failed to clean containers: ${e.message}`;
		})
		.finally(() => {
			cleaningAll.value = false;
		});
}

function restartDaemon(): void {
	restarting.value = true;
	fetch("/api/sandbox/daemon/restart", { method: "POST" })
		.then(async (r) => {
			if (!r.ok) {
				const msg = await responseErrorMessage(r, r.statusText);
				containerError.value = `Failed to restart daemon: ${msg}`;
				return;
			}
			fetchContainers();
			fetchDiskUsage();
		})
		.catch((e: Error) => {
			containerError.value = `Failed to restart daemon: ${e.message}`;
		})
		.finally(() => {
			restarting.value = false;
		});
}

function applySharedHomeConfig(config: SharedHomeConfig | null): void {
	const payload = config || {};
	sharedHomeEnabled.value = payload.enabled === true;
	sharedHomeMode.value = payload.mode || "off";
	sharedHomePath.value = payload.path || "";
	sharedHomeConfiguredPath.value = payload.configured_path || "";
}

function fetchSharedHomeConfig(): void {
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
		.catch((e: Error) => {
			sharedHomeErr.value = e.message;
		})
		.finally(() => {
			sharedHomeLoading.value = false;
		});
}

function saveSharedHomeConfig(): void {
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
					...(sandboxInfo.value as SandboxInfoValue),
					shared_home_enabled: sharedHomeEnabled.value,
					shared_home_dir: sharedHomePath.value,
				};
			}
		})
		.catch((e: Error) => {
			sharedHomeErr.value = e.message;
		})
		.finally(() => {
			sharedHomeSaving.value = false;
		});
}

function fetchRemoteBackends(): void {
	remoteLoading.value = true;
	remoteErr.value = "";
	fetch("/api/sandbox/remote-backends")
		.then(async (r) => {
			if (!r.ok) throw new Error(await responseErrorMessage(r, "Failed to load remote backend config."));
			return r.json() as Promise<RemoteBackendsConfig>;
		})
		.then((data) => {
			remoteConfig.value = data;
			vercelProjectId.value = data.vercel?.project_id || "";
			vercelTeamId.value = data.vercel?.team_id || "";
			daytonaApiUrl.value = data.daytona?.api_url || "https://app.daytona.io/api";
		})
		.catch((e: Error) => {
			remoteErr.value = e.message;
		})
		.finally(() => {
			remoteLoading.value = false;
		});
}

function saveRemoteBackend(backend: string, config: Record<string, unknown>): void {
	remoteSaving.value = backend;
	remoteErr.value = "";
	remoteMsg.value = "";
	fetch("/api/sandbox/remote-backends", {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ backend, config }),
	})
		.then(async (r) => {
			if (!r.ok) throw new Error(await responseErrorMessage(r, "Failed to save remote backend config."));
			return r.json();
		})
		.then((data) => {
			if (data?.config) remoteConfig.value = data.config;
			if (backend !== "_global") {
				remoteMsg.value = `${backend} configuration saved. Restart Moltis to apply.`;
				vercelToken.value = "";
				daytonaApiKey.value = "";
			}
		})
		.catch((e: Error) => {
			remoteErr.value = e.message;
		})
		.finally(() => {
			remoteSaving.value = "";
		});
}

function formatBytes(bytes: number | null | undefined): string {
	if (bytes == null) return "\u2014";
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

const STATE_LABELS: Record<string, { label: string; color: string }> = {
	running: { label: "running", color: "var(--accent)" },
	stopped: { label: "stopped", color: "var(--muted)" },
	exited: { label: "exited", color: "var(--muted)" },
	unknown: { label: "unknown", color: "var(--muted)" },
};

const BACKEND_ICONS: Record<string, string> = {
	"apple-container": "\u{1F34E}",
	docker: "\u{1F433}",
};

/** Truncate long hash suffixes: "repo:abcdef...uvwxyz" */
function truncateHash(str: string): string {
	const idx = str.lastIndexOf(":");
	if (idx !== -1) {
		const suffix = str.slice(idx + 1);
		if (suffix.length > 12) {
			return `${str.slice(0, idx + 1) + suffix.slice(0, 6)}\u2026${suffix.slice(-6)}`;
		}
	}
	if (str.length > 24 && str.indexOf(":") === -1) {
		return `${str.slice(0, 6)}\u2026${str.slice(-6)}`;
	}
	return str;
}

function ContainerRow({
	container: c,
	sandboxAvailable,
}: {
	container: ContainerInfo;
	sandboxAvailable: boolean;
}): VNode {
	const stateInfo = STATE_LABELS[c.state] || STATE_LABELS.unknown;
	const backendIcon = BACKEND_ICONS[c.backend] || "";
	const isRunning = c.state === "running";
	const resources: string[] = [];
	if (c.cpus) resources.push(`${c.cpus} CPU`);
	if (c.memory_mb) resources.push(`${c.memory_mb} MB`);

	return (
		<div className="provider-item flex-col gap-1 mb-1" style={{ alignItems: "stretch" }}>
			<div className="flex items-center justify-between gap-2 w-full min-w-0">
				<span className="font-mono text-xs truncate flex-1 text-[var(--text-strong)]" title={c.name}>
					{truncateHash(c.name)}
				</span>
				<span
					className="inline-flex items-center gap-1 text-xs px-1.5 py-0.5 rounded-full shrink-0"
					style={{ background: `color-mix(in srgb, ${stateInfo.color} 15%, transparent)`, color: stateInfo.color }}
				>
					<span className="inline-block w-1.5 h-1.5 rounded-full" style={{ background: stateInfo.color }} />
					{stateInfo.label}
				</span>
			</div>
			<div className="flex items-center justify-between gap-2 w-full">
				<div className="flex items-center gap-2 text-xs text-[var(--muted)]">
					<span title={c.backend}>{backendIcon}</span>
					<span className="font-mono" title={c.image}>
						{truncateHash(c.image)}
					</span>
					{resources.length > 0 && <span>{resources.join(" \u00b7 ")}</span>}
				</div>
				<div className="flex items-center gap-1">
					{isRunning && (
						<button
							className="text-xs px-2 py-0.5 rounded border border-[var(--border)] bg-transparent text-[var(--muted)] hover:text-[var(--text)] hover:border-[var(--border-strong)] transition-colors cursor-pointer"
							onClick={() => stopContainer(c.name)}
							disabled={!sandboxAvailable}
							title={sandboxAvailable ? "Stop container" : SANDBOX_DISABLED_HINT}
						>
							Stop
						</button>
					)}
					<button
						className="text-xs text-white border border-[var(--error)] px-2 py-0.5 rounded bg-[var(--error)] hover:opacity-80 transition-colors cursor-pointer"
						onClick={() => removeContainer(c.name)}
						disabled={!sandboxAvailable}
						title={sandboxAvailable ? "Delete container" : SANDBOX_DISABLED_HINT}
					>
						Delete
					</button>
				</div>
			</div>
		</div>
	);
}

function DiskUsageBar(): VNode | null {
	const du = diskUsage.value;
	if (!du) return null;

	return (
		<div className="text-xs text-[var(--muted)] flex flex-wrap gap-x-4 gap-y-1 mt-1 mb-2">
			<span>
				Containers: {du.containers_total} total, {du.containers_active} active &middot;{" "}
				{formatBytes(du.containers_size_bytes)} ({formatBytes(du.containers_reclaimable_bytes)} reclaimable)
			</span>
			<span>
				Images: {du.images_total} total, {du.images_active} active &middot; {formatBytes(du.images_size_bytes)}
			</span>
		</div>
	);
}

function RunningContainersSection(): VNode {
	const sandboxAvailable = sandboxRuntimeAvailable();
	const list = containers.value;

	return (
		<div className="max-w-form">
			<div className="flex items-center gap-3 mb-2">
				<h3 className="text-sm font-medium text-[var(--text-strong)]">
					Running Containers{list.length > 0 ? ` (${list.length})` : ""}
				</h3>
				<button
					className="text-xs text-[var(--muted)] border border-[var(--border)] px-2 py-0.5 rounded-md hover:text-[var(--text)] hover:border-[var(--border-strong)] transition-colors cursor-pointer bg-transparent"
					onClick={restartDaemon}
					disabled={restarting.value || !sandboxAvailable}
					title={sandboxAvailable ? "Restart container daemon" : SANDBOX_DISABLED_HINT}
				>
					{restarting.value ? "Restarting\u2026" : "Restart"}
				</button>
				<button
					className="text-xs text-[var(--muted)] border border-[var(--border)] px-2 py-0.5 rounded-md hover:text-[var(--text)] hover:border-[var(--border-strong)] transition-colors cursor-pointer bg-transparent"
					onClick={() => {
						fetchContainers();
						fetchDiskUsage();
					}}
					disabled={loadingContainers.value || !sandboxAvailable}
					title={sandboxAvailable ? "Refresh" : SANDBOX_DISABLED_HINT}
				>
					{loadingContainers.value ? "Loading\u2026" : "Refresh"}
				</button>
				{list.length > 0 && (
					<button
						className="text-xs text-white border border-[var(--error)] px-2 py-0.5 rounded-md bg-[var(--error)] hover:opacity-80 transition-colors cursor-pointer"
						onClick={cleanAllContainers}
						disabled={cleaningAll.value || !sandboxAvailable}
						title={sandboxAvailable ? "Stop and remove all containers" : SANDBOX_DISABLED_HINT}
					>
						{cleaningAll.value ? "Cleaning\u2026" : "Clean All"}
					</button>
				)}
			</div>
			<DiskUsageBar />
			{containerError.value && <div className="alert-error-text mb-2">{containerError.value}</div>}
			{loadingContainers.value && list.length === 0 && (
				<div className="text-xs text-[var(--muted)]">Loading&hellip;</div>
			)}
			{!loadingContainers.value && list.length === 0 && (
				<div className="text-xs text-[var(--muted)]" style={{ padding: "4px 0" }}>
					No containers found.
				</div>
			)}
			{list.map((c) => (
				<ContainerRow key={c.name} container={c} sandboxAvailable={sandboxAvailable} />
			))}
		</div>
	);
}

function backendRecommendation(info: SandboxInfoValue | null): { level: string; text: string; link?: string } | null {
	if (!info) return null;
	const os = info.os;
	const backend = info.backend;

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
		return { level: "warn", text: "No container runtime detected. Install Docker for sandboxed execution." };
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

interface AvailableBackendInfo {
	id: string;
	label: string;
	kind: string;
	available: boolean;
}
const availableBackendsList = signal<AvailableBackendInfo[]>([]);
const defaultBackendId = signal("auto");
const backendSaving = signal(false);

function fetchAvailableBackends(): void {
	fetch("/api/sandbox/available-backends")
		.then((r) => r.json())
		.then((data) => {
			availableBackendsList.value = data.backends || [];
			defaultBackendId.value = data.default || "auto";
		})
		.catch(() => {});
}

function SandboxBanner(): VNode | null {
	const info = sandboxInfo.value as SandboxInfoValue | null;
	if (!info) return null;

	const backends = availableBackendsList.value;
	const rec = backendRecommendation(info);

	function changeDefault(backendId: string): void {
		backendSaving.value = true;
		saveRemoteBackend("_global", { backend: backendId });
		defaultBackendId.value = backendId;
		setTimeout(() => {
			backendSaving.value = false;
		}, 1500);
	}

	return (
		<div className="max-w-form">
			<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
				Available backends
			</h3>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: "0 0 10px" }}>
				Backends available for sandbox execution. Select one per session in the chat panel, or set a default below.
			</p>

			{backends.length > 0 ? (
				<div className="flex flex-wrap gap-2" style={{ marginBottom: "12px" }}>
					{backends.map((b) => {
						const isDefault =
							b.id === defaultBackendId.value || (defaultBackendId.value === "auto" && b.id === info.backend);
						return (
							<button
								type="button"
								key={b.id}
								className="rounded-md border px-3 py-1.5 text-xs cursor-pointer bg-transparent transition-colors"
								style={{
									borderColor: isDefault ? "var(--accent)" : "var(--border)",
									color: isDefault ? "var(--accent)" : "var(--text)",
									fontFamily: "var(--font-mono)",
									fontWeight: isDefault ? "600" : "400",
								}}
								onClick={() => changeDefault(b.id)}
								disabled={backendSaving.value}
								title={isDefault ? "Active default backend" : `Set ${b.label} as default`}
							>
								{b.label}
								{b.kind === "remote" && <span style={{ marginLeft: "4px", opacity: 0.6 }}>{"\u2601"}</span>}
								{isDefault && <span style={{ marginLeft: "6px", fontSize: "0.6rem", opacity: 0.7 }}>(default)</span>}
							</button>
						);
					})}
				</div>
			) : (
				<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "12px" }}>
					Loading backends...
				</div>
			)}

			{rec && (
				<div className={rec.level === "warn" ? "alert-warning-text" : "alert-info-text"}>
					<span className={rec.level === "warn" ? "alert-label-warn" : "alert-label-info"}>
						{rec.level === "warn" ? "Warning: " : "Tip: "}
					</span>
					{rec.text}
				</div>
			)}
		</div>
	);
}

function DefaultImageSelector(): VNode {
	const info = sandboxInfo.value as SandboxInfoValue | null;
	const current = defaultImage.value || info?.default_image || "";
	const sandboxAvailable = sandboxRuntimeAvailable();

	function onSave(): void {
		const val = defaultImage.value.trim();
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

	return (
		<div className="max-w-form">
			<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
				Default image
			</h3>
			<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
				Base image used for new sessions and projects unless overridden. Leave empty to use the built-in default
				(ubuntu:25.10).
			</p>
			<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
				<input
					type="text"
					className="provider-key-input"
					list="default-image-list"
					placeholder="ubuntu:25.10"
					style={{ flex: 1, fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
					value={current}
					onInput={(e) => {
						defaultImage.value = (e.target as HTMLInputElement).value;
					}}
				/>
				<button
					className="provider-btn"
					onClick={onSave}
					disabled={savingDefault.value || !sandboxAvailable}
					title={sandboxAvailable ? undefined : SANDBOX_DISABLED_HINT}
				>
					{savingDefault.value ? "Saving\u2026" : "Save"}
				</button>
			</div>
			<datalist id="default-image-list">
				{images.value.map((img) => (
					<option key={img.tag} value={img.tag} />
				))}
			</datalist>
		</div>
	);
}

function SharedHomeSection(): VNode {
	const modeLabel = sharedHomeMode.value === "shared" ? "enabled" : `disabled (${sharedHomeMode.value})`;

	return (
		<div className="max-w-form" style={{ borderTop: "1px solid var(--border)", paddingTop: "16px" }}>
			<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
				Shared home folder
			</h3>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: "0 0 10px" }}>
				Controls where <code>/home/sandbox</code> is persisted when shared home mode is enabled.
			</p>
			<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "10px" }}>
				Status:{" "}
				<span style={{ color: sharedHomeMode.value === "shared" ? "var(--accent)" : "var(--muted)" }}>{modeLabel}</span>
			</div>
			{sharedHomeLoading.value ? (
				<div className="text-xs text-[var(--muted)]">Loading...</div>
			) : (
				<div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
					<label
						htmlFor="sandboxSharedHomeEnabled"
						className="text-xs text-[var(--text)]"
						style={{ display: "flex", alignItems: "center", gap: "8px" }}
					>
						<input
							id="sandboxSharedHomeEnabled"
							type="checkbox"
							checked={sharedHomeEnabled.value}
							onInput={(e) => {
								sharedHomeEnabled.value = (e.target as HTMLInputElement).checked;
							}}
						/>
						<span>Enable shared home folder</span>
					</label>
					<label htmlFor="sandboxSharedHomePath" className="text-xs text-[var(--muted)]">
						Shared folder location
					</label>
					<input
						id="sandboxSharedHomePath"
						type="text"
						className="provider-key-input"
						placeholder="data_dir()/sandbox/home/shared"
						value={sharedHomePath.value}
						onInput={(e) => {
							sharedHomePath.value = (e.target as HTMLInputElement).value;
						}}
						style={{ fontFamily: "var(--font-mono)", fontSize: ".75rem" }}
					/>
					{sharedHomeConfiguredPath.value ? (
						<div className="text-xs text-[var(--muted)]">
							Configured path: <code>{sharedHomeConfiguredPath.value}</code>
						</div>
					) : (
						<div className="text-xs text-[var(--muted)]">
							Configured path: <em>default</em>
						</div>
					)}
					<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
						<button className="provider-btn" onClick={saveSharedHomeConfig} disabled={sharedHomeSaving.value}>
							{sharedHomeSaving.value ? "Saving..." : "Save"}
						</button>
						{sharedHomeErr.value ? (
							<span className="text-xs" style={{ color: "var(--error)" }}>
								{sharedHomeErr.value}
							</span>
						) : sharedHomeMsg.value ? (
							<span className="text-xs" style={{ color: "var(--accent)" }}>
								{sharedHomeMsg.value}
							</span>
						) : null}
					</div>
				</div>
			)}
		</div>
	);
}

function ImageRow({ image: img, sandboxAvailable }: { image: CachedImage; sandboxAvailable: boolean }): VNode {
	const kindLabel = img.kind === "sandbox" ? "sandbox" : "tool";
	const kindColor = img.kind === "sandbox" ? "var(--accent)" : "var(--muted)";
	return (
		<div className="provider-item" style={{ marginBottom: "4px" }}>
			<div style={{ flex: 1, minWidth: 0 }}>
				<div className="flex items-center gap-2">
					<span
						className="provider-item-name"
						style={{ fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
						title={img.tag}
					>
						{truncateHash(img.tag)}
					</span>
					<span
						className="text-[0.65rem] px-1.5 py-0.5 rounded-full"
						style={{ background: `color-mix(in srgb, ${kindColor} 15%, transparent)`, color: kindColor }}
					>
						{kindLabel}
					</span>
				</div>
				<div style={{ fontSize: ".7rem", color: "var(--muted)", marginTop: "2px", display: "flex", gap: "12px" }}>
					<span>{img.size}</span>
					<span>{img.created}</span>
				</div>
			</div>
			<button
				className="session-action-btn session-delete"
				title={sandboxAvailable ? "Delete image" : SANDBOX_DISABLED_HINT}
				disabled={!sandboxAvailable}
				onClick={() => deleteImage(img.tag)}
			>
				x
			</button>
		</div>
	);
}

function ImagesPage(): VNode {
	useEffect(() => {
		fetchImages();
		fetchContainers();
		fetchDiskUsage();
		fetchSharedHomeConfig();
		fetchRemoteBackends();
		fetchAvailableBackends();
	}, []);

	return (
		<div className="flex-1 flex flex-col min-w-0 overflow-y-auto">
			<div className="px-4 pt-4 pb-0">
				<h2 className="text-lg font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
					Sandboxes
				</h2>
				{!sandboxRuntimeAvailable() && (
					<div className="alert-warning-text max-w-form" style={{ marginBottom: "8px" }}>
						<span className="alert-label-warn">Warning: </span>
						{SANDBOX_DISABLED_HINT}
					</div>
				)}
			</div>
			<TabBar
				tabs={SANDBOX_TABS}
				active={activeTab.value}
				onChange={(id) => {
					activeTab.value = id;
				}}
				className="flex border-b border-[var(--border)] text-xs px-4"
			/>
			<div className="flex-1 flex flex-col p-4 gap-4 overflow-y-auto">
				{activeTab.value === "general" && <GeneralTabContent />}
				{activeTab.value === "vercel" && <VercelTabContent />}
				{activeTab.value === "daytona" && <DaytonaTabContent />}
				{activeTab.value === "containers" && <ContainersTabContent />}
			</div>
		</div>
	);
}

function GeneralTabContent(): VNode {
	return (
		<>
			<SandboxBanner />
			<DefaultImageSelector />
			<SharedHomeSection />
		</>
	);
}

function VercelTabContent(): VNode {
	const cfg = remoteConfig.value;

	function saveVercel(): void {
		const config: Record<string, unknown> = {};
		if (vercelToken.value.trim()) config.token = vercelToken.value.trim();
		if (vercelProjectId.value.trim()) config.project_id = vercelProjectId.value.trim();
		if (vercelTeamId.value.trim()) config.team_id = vercelTeamId.value.trim();
		saveRemoteBackend("vercel", config);
	}

	return (
		<div className="max-w-form">
			<div className="flex items-center gap-2" style={{ marginBottom: "8px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]">Vercel Sandbox</h3>
				{cfg?.vercel?.configured ? (
					<span
						className="text-[10px] px-1.5 py-0.5 rounded-full border"
						style={{ borderColor: "var(--success)", color: "var(--success)" }}
					>
						configured
					</span>
				) : (
					<span
						className="text-[10px] px-1.5 py-0.5 rounded-full border"
						style={{ borderColor: "var(--muted)", color: "var(--muted)" }}
					>
						not configured
					</span>
				)}
			</div>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: "0 0 12px" }}>
				Firecracker microVMs via the Vercel API. Each session gets an ephemeral isolated VM with millisecond boot times.
			</p>
			<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
				<input
					type="password"
					className="provider-key-input"
					placeholder={
						cfg?.vercel?.from_env
							? "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (set via VERCEL_TOKEN env var)"
							: cfg?.vercel?.configured
								? "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (set in config)"
								: "Vercel token (VERCEL_TOKEN)"
					}
					style={{ fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
					value={vercelToken.value}
					disabled={cfg?.vercel?.from_env}
					onInput={(e) => {
						vercelToken.value = (e.target as HTMLInputElement).value;
					}}
				/>
				{cfg?.vercel?.from_env && (
					<div className="text-[10px] text-[var(--muted)]">
						Token managed by environment variable. Remove VERCEL_TOKEN from env to configure here.
					</div>
				)}
				<div style={{ display: "flex", gap: "6px" }}>
					<input
						type="text"
						className="provider-key-input"
						placeholder="Project ID (required)"
						style={{ flex: 1, fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
						value={vercelProjectId.value}
						onInput={(e) => {
							vercelProjectId.value = (e.target as HTMLInputElement).value;
						}}
					/>
					<input
						type="text"
						className="provider-key-input"
						placeholder="Team ID (optional)"
						style={{ flex: 1, fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
						value={vercelTeamId.value}
						onInput={(e) => {
							vercelTeamId.value = (e.target as HTMLInputElement).value;
						}}
					/>
				</div>
				<button
					className="provider-btn"
					style={{ alignSelf: "flex-start" }}
					onClick={saveVercel}
					disabled={remoteSaving.value === "vercel" || !vercelToken.value.trim() || !vercelProjectId.value.trim()}
				>
					{remoteSaving.value === "vercel" ? "Saving\u2026" : "Save"}
				</button>
			</div>
			{remoteMsg.value && remoteMsg.value.includes("vercel") && (
				<div className="text-xs" style={{ marginTop: "8px", color: "var(--success)" }}>
					{remoteMsg.value}
				</div>
			)}
			{remoteErr.value && (
				<div className="alert-error-text" style={{ marginTop: "8px" }}>
					{remoteErr.value}
				</div>
			)}
		</div>
	);
}

function DaytonaTabContent(): VNode {
	const cfg = remoteConfig.value;

	function saveDaytona(): void {
		const config: Record<string, unknown> = {};
		if (daytonaApiKey.value.trim()) config.api_key = daytonaApiKey.value.trim();
		if (daytonaApiUrl.value.trim()) config.api_url = daytonaApiUrl.value.trim();
		saveRemoteBackend("daytona", config);
	}

	return (
		<div className="max-w-form">
			<div className="flex items-center gap-2" style={{ marginBottom: "8px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]">Daytona</h3>
				{cfg?.daytona?.configured ? (
					<span
						className="text-[10px] px-1.5 py-0.5 rounded-full border"
						style={{ borderColor: "var(--success)", color: "var(--success)" }}
					>
						configured
					</span>
				) : (
					<span
						className="text-[10px] px-1.5 py-0.5 rounded-full border"
						style={{ borderColor: "var(--muted)", color: "var(--muted)" }}
					>
						not configured
					</span>
				)}
			</div>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: "0 0 12px" }}>
				Open-source cloud sandboxes. Self-hostable on your own infrastructure (Proxmox, bare-metal) or use the managed
				Daytona service.
			</p>
			<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
				<input
					type="password"
					className="provider-key-input"
					placeholder={
						cfg?.daytona?.from_env
							? "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (set via DAYTONA_API_KEY env var)"
							: cfg?.daytona?.configured
								? "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022 (set in config)"
								: "Daytona API key (DAYTONA_API_KEY)"
					}
					style={{ fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
					value={daytonaApiKey.value}
					disabled={cfg?.daytona?.from_env}
					onInput={(e) => {
						daytonaApiKey.value = (e.target as HTMLInputElement).value;
					}}
				/>
				{cfg?.daytona?.from_env && (
					<div className="text-[10px] text-[var(--muted)]">
						Token managed by environment variable. Remove DAYTONA_API_KEY from env to configure here.
					</div>
				)}
				<input
					type="text"
					className="provider-key-input"
					placeholder="API URL (default: https://app.daytona.io/api)"
					style={{ fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
					value={daytonaApiUrl.value}
					onInput={(e) => {
						daytonaApiUrl.value = (e.target as HTMLInputElement).value;
					}}
				/>
				<button
					className="provider-btn"
					style={{ alignSelf: "flex-start" }}
					onClick={saveDaytona}
					disabled={remoteSaving.value === "daytona" || !daytonaApiKey.value.trim()}
				>
					{remoteSaving.value === "daytona" ? "Saving\u2026" : "Save"}
				</button>
			</div>
			{remoteMsg.value && remoteMsg.value.includes("daytona") && (
				<div className="text-xs" style={{ marginTop: "8px", color: "var(--success)" }}>
					{remoteMsg.value}
				</div>
			)}
			{remoteErr.value && (
				<div className="alert-error-text" style={{ marginTop: "8px" }}>
					{remoteErr.value}
				</div>
			)}
		</div>
	);
}

function ContainersTabContent(): VNode {
	const sbInfo = sandboxInfo.value as SandboxInfoValue | null;

	return (
		<>
			<div className="flex items-center gap-3">
				<button
					className="provider-btn-secondary provider-btn-sm"
					onClick={pruneAll}
					disabled={pruning.value || !sandboxRuntimeAvailable()}
					title={sandboxRuntimeAvailable() ? "Prune all" : SANDBOX_DISABLED_HINT}
				>
					{pruning.value ? "Pruning\u2026" : "Prune all"}
				</button>
			</div>
			{sbInfo?.backend === "apple-container" && (
				<p className="text-xs text-[var(--muted)] leading-relaxed max-w-form" style={{ margin: 0 }}>
					Apple Container provides VM-isolated execution but does not support building images. Docker (or OrbStack) is
					required alongside Apple Container to build and cache custom images.
				</p>
			)}
			<RunningContainersSection />

			{/* Cached images list */}
			<div className="max-w-form">
				{loading.value && <div className="text-xs text-[var(--muted)]">Loading&hellip;</div>}
				{!loading.value && images.value.length === 0 && (
					<div className="text-xs text-[var(--muted)]" style={{ padding: "12px 0" }}>
						No cached images.
					</div>
				)}
				{images.value.map((img) => (
					<ImageRow key={img.tag} image={img} sandboxAvailable={sandboxRuntimeAvailable()} />
				))}
			</div>

			{/* Build custom image */}
			<div
				className="max-w-form"
				style={{ marginTop: "8px", borderTop: "1px solid var(--border)", paddingTop: "16px" }}
			>
				<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "12px" }}>
					Build custom image
				</h3>
				<div className="project-edit-group" style={{ marginBottom: "8px" }}>
					<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
						Image name
					</div>
					<input
						type="text"
						className="provider-key-input"
						placeholder="my-tools"
						style={{ width: "100%" }}
						value={buildName.value}
						onInput={(e) => {
							buildName.value = (e.target as HTMLInputElement).value;
						}}
					/>
				</div>
				<div className="project-edit-group" style={{ marginBottom: "8px" }}>
					<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
						Base image
					</div>
					<input
						type="text"
						className="provider-key-input"
						placeholder="ubuntu:25.10"
						style={{ width: "100%", fontFamily: "var(--font-mono)" }}
						value={buildBase.value}
						onInput={(e) => {
							buildBase.value = (e.target as HTMLInputElement).value;
						}}
					/>
				</div>
				<div className="project-edit-group" style={{ marginBottom: "8px" }}>
					<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
						Packages (space or newline separated)
					</div>
					<textarea
						className="provider-key-input"
						placeholder="ffmpeg python3-pip curl"
						style={{
							width: "100%",
							minHeight: "60px",
							resize: "vertical",
							fontFamily: "var(--font-mono)",
							fontSize: ".8rem",
						}}
						value={buildPackages.value}
						onInput={(e) => {
							buildPackages.value = (e.target as HTMLTextAreaElement).value;
						}}
					/>
				</div>
				<button
					className="provider-btn"
					onClick={buildImage}
					disabled={
						building.value || !buildName.value.trim() || !buildPackages.value.trim() || !sandboxRuntimeAvailable()
					}
					title={sandboxRuntimeAvailable() ? "Build" : SANDBOX_DISABLED_HINT}
				>
					{building.value ? "Building\u2026" : "Build"}
				</button>
				{buildWarning.value && (
					<div className="alert-warning-text" style={{ marginTop: "8px" }}>
						<span className="alert-label-warn">Warning: </span>
						{buildWarning.value}
					</div>
				)}
				{buildStatus.value &&
					(buildStatus.value.startsWith("Error") ? (
						<div className="alert-error-text" style={{ marginTop: "8px" }}>
							<pre>{buildStatus.value}</pre>
						</div>
					) : (
						<div className="text-xs" style={{ marginTop: "8px", color: "var(--muted)" }}>
							{buildStatus.value}
						</div>
					))}
			</div>
		</>
	);
}

let _imagesContainer: HTMLElement | null = null;

export function initImages(container: HTMLElement): void {
	_imagesContainer = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	images.value = [];
	containers.value = [];
	diskUsage.value = null;
	defaultImage.value = (sandboxInfo.value as SandboxInfoValue | null)?.default_image || "";
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
	activeTab.value = "general";
	availableBackendsList.value = [];
	remoteConfig.value = null;
	remoteLoading.value = false;
	remoteSaving.value = "";
	remoteMsg.value = "";
	remoteErr.value = "";
	vercelToken.value = "";
	vercelProjectId.value = "";
	vercelTeamId.value = "";
	daytonaApiKey.value = "";
	daytonaApiUrl.value = "";
	render(<ImagesPage />, container);
}

export function teardownImages(): void {
	if (_imagesContainer) render(null, _imagesContainer);
	_imagesContainer = null;
}
