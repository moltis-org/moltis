// ── MCP page ────────────────────────────────────────────────

import { signal, useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { onEvent } from "../events";
import { sendRpc } from "../helpers";
import { updateNavCount } from "../nav-counts";
import { ConfirmDialog, requestConfirm } from "../ui";

// ── Types ────────────────────────────────────────────────────

interface McpServer {
	name: string;
	display_name?: string;
	state?: string;
	enabled?: boolean;
	transport?: string;
	command?: string;
	args?: string[];
	env?: Record<string, string>;
	url?: string;
	headers?: Record<string, string>;
	header_names?: string[];
	headerNames?: string[];
	header_count?: number;
	headerCount?: number;
	tool_count?: number;
	auth_state?: string;
	request_timeout_secs?: number | null;
	configured_request_timeout_secs?: number;
}

interface McpTool {
	name: string;
	description?: string;
}
interface ToastItem {
	id: number;
	message: string;
	type: string;
}
interface FeaturedServer {
	name: string;
	repo?: string;
	desc: string;
	command?: string;
	args?: string[];
	transport?: string;
	url?: string;
	requiresConfig?: boolean;
	envKeys?: string[];
	hint?: string;
}

// ── Signals ─────────────────────────────────────────────────

const servers = signal<McpServer[]>([]);
const loading = signal(false);
const configLoading = signal(false);
const configSaving = signal(false);
const requestTimeoutSecs = signal("30");
const configDirty = signal(false);
const toasts = signal<ToastItem[]>([]);
let toastId = 0;

function showToast(message: string, type: string): void {
	const id = ++toastId;
	toasts.value = [...toasts.value, { id, message, type }];
	setTimeout(() => {
		toasts.value = toasts.value.filter((t) => t.id !== id);
	}, 4000);
}

async function refreshServers(): Promise<void> {
	loading.value = true;
	try {
		const res = await fetch("/api/mcp");
		if (res.ok) servers.value = (await res.json()) || [];
	} catch {
		const rpc = await sendRpc("mcp.list", {});
		if (rpc.ok) servers.value = (rpc.payload as McpServer[]) || [];
	}
	loading.value = false;
	updateNavCount("mcp", servers.value.filter((s) => s.state === "running").length);
}

async function refreshConfig(): Promise<void> {
	configLoading.value = true;
	try {
		const res = await sendRpc("mcp.config.get", {});
		if (res?.ok && res.payload) {
			requestTimeoutSecs.value = String((res.payload as Record<string, number>).request_timeout_secs || 30);
			configDirty.value = false;
		}
	} finally {
		configLoading.value = false;
	}
}

function normalizeOptionalTimeout(rawValue: string): { ok: boolean; value: number | null; message?: string } {
	const trimmed = String(rawValue || "").trim();
	if (!trimmed) return { ok: true, value: null };
	if (!/^\d+$/.test(trimmed)) return { ok: false, value: null, message: "Timeout must be a positive number" };
	const parsed = Number.parseInt(trimmed, 10);
	if (!Number.isFinite(parsed) || parsed <= 0)
		return { ok: false, value: null, message: "Timeout must be a positive number" };
	return { ok: true, value: parsed };
}

function resolveTimeoutOrAbort(
	rawValue: string,
	setBusy: (v: boolean) => void,
): { ok: boolean; value?: number | null } {
	const t = normalizeOptionalTimeout(rawValue);
	if (!t.ok) {
		showToast(t.message || "Invalid timeout", "error");
		setBusy(false);
		return { ok: false };
	}
	return { ok: true, value: t.value };
}

async function saveConfig(): Promise<void> {
	const t = normalizeOptionalTimeout(requestTimeoutSecs.value);
	if (!t.ok || t.value === null) {
		showToast("MCP request timeout must be a positive number", "error");
		return;
	}
	configSaving.value = true;
	try {
		const res = await sendRpc("mcp.config.update", { request_timeout_secs: t.value });
		if (res?.ok) {
			requestTimeoutSecs.value = String((res.payload as Record<string, number>)?.request_timeout_secs || t.value);
			configDirty.value = false;
			showToast("Saved MCP settings. Restart servers to apply.", "success");
		} else showToast(`Failed: ${res?.error?.message || "unknown"}`, "error");
	} finally {
		configSaving.value = false;
	}
}

function oauthCallbackUrl(): string {
	return `${window.location.origin}/auth/callback`;
}

async function addServer(payload: Record<string, unknown>): Promise<void> {
	const req = { ...payload };
	const t = (payload.transport as string) || "stdio";
	if (t === "sse" || t === "streamable-http") (req as Record<string, string>).redirectUri = oauthCallbackUrl();
	const res = await sendRpc("mcp.add", req);
	if (res?.ok) {
		const p = res.payload as Record<string, unknown>;
		showToast(`Added MCP tool "${p?.name || payload.name}"`, "success");
		if (p?.oauthPending && p?.authUrl) window.open(p.authUrl as string, "_blank", "noopener,noreferrer");
	} else showToast(`Failed to add "${payload.name}": ${res?.error?.message || "unknown"}`, "error");
	await refreshServers();
}

function parseEnvLines(text: string): Record<string, string> {
	const env: Record<string, string> = {};
	if (!text) return env;
	for (const line of text.split("\n")) {
		const trimmed = line.trim();
		if (!trimmed || trimmed.startsWith("#")) continue;
		const idx = trimmed.indexOf("=");
		if (idx > 0) env[trimmed.slice(0, idx).trim()] = trimmed.slice(idx + 1).trim();
	}
	return env;
}

function transportLabel(transport: string | undefined): string {
	if (transport === "streamable-http") return "streamable-http remote";
	return transport === "sse" ? "sse remote" : "stdio local";
}
function deriveNameFromCommand(cmdLine: string): string {
	const parts = cmdLine.trim().split(/\s+/).filter(Boolean);
	for (let i = parts.length - 1; i >= 0; i--) {
		const tk = parts[i];
		if (tk.startsWith("-")) continue;
		let base = tk.includes("/") ? tk.split("/").pop()! : tk;
		base = base
			.replace(/^mcp-server-/, "")
			.replace(/^server-/, "")
			.replace(/^mcp-/, "");
		if (base) return base.toLowerCase().replace(/[^a-z0-9-]/g, "-");
	}
	return parts[0] || "";
}
function deriveSseName(url: string): string {
	if (!url) return "";
	try {
		const parts = new URL(url.trim()).hostname.split(".").filter((p) => p !== "mcp" && p !== "www");
		return parts.length > 0 ? parts[0].toLowerCase() : "";
	} catch {
		return "";
	}
}
function remoteHeaderNames(server: McpServer): string[] {
	return (server.header_names || server.headerNames || (server.headers ? Object.keys(server.headers) : []))
		.filter((n) => typeof n === "string" && n.trim())
		.map((n) => n.trim());
}
function remoteHeaderCount(server: McpServer): number {
	const ec = server.header_count ?? server.headerCount;
	if (ec != null && Number.isFinite(ec) && ec >= 0) return ec;
	return remoteHeaderNames(server).length;
}
function remoteHeaderSummary(server: McpServer): string {
	const names = remoteHeaderNames(server);
	const count = remoteHeaderCount(server);
	if (!(count || names.length)) return "none configured";
	if (!names.length) return `${count} configured`;
	return `${names.join(", ")} (${count} total)`;
}

// ── Featured MCP servers ────────────────────────────────────

const featuredServers: FeaturedServer[] = [
	{
		name: "filesystem",
		repo: "modelcontextprotocol/servers",
		desc: "Secure file operations",
		command: "npx",
		args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
		requiresConfig: true,
		hint: "Last arg is the allowed directory path",
	},
	{
		name: "memory",
		repo: "modelcontextprotocol/servers",
		desc: "Knowledge graph-based persistent memory",
		command: "npx",
		args: ["-y", "@modelcontextprotocol/server-memory"],
	},
	{
		name: "github",
		repo: "modelcontextprotocol/servers",
		desc: "GitHub API integration",
		command: "npx",
		args: ["-y", "@modelcontextprotocol/server-github"],
		requiresConfig: true,
		envKeys: ["GITHUB_PERSONAL_ACCESS_TOKEN"],
		hint: "Requires a GitHub personal access token",
	},
	{
		name: "linear",
		repo: "linear/linear",
		desc: "Remote Linear MCP server with browser OAuth",
		transport: "sse",
		url: "https://mcp.linear.app/mcp",
		hint: "After adding, click Enable and complete OAuth",
	},
];

// ── Components ──────────────────────────────────────────────

function Toasts(): VNode {
	return (
		<div className="skills-toast-container">
			{toasts.value.map((t) => {
				const cls = t.type === "error" ? "bg-[var(--error)]" : "bg-[var(--accent)]";
				return (
					<div
						key={t.id}
						className={`pointer-events-auto max-w-[420px] px-4 py-2.5 rounded-md text-xs font-medium text-white shadow-lg ${cls}`}
					>
						{t.message}
					</div>
				);
			})}
		</div>
	);
}

function StatusBadge({ state }: { state?: string }): VNode {
	const colors: Record<string, string> = {
		running: "bg-[var(--ok)]",
		stopped: "bg-[var(--muted)]",
		dead: "bg-[var(--error)]",
		connecting: "bg-[var(--warn)]",
	};
	return <span className={`inline-block w-2 h-2 rounded-full ${colors[state || "stopped"] || colors.stopped}`} />;
}

function FeaturedCard({ server: f }: { server: FeaturedServer }): VNode {
	const installing = useSignal(false);
	const configuring = useSignal(false);
	const argsVal = useSignal((f.args || []).join(" "));
	const envVal = useSignal((f.envKeys || []).map((k) => `${k}=`).join("\n"));
	const urlVal = useSignal(f.url || "");
	const headerVal = useSignal("");
	const timeoutVal = useSignal("");
	const needsConfig = Boolean(
		f.requiresConfig || f.envKeys?.length || f.transport === "sse" || f.transport === "streamable-http",
	);
	const isSse = f.transport === "sse" || f.transport === "streamable-http";

	function onAdd(): void {
		if (needsConfig && !configuring.value) {
			configuring.value = true;
			return;
		}
		installing.value = true;
		const tr = resolveTimeoutOrAbort(timeoutVal.value, (v) => {
			installing.value = v;
		});
		if (!tr.ok) return;
		if (isSse) {
			const url = (urlVal.value || "").trim();
			if (!url) {
				showToast("URL required", "error");
				installing.value = false;
				return;
			}
			addServer({
				headers: parseEnvLines(headerVal.value),
				name: f.name,
				command: "",
				args: [],
				transport: "sse",
				url,
				request_timeout_secs: tr.value,
			}).finally(() => {
				installing.value = false;
				configuring.value = false;
			});
			return;
		}
		addServer({
			name: f.name,
			command: f.command,
			args: argsVal.value.split(/\s+/).filter(Boolean),
			env: parseEnvLines(envVal.value),
			request_timeout_secs: tr.value,
		}).finally(() => {
			installing.value = false;
			configuring.value = false;
		});
	}

	return (
		<div className="mb-1">
			<div className="provider-item">
				<div className="flex-1 min-w-0">
					<div className="provider-item-name font-mono text-sm">{f.name}</div>
					<div className="text-xs text-[var(--muted)] mt-0.5 flex gap-3 items-center">
						<span>{f.desc}</span>
						<span className="text-[0.6rem] px-1.5 py-px rounded-full bg-[var(--surface2)] text-[var(--muted)] font-medium">
							{transportLabel(f.transport)}
						</span>
					</div>
				</div>
				<button
					onClick={onAdd}
					disabled={installing.value}
					className="shrink-0 whitespace-nowrap provider-btn provider-btn-sm"
				>
					{installing.value ? "Adding\u2026" : configuring.value ? "Confirm" : needsConfig ? "Configure" : "Add"}
				</button>
			</div>
			{configuring.value && (
				<div className="px-3 pb-3 border border-t-0 border-[var(--border)] rounded-b-[var(--radius-sm)]">
					{isSse ? (
						<>
							<div className="project-edit-group">
								<div className="text-xs text-[var(--muted)] mb-1">Server URL</div>
								<input
									type="text"
									value={urlVal.value}
									onInput={(e) => {
										urlVal.value = (e.target as HTMLInputElement).value;
									}}
									className="provider-key-input w-full font-mono"
								/>
							</div>
							<div className="project-edit-group">
								<div className="text-xs text-[var(--muted)] mb-1">Headers (KEY=VALUE per line)</div>
								<textarea
									value={headerVal.value}
									onInput={(e) => {
										headerVal.value = (e.target as HTMLTextAreaElement).value;
									}}
									rows={3}
									className="provider-key-input w-full resize-y font-mono text-sm"
								/>
							</div>
						</>
					) : (
						<>
							<div className="project-edit-group">
								<div className="text-xs text-[var(--muted)] mb-1">Arguments</div>
								<input
									type="text"
									value={argsVal.value}
									onInput={(e) => {
										argsVal.value = (e.target as HTMLInputElement).value;
									}}
									className="provider-key-input w-full"
								/>
							</div>
							{f.envKeys?.length ? (
								<div className="project-edit-group">
									<div className="text-xs text-[var(--muted)] mb-1">Env vars (KEY=VALUE per line)</div>
									<textarea
										value={envVal.value}
										onInput={(e) => {
											envVal.value = (e.target as HTMLTextAreaElement).value;
										}}
										rows={f.envKeys.length}
										className="provider-key-input w-full resize-y"
									/>
								</div>
							) : null}
						</>
					)}
					<button
						onClick={() => {
							configuring.value = false;
						}}
						className="self-start provider-btn provider-btn-secondary provider-btn-sm"
					>
						Cancel
					</button>
				</div>
			)}
		</div>
	);
}

function FeaturedSection(): VNode {
	return (
		<div>
			<div className="flex items-center justify-between mb-2">
				<h3 className="text-sm font-medium text-[var(--text-strong)]">Popular MCP Servers</h3>
				<a
					href="https://github.com/modelcontextprotocol/servers"
					target="_blank"
					rel="noopener noreferrer"
					className="text-xs text-[var(--accent)] hover:underline"
				>
					Browse all on GitHub &rarr;
				</a>
			</div>
			<div>
				{featuredServers.map((f) => (
					<FeaturedCard key={f.name} server={f} />
				))}
			</div>
		</div>
	);
}

function InstallBox(): VNode {
	const cmdLine = useSignal("");
	const envVal = useSignal("");
	const adding = useSignal(false);
	const showEnv = useSignal(false);
	const transportType = useSignal("stdio");
	const sseUrl = useSignal("");
	const sseHeaders = useSignal("");
	const timeoutVal = useSignal("");
	const displayNameVal = useSignal("");
	const isSse = transportType.value === "sse" || transportType.value === "streamable-http";
	const canAdd = isSse ? sseUrl.value.trim().length > 0 : cmdLine.value.trim().length > 0;
	const detectedName = isSse ? deriveSseName(sseUrl.value) : deriveNameFromCommand(cmdLine.value);

	function onAdd(): void {
		if (!canAdd) return;
		adding.value = true;
		const tr = resolveTimeoutOrAbort(timeoutVal.value, (v) => {
			adding.value = v;
		});
		if (!tr.ok) return;
		if (isSse) {
			addServer({
				name: detectedName || "remote",
				display_name: displayNameVal.value.trim() || null,
				command: "",
				args: [],
				headers: parseEnvLines(sseHeaders.value),
				transport: transportType.value,
				url: sseUrl.value.trim(),
				request_timeout_secs: tr.value,
			})
				.then(() => {
					sseUrl.value = "";
					sseHeaders.value = "";
					timeoutVal.value = "";
					displayNameVal.value = "";
				})
				.finally(() => {
					adding.value = false;
				});
			return;
		}
		const parts = cmdLine.value.trim().split(/\s+/).filter(Boolean);
		addServer({
			name: detectedName || parts[0],
			display_name: displayNameVal.value.trim() || null,
			command: parts[0],
			args: parts.slice(1),
			env: parseEnvLines(envVal.value),
			request_timeout_secs: tr.value,
		})
			.then(() => {
				cmdLine.value = "";
				envVal.value = "";
				timeoutVal.value = "";
				displayNameVal.value = "";
			})
			.finally(() => {
				adding.value = false;
			});
	}

	return (
		<div className="max-w-[600px] border-t border-[var(--border)] pt-4">
			<h3 className="text-sm font-medium text-[var(--text-strong)] mb-3">Add Custom MCP Server</h3>
			<div className="flex gap-2 mb-3">
				<button
					onClick={() => {
						transportType.value = "stdio";
					}}
					className={`provider-btn provider-btn-sm ${transportType.value === "stdio" ? "" : "provider-btn-secondary"}`}
				>
					Stdio (local)
				</button>
				<button
					onClick={() => {
						transportType.value = "sse";
					}}
					className={`provider-btn provider-btn-sm ${transportType.value === "sse" ? "" : "provider-btn-secondary"}`}
				>
					SSE (remote)
				</button>
				<button
					onClick={() => {
						transportType.value = "streamable-http";
					}}
					className={`provider-btn provider-btn-sm ${transportType.value === "streamable-http" ? "" : "provider-btn-secondary"}`}
				>
					Streamable HTTP
				</button>
			</div>
			{!isSse && (
				<div className="project-edit-group mb-2">
					<div className="text-xs text-[var(--muted)] mb-1">Command</div>
					<input
						type="text"
						className="provider-key-input w-full font-mono"
						placeholder="npx -y mcp-remote https://..."
						value={cmdLine.value}
						onInput={(e) => {
							cmdLine.value = (e.target as HTMLInputElement).value;
						}}
						onKeyDown={(e) => {
							if ((e as KeyboardEvent).key === "Enter") onAdd();
						}}
					/>
				</div>
			)}
			{isSse && (
				<div className="project-edit-group mb-2">
					<div className="text-xs text-[var(--muted)] mb-1">Server URL</div>
					<input
						type="text"
						className="provider-key-input w-full font-mono"
						placeholder="https://mcp.linear.app/mcp"
						value={sseUrl.value}
						onInput={(e) => {
							sseUrl.value = (e.target as HTMLInputElement).value;
						}}
						onKeyDown={(e) => {
							if ((e as KeyboardEvent).key === "Enter") onAdd();
						}}
					/>
					<div className="text-xs text-[var(--muted)] mt-1">
						If the server requires OAuth, your browser opens for sign-in when you enable or restart it. URL query values
						may use <code>$NAME</code> or <code>{"${NAME}"}</code> placeholders from Settings &rarr; Environment
						Variables.
					</div>
				</div>
			)}
			{isSse && (
				<div className="project-edit-group mb-2">
					<div className="text-xs text-[var(--muted)] mb-1">Request headers (optional, KEY=VALUE per line)</div>
					<textarea
						className="provider-key-input w-full min-h-[72px] resize-y font-mono text-sm"
						rows={3}
						placeholder="Authorization=Bearer ..."
						value={sseHeaders.value}
						onInput={(e) => {
							sseHeaders.value = (e.target as HTMLTextAreaElement).value;
						}}
					/>
					<div className="text-xs text-[var(--muted)] mt-1">
						Optional request headers are sent to the remote MCP host. Stored header values stay hidden after save, and
						values may use <code>$NAME</code> or <code>{"${NAME}"}</code> placeholders.
					</div>
				</div>
			)}
			{showEnv.value && (
				<div className="project-edit-group mb-2">
					<div className="text-xs text-[var(--muted)] mb-1">Env vars (KEY=VALUE per line)</div>
					<textarea
						className="provider-key-input w-full min-h-[60px] resize-y font-mono text-sm"
						rows={3}
						value={envVal.value}
						onInput={(e) => {
							envVal.value = (e.target as HTMLTextAreaElement).value;
						}}
					/>
				</div>
			)}
			<div className="project-edit-group mb-2">
				<div className="text-xs text-[var(--muted)] mb-1">Timeout override (seconds, optional)</div>
				<input
					type="number"
					className="provider-key-input w-full font-mono"
					min="1"
					step="1"
					placeholder="Use global default"
					value={timeoutVal.value}
					onInput={(e) => {
						timeoutVal.value = (e.target as HTMLInputElement).value;
					}}
				/>
			</div>
			<div className="flex gap-2 items-center">
				<button className="provider-btn" onClick={onAdd} disabled={adding.value || !canAdd}>
					{adding.value ? "Adding\u2026" : "Add"}
				</button>
				{!isSse && (
					<button
						onClick={() => {
							showEnv.value = !showEnv.value;
						}}
						className="provider-btn provider-btn-secondary provider-btn-sm whitespace-nowrap"
					>
						{showEnv.value ? "Hide env vars" : "+ Environment variables"}
					</button>
				)}
			</div>
		</div>
	);
}

function ServerCard({ server }: { server: McpServer }): VNode {
	const expanded = useSignal(false);
	const tools = useSignal<McpTool[] | null>(null);
	const toggling = useSignal(false);
	const editing = useSignal(false);
	const editTransport = useSignal("stdio");
	const editCmd = useSignal("");
	const editArgs = useSignal("");
	const editEnv = useSignal("");
	const editUrl = useSignal("");
	const editHeaders = useSignal("");
	const editDisplayName = useSignal("");
	const clearHeaders = useSignal(false);
	const editTimeout = useSignal("");
	const saving = useSignal(false);
	const isSse = (server.transport || "stdio") === "sse" || (server.transport || "stdio") === "streamable-http";

	async function toggleTools(): Promise<void> {
		expanded.value = !expanded.value;
		if (expanded.value && !tools.value) {
			const res = await sendRpc("mcp.tools", { name: server.name });
			if (res.ok) tools.value = (res.payload as McpTool[]) || [];
		}
	}
	async function toggleEnabled(): Promise<void> {
		toggling.value = true;
		const method = server.enabled ? "mcp.disable" : "mcp.enable";
		const payload = server.enabled ? { name: server.name } : { name: server.name, redirectUri: oauthCallbackUrl() };
		const res = await sendRpc(method, payload);
		if (res?.ok) {
			const p = res.payload as Record<string, unknown>;
			if (p?.oauthPending) {
				showToast(`OAuth required for "${server.name}"`, "success");
				if (p?.authUrl) window.open(p.authUrl as string, "_blank", "noopener,noreferrer");
			} else showToast(`${server.enabled ? "Disabled" : "Enabled"} "${server.name}"`, "success");
		} else showToast(`Failed: ${res?.error?.message || "unknown"}`, "error");
		await refreshServers();
		toggling.value = false;
	}
	async function restart(): Promise<void> {
		await sendRpc("mcp.restart", { name: server.name });
		showToast(`Restarted "${server.name}"`, "success");
		await refreshServers();
	}
	function startEdit(e: Event): void {
		e.stopPropagation();
		editTransport.value = server.transport || "stdio";
		editCmd.value = server.command || "";
		editArgs.value = (server.args || []).join(" ");
		editEnv.value = Object.entries(server.env || {})
			.map(([k, v]) => `${k}=${v}`)
			.join("\n");
		editUrl.value = "";
		editHeaders.value = "";
		clearHeaders.value = false;
		editTimeout.value = server.request_timeout_secs == null ? "" : String(server.request_timeout_secs);
		editDisplayName.value = server.display_name || "";
		editing.value = true;
	}
	async function saveEdit(): Promise<void> {
		saving.value = true;
		try {
			const tr = resolveTimeoutOrAbort(editTimeout.value, (v) => {
				saving.value = v;
			});
			if (!tr.ok) return;
			const t = editTransport.value;
			const isSseEdit = t === "sse" || t === "streamable-http";
			let payload: Record<string, unknown>;
			if (isSseEdit) {
				payload = {
					name: server.name,
					transport: t,
					request_timeout_secs: tr.value,
					command: "",
					args: [],
					display_name: editDisplayName.value.trim() || null,
				};
				if (editUrl.value.trim()) payload.url = editUrl.value.trim();
				if (clearHeaders.value) payload.headers = {};
				else if (editHeaders.value.trim()) payload.headers = parseEnvLines(editHeaders.value);
			} else {
				const cmd = editCmd.value.trim();
				if (!cmd) {
					showToast("Command required", "error");
					return;
				}
				payload = {
					name: server.name,
					transport: t,
					request_timeout_secs: tr.value,
					command: cmd,
					args: editArgs.value.split(/\s+/).filter(Boolean),
					env: parseEnvLines(editEnv.value),
					headers: {},
					url: null,
					display_name: editDisplayName.value.trim() || null,
				};
			}
			const res = await sendRpc("mcp.update", payload);
			if (res?.ok) {
				showToast(`Updated "${server.name}"`, "success");
				editing.value = false;
			} else showToast(`Failed: ${res?.error?.message || "unknown"}`, "error");
			await refreshServers();
		} finally {
			saving.value = false;
		}
	}
	function remove(e: Event): void {
		e.stopPropagation();
		requestConfirm(`Remove "${server.name}"?`).then((yes) => {
			if (!yes) return;
			sendRpc("mcp.remove", { name: server.name }).then(() => {
				showToast(`Removed "${server.name}"`, "success");
				refreshServers();
			});
		});
	}

	const displayName = server.display_name || server.name;
	const showTechnical = server.display_name && server.display_name !== server.name;
	const currentSafeUrl = typeof server.url === "string" ? server.url.trim() : "";
	const currentHeaderSummary = remoteHeaderSummary(server);

	return (
		<div className="skills-repo-card">
			<div className="skills-repo-header" onClick={toggleTools}>
				<div className="flex items-center gap-2">
					<span
						className={`text-[0.65rem] text-[var(--muted)] transition-transform duration-150 ${expanded.value ? "rotate-90" : ""}`}
					>
						{"\u25B6"}
					</span>
					<StatusBadge state={server.state} />
					<span className="text-sm font-medium text-[var(--text-strong)]">{displayName}</span>
					{showTechnical && (
						<span className="text-[0.62rem] px-1.5 py-px rounded-full bg-[var(--surface2)] text-[var(--muted)] font-mono">
							{server.name}
						</span>
					)}
					<span className="text-[0.62rem] px-1.5 py-px rounded-full bg-[var(--surface2)] text-[var(--muted)] font-medium">
						{server.state || "stopped"}
					</span>
					<span className="text-[0.62rem] px-1.5 py-px rounded-full bg-[var(--surface2)] text-[var(--muted)] font-medium">
						{transportLabel(server.transport)}
					</span>
					<span className="text-xs text-[var(--muted)]">
						{server.tool_count} tool{server.tool_count !== 1 ? "s" : ""}
					</span>
				</div>
				<div className="flex items-center gap-1.5">
					<button onClick={startEdit} className="provider-btn provider-btn-secondary provider-btn-sm">
						Edit
					</button>
					<button
						onClick={(e) => {
							e.stopPropagation();
							toggleEnabled();
						}}
						disabled={toggling.value}
						className={`provider-btn provider-btn-sm ${server.enabled ? "provider-btn-secondary" : ""}`}
					>
						{toggling.value ? "\u2026" : server.enabled ? "Disable" : "Enable"}
					</button>
					<button
						onClick={(e) => {
							e.stopPropagation();
							restart();
						}}
						disabled={!server.enabled}
						className="provider-btn provider-btn-secondary provider-btn-sm"
					>
						Restart
					</button>
					<button onClick={remove} className="provider-btn provider-btn-danger provider-btn-sm">
						Remove
					</button>
				</div>
			</div>
			{editing.value && (
				<div
					className="px-3 pb-3 border border-t-0 border-[var(--border)] rounded-b-[var(--radius-sm)]"
					onClick={(e) => e.stopPropagation()}
				>
					<div className="project-edit-group mb-2 mt-2">
						<div className="text-xs text-[var(--muted)] mb-1">Transport</div>
						<div className="flex gap-2">
							<button
								onClick={() => {
									editTransport.value = "stdio";
								}}
								className={`provider-btn provider-btn-sm ${editTransport.value === "stdio" ? "" : "provider-btn-secondary"}`}
							>
								Stdio
							</button>
							<button
								onClick={() => {
									editTransport.value = "sse";
								}}
								className={`provider-btn provider-btn-sm ${editTransport.value === "sse" ? "" : "provider-btn-secondary"}`}
							>
								SSE
							</button>
							<button
								onClick={() => {
									editTransport.value = "streamable-http";
								}}
								className={`provider-btn provider-btn-sm ${editTransport.value === "streamable-http" ? "" : "provider-btn-secondary"}`}
							>
								Streamable HTTP
							</button>
						</div>
					</div>
					{editTransport.value === "sse" || editTransport.value === "streamable-http" ? (
						<>
							<div className="project-edit-group mb-2">
								<div className="text-xs text-[var(--muted)] mb-1">Current URL</div>
								<div className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface2)] px-3 py-2 text-xs font-mono text-[var(--text)]">
									{currentSafeUrl || "(stored URL hidden until the API returns sanitized text)"}
								</div>
								<div className="text-xs text-[var(--muted)] mt-2 mb-1">
									Replace URL (leave blank to keep the current URL)
								</div>
								<input
									type="text"
									className="provider-key-input w-full font-mono"
									value={editUrl.value}
									placeholder={currentSafeUrl || "https://mcp.example.com/mcp"}
									onInput={(e) => {
										editUrl.value = (e.target as HTMLInputElement).value;
									}}
								/>
								<div className="text-xs text-[var(--muted)] mt-1">
									Leave this blank to preserve the stored URL. Query values may use <code>$NAME</code> or{" "}
									<code>{"${NAME}"}</code> placeholders. OAuth, if required, runs in your browser when the server is
									enabled.
								</div>
							</div>
							<div className="project-edit-group mb-2">
								<div className="text-xs text-[var(--muted)] mb-1">Current headers</div>
								<div className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface2)] px-3 py-2 text-xs font-mono text-[var(--text)]">
									{currentHeaderSummary}
								</div>
								<div className="mt-2">
									<button
										onClick={() => {
											clearHeaders.value = !clearHeaders.value;
										}}
										className="provider-btn provider-btn-secondary provider-btn-sm"
									>
										{clearHeaders.value ? "Keep stored headers" : "Clear stored headers"}
									</button>
								</div>
								<div className="text-xs text-[var(--muted)] mt-2 mb-1">
									Replace headers (optional, KEY=VALUE per line)
								</div>
								<textarea
									className="provider-key-input w-full min-h-[72px] resize-y font-mono text-sm"
									rows={3}
									placeholder="Authorization=Bearer ..."
									value={editHeaders.value}
									disabled={clearHeaders.value}
									onInput={(e) => {
										editHeaders.value = (e.target as HTMLTextAreaElement).value;
									}}
								/>
								<div className="text-xs text-[var(--muted)] mt-1">
									{clearHeaders.value ? (
										"Saving now removes every stored header for this remote server."
									) : (
										<>
											Leave blank to preserve stored headers. Enter new lines to replace them, or click{" "}
											<strong>Clear stored headers</strong> to remove them entirely. Use <code>$NAME</code> or{" "}
											<code>{"${NAME}"}</code> for env-backed values.
										</>
									)}
								</div>
							</div>
						</>
					) : (
						<>
							<div className="project-edit-group mb-2">
								<div className="text-xs text-[var(--muted)] mb-1">Command</div>
								<input
									type="text"
									className="provider-key-input w-full font-mono"
									value={editCmd.value}
									onInput={(e) => {
										editCmd.value = (e.target as HTMLInputElement).value;
									}}
								/>
							</div>
							<div className="project-edit-group mb-2">
								<div className="text-xs text-[var(--muted)] mb-1">Arguments</div>
								<input
									type="text"
									className="provider-key-input w-full font-mono"
									value={editArgs.value}
									onInput={(e) => {
										editArgs.value = (e.target as HTMLInputElement).value;
									}}
								/>
							</div>
							<div className="project-edit-group mb-2">
								<div className="text-xs text-[var(--muted)] mb-1">Env vars (KEY=VALUE per line)</div>
								<textarea
									className="provider-key-input w-full min-h-[40px] resize-y font-mono text-sm"
									rows={2}
									value={editEnv.value}
									onInput={(e) => {
										editEnv.value = (e.target as HTMLTextAreaElement).value;
									}}
								/>
							</div>
						</>
					)}
					<div className="project-edit-group mb-2">
						<div className="text-xs text-[var(--muted)] mb-1">Timeout override (seconds)</div>
						<input
							type="number"
							className="provider-key-input w-full font-mono"
							min="1"
							step="1"
							placeholder="Use global default"
							value={editTimeout.value}
							onInput={(e) => {
								editTimeout.value = (e.target as HTMLInputElement).value;
							}}
						/>
					</div>
					<div className="flex gap-2">
						<button className="provider-btn" onClick={saveEdit} disabled={saving.value}>
							{saving.value ? "Saving\u2026" : "Save"}
						</button>
						<button
							onClick={() => {
								editing.value = false;
							}}
							className="provider-btn provider-btn-secondary provider-btn-sm"
						>
							Cancel
						</button>
					</div>
				</div>
			)}
			{expanded.value && (
				<div className="skills-repo-detail" style={{ display: "block" }}>
					{isSse ? (
						<div>
							<div className="flex items-center gap-1.5 py-1.5 text-xs text-[var(--muted)]">
								<span className="opacity-60">URL</span>
								<code className="font-mono text-[var(--text)]">{currentSafeUrl || "(hidden)"}</code>
							</div>
							<div className="flex items-center gap-1.5 py-1.5 text-xs text-[var(--muted)]">
								<span className="opacity-60">HEADERS</span>
								<code className="font-mono text-[var(--text)]">{currentHeaderSummary}</code>
							</div>
						</div>
					) : (
						<div className="flex items-center gap-1.5 py-1.5 text-xs text-[var(--muted)]">
							<span className="opacity-60">$</span>
							<code className="font-mono text-[var(--text)]">
								{server.command} {(server.args || []).join(" ")}
							</code>
						</div>
					)}
					{!tools.value && <div className="text-[var(--muted)] text-sm py-2">Loading tools&hellip;</div>}
					{tools.value && tools.value.length > 0 && (
						<div className="max-h-[360px] overflow-y-auto">
							{tools.value.map((t) => (
								<div key={t.name} className="flex items-center justify-between py-1.5 border-b border-[var(--border)]">
									<div className="flex items-center gap-2 min-w-0 flex-1 overflow-hidden">
										<span className="font-mono text-sm font-medium text-[var(--text-strong)] whitespace-nowrap">
											{t.name}
										</span>
										{t.description && (
											<span className="text-[var(--muted)] text-xs overflow-hidden text-ellipsis whitespace-nowrap">
												{t.description}
											</span>
										)}
									</div>
								</div>
							))}
						</div>
					)}
					{tools.value && tools.value.length === 0 && (
						<div className="text-[var(--muted)] text-sm py-2">No tools exposed.</div>
					)}
				</div>
			)}
		</div>
	);
}

function ConfiguredServersSection(): VNode {
	return (
		<div>
			<h3 className="text-sm font-medium text-[var(--text-strong)] mb-2">Configured MCP Servers</h3>
			<div>
				{(!servers.value || servers.value.length === 0) && !loading.value && (
					<div className="p-3 text-[var(--muted)] text-sm">No MCP tools configured.</div>
				)}
				{servers.value.map((s) => (
					<ServerCard key={s.name} server={s} />
				))}
			</div>
		</div>
	);
}

function ConfigSection(): VNode {
	return (
		<div className="max-w-[600px] bg-[var(--surface2)] border border-[var(--border)] rounded-[var(--radius)] px-5 py-4">
			<div className="flex items-center justify-between gap-3 mb-2">
				<h3 className="text-sm font-medium text-[var(--text-strong)]">Request Timeout</h3>
				<button
					className="provider-btn provider-btn-secondary provider-btn-sm"
					onClick={refreshConfig}
					disabled={configLoading.value || configSaving.value}
				>
					{configLoading.value ? "Loading\u2026" : "Reload"}
				</button>
			</div>
			<p className="text-xs text-[var(--muted)] mb-3">Controls how long Moltis waits for an MCP server response.</p>
			<div className="flex flex-wrap items-end gap-3">
				<label className="flex flex-col gap-1">
					<span className="text-xs text-[var(--muted)]">Timeout (seconds)</span>
					<input
						type="number"
						min="1"
						step="1"
						value={requestTimeoutSecs.value}
						onInput={(e) => {
							requestTimeoutSecs.value = (e.target as HTMLInputElement).value;
							configDirty.value = true;
						}}
						className="provider-key-input w-[140px]"
					/>
				</label>
				<button
					className="provider-btn provider-btn-sm"
					onClick={saveConfig}
					disabled={configSaving.value || configLoading.value || !configDirty.value}
				>
					{configSaving.value ? "Saving\u2026" : "Save"}
				</button>
			</div>
		</div>
	);
}

function McpPageComponent(): VNode {
	useEffect(() => {
		refreshServers();
		refreshConfig();
		const off = onEvent("mcp.status", (payload: unknown) => {
			if (Array.isArray(payload)) {
				servers.value = payload as McpServer[];
				updateNavCount("mcp", (payload as McpServer[]).filter((s) => s.state === "running").length);
			}
		});
		return off;
	}, []);

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div className="flex items-center gap-3">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">MCP</h2>
				<button className="provider-btn provider-btn-secondary provider-btn-sm" onClick={refreshServers}>
					Refresh
				</button>
			</div>
			<div className="max-w-[600px] bg-[var(--surface2)] border border-[var(--border)] rounded-[var(--radius)] px-5 py-4 leading-relaxed">
				<p className="text-sm text-[var(--text)] mb-2.5">
					<strong className="text-[var(--text-strong)]">MCP (Model Context Protocol)</strong> tools extend the AI agent
					with external capabilities.
				</p>
				<div className="flex items-center gap-2 my-3 px-3.5 py-2.5 bg-[var(--surface)] rounded-[var(--radius-sm)] font-mono text-xs text-[var(--text-strong)]">
					<span className="opacity-50">Agent</span>
					<span className="text-[var(--accent)]">&rarr;</span>
					<span>Moltis</span>
					<span className="text-[var(--accent)]">&rarr;</span>
					<span>Local / Remote MCP</span>
					<span className="text-[var(--accent)]">&rarr;</span>
					<span className="opacity-50">External API</span>
				</div>
				<p className="text-xs text-[var(--muted)]">
					Supports both <strong>local stdio MCP processes</strong> and{" "}
					<strong>remote Streamable HTTP/SSE servers</strong>.
				</p>
			</div>
			<div className="skills-warn max-w-[600px]">
				<div className="skills-warn-title">{"\u26a0\ufe0f"} Review MCP trust boundaries before enabling</div>
				<div>
					Local stdio servers run with <strong>your full system privileges</strong>.
				</div>
				<div className="mt-1">Remote SSE servers receive your tool inputs. Use trusted hosts only.</div>
			</div>
			<ConfigSection />
			<InstallBox />
			<FeaturedSection />
			<ConfiguredServersSection />
			{loading.value && servers.value.length === 0 && (
				<div className="p-6 text-center text-[var(--muted)] text-sm">Loading MCP servers&hellip;</div>
			)}
		</div>
	);
}

// ── Exported init/teardown ──────────────────────────────────

let _mcpContainer: HTMLElement | null = null;

export function initMcp(container: HTMLElement): void {
	_mcpContainer = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	render(
		<>
			<McpPageComponent />
			<Toasts />
			<ConfirmDialog />
		</>,
		container,
	);
}

export function teardownMcp(): void {
	if (_mcpContainer) render(null, _mcpContainer);
	_mcpContainer = null;
}
