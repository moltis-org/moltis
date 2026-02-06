// ── MCP Tools page ──────────────────────────────────────────

import { signal, useSignal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { onEvent } from "./events.js";
import { sendRpc } from "./helpers.js";
import { updateNavCount } from "./nav-counts.js";
import { registerPage } from "./router.js";
import * as S from "./state.js";
import { ConfirmDialog, requestConfirm } from "./ui.js";

// ── Signals ─────────────────────────────────────────────────
var servers = signal([]);
var loading = signal(false);
var toasts = signal([]);
var toastId = 0;

// ── Helpers ─────────────────────────────────────────────────
function showToast(message, type) {
	var id = ++toastId;
	toasts.value = toasts.value.concat([{ id: id, message: message, type: type }]);
	setTimeout(() => {
		toasts.value = toasts.value.filter((t) => t.id !== id);
	}, 4000);
}

async function refreshServers() {
	loading.value = true;
	try {
		var res = await fetch("/api/mcp");
		if (res.ok) {
			servers.value = (await res.json()) || [];
		}
	} catch {
		// fall back to WS RPC if HTTP fails
		var rpc = await sendRpc("mcp.list", {});
		if (rpc.ok) servers.value = rpc.payload || [];
	}
	loading.value = false;
	updateNavCount("mcp", servers.value.filter((s) => s.state === "running").length);
}

async function addServer(name, command, args, env) {
	var res = await sendRpc("mcp.add", { name, command, args, env });
	if (res?.ok) {
		var finalName = res.payload?.name || name;
		showToast(`Added MCP tool "${finalName}"`, "success");
	} else {
		var msg = res?.error?.message || res?.error || "unknown error";
		showToast(`Failed to add "${name}": ${msg}`, "error");
	}
	await refreshServers();
}

/** Parse "KEY=VALUE" lines into an object. */
function parseEnvLines(text) {
	var env = {};
	if (!text) return env;
	for (var line of text.split("\n")) {
		var trimmed = line.trim();
		if (!trimmed || trimmed.startsWith("#")) continue;
		var idx = trimmed.indexOf("=");
		if (idx > 0) {
			env[trimmed.slice(0, idx).trim()] = trimmed.slice(idx + 1).trim();
		}
	}
	return env;
}

// ── Featured MCP servers ────────────────────────────────────
var featuredServers = [
	{
		name: "filesystem",
		repo: "modelcontextprotocol/servers",
		desc: "Secure file operations with configurable access controls",
		command: "npx",
		args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
		hint: "Last arg is the allowed directory path",
	},
	{
		name: "memory",
		repo: "modelcontextprotocol/servers",
		desc: "Knowledge graph-based persistent memory system",
		command: "npx",
		args: ["-y", "@modelcontextprotocol/server-memory"],
	},
	{
		name: "github",
		repo: "modelcontextprotocol/servers",
		desc: "GitHub API integration — repos, issues, PRs, code search",
		command: "npx",
		args: ["-y", "@modelcontextprotocol/server-github"],
		envKeys: ["GITHUB_PERSONAL_ACCESS_TOKEN"],
		hint: "Requires a GitHub personal access token",
	},
];

// ── Components ──────────────────────────────────────────────

function Toasts() {
	return html`<div class="skills-toast-container">
    ${toasts.value.map((t) => {
			var cls = t.type === "error" ? "bg-[var(--error)]" : "bg-[var(--accent)]";
			return html`<div key=${t.id}
        class="pointer-events-auto max-w-[420px] px-4 py-2.5 rounded-md text-xs font-medium text-white shadow-lg ${cls}"
      >${t.message}</div>`;
		})}
  </div>`;
}

function StatusBadge({ state }) {
	var colors = {
		running: "bg-[var(--ok)]",
		stopped: "bg-[var(--muted)]",
		dead: "bg-[var(--error)]",
		connecting: "bg-[var(--warn)]",
	};
	var cls = colors[state] || colors.stopped;
	return html`<span class="inline-block w-2 h-2 rounded-full ${cls}"></span>`;
}

function ConfigForm({ server, argsVal, envVal, onCancel }) {
	return html`<div class="mt-2 flex flex-col gap-1.5">
    ${server.hint && html`<div class="text-xs text-[var(--warn)]">${server.hint}</div>`}
    <div class="project-edit-group">
      <div class="text-xs text-[var(--muted)] mb-1">Arguments</div>
      <input type="text" value=${argsVal.value}
        onInput=${(e) => {
					argsVal.value = e.target.value;
				}}
        class="provider-key-input w-full" />
    </div>
    ${
			server.envKeys &&
			server.envKeys.length > 0 &&
			html`<div class="project-edit-group">
        <div class="text-xs text-[var(--muted)] mb-1">Environment variables (KEY=VALUE per line)</div>
        <textarea value=${envVal.value}
          onInput=${(e) => {
						envVal.value = e.target.value;
					}}
          rows=${server.envKeys.length}
          class="provider-key-input w-full resize-y" />
      </div>`
		}
    <button onClick=${onCancel}
      class="self-start provider-btn provider-btn-secondary provider-btn-sm">Cancel</button>
  </div>`;
}

function featuredButtonLabel(installing, configuring, needsConfig) {
	if (installing) return "Adding\u2026";
	if (configuring) return "Confirm";
	if (needsConfig) return "Configure";
	return "Add";
}

function FeaturedCard(props) {
	var f = props.server;
	var installing = useSignal(false);
	var configuring = useSignal(false);
	var argsVal = useSignal(f.args.join(" "));
	var envVal = useSignal((f.envKeys || []).map((k) => `${k}=`).join("\n"));

	var needsConfig = f.envKeys || f.hint;

	function onAdd() {
		if (needsConfig && !configuring.value) {
			configuring.value = true;
			return;
		}
		installing.value = true;
		var argsList = argsVal.value.split(/\s+/).filter(Boolean);
		var env = parseEnvLines(envVal.value);
		addServer(f.name, f.command, argsList, env).then(() => {
			installing.value = false;
			configuring.value = false;
		});
	}

	return html`<div class="mb-1">
    <div class="provider-item">
      <div class="flex-1 min-w-0">
        <div class="provider-item-name font-mono text-sm">${f.name}</div>
        <div class="text-xs text-[var(--muted)] mt-0.5 flex gap-3 items-center">
          <span>${f.desc}</span>
          ${needsConfig && html`<span class="text-[0.6rem] px-1.5 py-px rounded-full bg-[var(--surface2)] text-[var(--muted)] font-medium">config required</span>`}
        </div>
      </div>
      <button onClick=${onAdd} disabled=${installing.value}
        class="shrink-0 whitespace-nowrap provider-btn provider-btn-sm">
        ${featuredButtonLabel(installing.value, configuring.value, needsConfig)}
      </button>
    </div>
    ${
			configuring.value &&
			html`<div class="px-3 pb-3 border border-t-0 border-[var(--border)] rounded-b-[var(--radius-sm)]">
        <${ConfigForm} server=${f} argsVal=${argsVal} envVal=${envVal} onCancel=${() => {
					configuring.value = false;
				}} />
      </div>`
		}
  </div>`;
}

function FeaturedSection() {
	return html`<div>
    <div class="flex items-center justify-between mb-2">
      <h3 class="text-sm font-medium text-[var(--text-strong)]">Popular MCP Tools</h3>
      <a href="https://github.com/modelcontextprotocol/servers" target="_blank" rel="noopener noreferrer"
        class="text-xs text-[var(--accent)] hover:underline">Browse all servers on GitHub \u2192</a>
    </div>
    <div>
      ${featuredServers.map((f) => html`<${FeaturedCard} key=${f.name} server=${f} />`)}
    </div>
  </div>`;
}

/** Derive a short name from a command line, e.g. "npx -y @modelcontextprotocol/server-memory" → "memory". */
function deriveNameFromCommand(cmdLine) {
	var parts = cmdLine.trim().split(/\s+/).filter(Boolean);
	// For remote MCP servers (mcp-remote <url>), extract hostname as name.
	// e.g. "npx -y mcp-remote https://mcp.linear.app/mcp" → "linear"
	var urlIdx = parts.findIndex((p) => /^https?:\/\//.test(p));
	if (urlIdx >= 0) {
		try {
			var hostname = new URL(parts[urlIdx]).hostname;
			// Strip common prefixes: mcp.linear.app → linear
			var hostParts = hostname.split(".").filter((p) => p !== "mcp" && p !== "www");
			if (hostParts.length > 0) return hostParts[0].toLowerCase();
		} catch {
			/* not a valid URL, fall through */
		}
	}
	// Walk backwards to find the most meaningful token (skip flags like -y, --yes).
	for (var i = parts.length - 1; i >= 0; i--) {
		var token = parts[i];
		if (token.startsWith("-")) continue;
		// Strip npm scope: @scope/server-foo → server-foo
		var base = token.includes("/") ? token.split("/").pop() : token;
		// Strip common prefixes: mcp-server-foo → foo, server-foo → foo
		base = base
			.replace(/^mcp-server-/, "")
			.replace(/^server-/, "")
			.replace(/^mcp-/, "");
		if (base) return base.toLowerCase().replace(/[^a-z0-9-]/g, "-");
	}
	return parts[0] || "";
}

/** Derive a short name from an SSE URL, e.g. "https://mcp.linear.app/mcp" → "linear". */
function deriveSseName(url) {
	if (!url) return "";
	try {
		var hostname = new URL(url.trim()).hostname;
		var parts = hostname.split(".").filter((p) => p !== "mcp" && p !== "www");
		return parts.length > 0 ? parts[0].toLowerCase() : "";
	} catch {
		return "";
	}
}

function InstallBox() {
	var cmdLine = useSignal("");
	var envVal = useSignal("");
	var adding = useSignal(false);
	var showEnv = useSignal(false);
	var transportType = useSignal("stdio");
	var sseUrl = useSignal("");

	var isSse = transportType.value === "sse";
	var canAdd = isSse ? sseUrl.value.trim().length > 0 : cmdLine.value.trim().length > 0;
	var detectedName = isSse ? deriveSseName(sseUrl.value) : deriveNameFromCommand(cmdLine.value);

	function onAdd() {
		if (!canAdd) return;
		adding.value = true;
		if (isSse) {
			var sseName = detectedName || "remote";
			sendRpc("mcp.add", {
				name: sseName,
				command: "",
				args: [],
				env: {},
				transport: "sse",
				url: sseUrl.value.trim(),
			}).then((res) => {
				if (res?.ok) {
					showToast(`Added MCP tool "${res.payload?.name || sseName}"`, "success");
				} else {
					showToast(`Failed: ${res?.error?.message || res?.error || "unknown error"}`, "error");
				}
				refreshServers();
				adding.value = false;
				sseUrl.value = "";
			});
			return;
		}
		var parts = cmdLine.value.trim().split(/\s+/).filter(Boolean);
		var command = parts[0];
		var argsList = parts.slice(1);
		var name = detectedName || command;
		var env = parseEnvLines(envVal.value);
		addServer(name, command, argsList, env).then(() => {
			adding.value = false;
			cmdLine.value = "";
			envVal.value = "";
		});
	}

	function onKey(e) {
		if (e.key === "Enter") onAdd();
	}

	return html`<div class="max-w-[600px] border-t border-[var(--border)] pt-4">
    <h3 class="text-sm font-medium text-[var(--text-strong)] mb-3">Add custom MCP tool</h3>
    <div class="flex gap-2 mb-3">
      <button onClick=${() => {
				transportType.value = "stdio";
			}}
        class="provider-btn provider-btn-sm ${transportType.value === "stdio" ? "" : "provider-btn-secondary"}">Stdio (local)</button>
      <button onClick=${() => {
				transportType.value = "sse";
			}}
        class="provider-btn provider-btn-sm ${transportType.value === "sse" ? "" : "provider-btn-secondary"}">SSE (remote)</button>
    </div>
    ${
			!isSse &&
			html`<div class="project-edit-group mb-2">
      <div class="text-xs text-[var(--muted)] mb-1">Command</div>
      <input type="text" class="provider-key-input w-full font-mono" placeholder="npx -y mcp-remote https://mcp.example.com/mcp"
        value=${cmdLine.value}
        onInput=${(e) => {
					cmdLine.value = e.target.value;
				}}
        onKeyDown=${onKey} />
      ${detectedName && html`<div class="text-xs text-[var(--muted)] mt-1">Name: <span class="font-mono text-[var(--text-strong)]">${detectedName}</span> <span class="opacity-60">(editable after adding)</span></div>`}
    </div>`
		}
    ${
			isSse &&
			html`<div class="project-edit-group mb-2">
      <div class="text-xs text-[var(--muted)] mb-1">Server URL</div>
      <input type="text" class="provider-key-input w-full font-mono" placeholder="https://mcp.example.com/mcp"
        value=${sseUrl.value}
        onInput=${(e) => {
					sseUrl.value = e.target.value;
				}}
        onKeyDown=${onKey} />
      ${detectedName && html`<div class="text-xs text-[var(--muted)] mt-1">Name: <span class="font-mono text-[var(--text-strong)]">${detectedName}</span></div>`}
    </div>`
		}
    ${
			showEnv.value &&
			html`<div class="project-edit-group mb-2">
        <div class="text-xs text-[var(--muted)] mb-1">Environment variables (KEY=VALUE per line)</div>
        <textarea class="provider-key-input w-full min-h-[60px] resize-y font-mono text-sm" placeholder="API_KEY=sk-..."
          rows="3"
          value=${envVal.value}
          onInput=${(e) => {
						envVal.value = e.target.value;
					}} />
      </div>`
		}
    <div class="flex gap-2 items-center">
      <button class="provider-btn" onClick=${onAdd} disabled=${adding.value || !canAdd}>
        ${adding.value ? "Adding\u2026" : "Add"}
      </button>
      <button onClick=${() => {
				showEnv.value = !showEnv.value;
			}}
        class="provider-btn provider-btn-secondary provider-btn-sm whitespace-nowrap">
        ${showEnv.value ? "Hide env vars" : "+ Environment variables"}
      </button>
    </div>
  </div>`;
}

function ServerCard({ server }) {
	var expanded = useSignal(false);
	var tools = useSignal(null);
	var toggling = useSignal(false);
	var editing = useSignal(false);
	var editCmd = useSignal("");
	var editArgs = useSignal("");
	var editEnv = useSignal("");
	var saving = useSignal(false);

	async function toggleTools() {
		expanded.value = !expanded.value;
		if (expanded.value && !tools.value) {
			var res = await sendRpc("mcp.tools", { name: server.name });
			if (res.ok) tools.value = res.payload || [];
		}
	}

	async function toggleEnabled() {
		toggling.value = true;
		var method = server.enabled ? "mcp.disable" : "mcp.enable";
		await sendRpc(method, { name: server.name });
		await refreshServers();
		toggling.value = false;
	}

	async function restart() {
		await sendRpc("mcp.restart", { name: server.name });
		showToast(`Restarted "${server.name}"`, "success");
		await refreshServers();
	}

	function startEdit(e) {
		e.stopPropagation();
		editCmd.value = server.command || "";
		editArgs.value = (server.args || []).join(" ");
		editEnv.value = Object.entries(server.env || {})
			.map(([k, v]) => `${k}=${v}`)
			.join("\n");
		editing.value = true;
	}

	async function saveEdit() {
		saving.value = true;
		var argsList = editArgs.value.split(/\s+/).filter(Boolean);
		var env = parseEnvLines(editEnv.value);
		var res = await sendRpc("mcp.update", {
			name: server.name,
			command: editCmd.value.trim(),
			args: argsList,
			env,
		});
		if (res?.ok) {
			showToast(`Updated "${server.name}"`, "success");
			editing.value = false;
		} else {
			var msg = res?.error?.message || res?.error || "unknown error";
			showToast(`Failed to update: ${msg}`, "error");
		}
		saving.value = false;
		await refreshServers();
	}

	function remove(e) {
		e.stopPropagation();
		requestConfirm(`This will stop and remove the "${server.name}" MCP tool. This action cannot be undone.`).then(
			(yes) => {
				if (!yes) return;
				sendRpc("mcp.remove", { name: server.name }).then(() => {
					showToast(`Removed "${server.name}"`, "success");
					refreshServers();
				});
			},
		);
	}

	return html`<div class="skills-repo-card">
    <div class="skills-repo-header" onClick=${toggleTools}>
      <div class="flex items-center gap-2">
        <span class="text-[0.65rem] text-[var(--muted)] transition-transform duration-150 ${expanded.value ? "rotate-90" : ""}">\u25B6</span>
        <${StatusBadge} state=${server.state} />
        <span class="font-mono text-sm font-medium text-[var(--text-strong)]">${server.name}</span>
        <span class="text-[0.62rem] px-1.5 py-px rounded-full bg-[var(--surface2)] text-[var(--muted)] font-medium">${server.state || "stopped"}</span>
        <span class="text-xs text-[var(--muted)]">${server.tool_count} tool${server.tool_count !== 1 ? "s" : ""}${server.state === "running" && server.tool_count > 0 ? ` · ~${server.tool_count * 300} tokens` : ""}</span>
      </div>
      <div class="flex items-center gap-1.5">
        <button onClick=${startEdit}
          class="provider-btn provider-btn-secondary provider-btn-sm" title="Edit">Edit</button>
        <button onClick=${(e) => {
					e.stopPropagation();
					toggleEnabled();
				}} disabled=${toggling.value}
          class="provider-btn provider-btn-sm ${server.enabled ? "provider-btn-secondary" : ""} ${toggling.value ? "cursor-wait opacity-60" : ""}">${toggling.value ? "\u2026" : server.enabled ? "Disable" : "Enable"}</button>
        <button onClick=${(e) => {
					e.stopPropagation();
					restart();
				}} disabled=${!server.enabled}
          class="provider-btn provider-btn-secondary provider-btn-sm">Restart</button>
        <button onClick=${remove}
          class="provider-btn provider-btn-danger provider-btn-sm">Remove</button>
      </div>
    </div>
    ${
			editing.value &&
			html`<div class="px-3 pb-3 border border-t-0 border-[var(--border)] rounded-b-[var(--radius-sm)]" onClick=${(e) => e.stopPropagation()}>
        <div class="project-edit-group mb-2 mt-2">
          <div class="text-xs text-[var(--muted)] mb-1">Command</div>
          <input type="text" class="provider-key-input w-full font-mono" value=${editCmd.value}
            onInput=${(e) => {
							editCmd.value = e.target.value;
						}} />
        </div>
        <div class="project-edit-group mb-2">
          <div class="text-xs text-[var(--muted)] mb-1">Arguments</div>
          <input type="text" class="provider-key-input w-full font-mono" value=${editArgs.value}
            onInput=${(e) => {
							editArgs.value = e.target.value;
						}} />
        </div>
        <div class="project-edit-group mb-2">
          <div class="text-xs text-[var(--muted)] mb-1">Environment variables (KEY=VALUE per line)</div>
          <textarea class="provider-key-input w-full min-h-[40px] resize-y font-mono text-sm" rows="2"
            value=${editEnv.value}
            onInput=${(e) => {
							editEnv.value = e.target.value;
						}} />
        </div>
        <div class="flex gap-2">
          <button class="provider-btn" onClick=${saveEdit} disabled=${saving.value}>
            ${saving.value ? "Saving\u2026" : "Save"}
          </button>
          <button onClick=${() => {
						editing.value = false;
					}}
            class="provider-btn provider-btn-secondary provider-btn-sm">Cancel</button>
        </div>
      </div>`
		}
    ${
			expanded.value &&
			html`<div class="skills-repo-detail" style="display:block">
      <div class="flex items-center gap-1.5 py-1.5 text-xs text-[var(--muted)]">
        <span class="opacity-60">$</span>
        <code class="font-mono text-[var(--text)]">${server.command} ${(server.args || []).join(" ")}</code>
      </div>
      ${!tools.value && html`<div class="text-[var(--muted)] text-sm py-2">Loading tools\u2026</div>`}
      ${
				tools.value &&
				tools.value.length > 0 &&
				html`<div class="max-h-[360px] overflow-y-auto">
        ${tools.value.map(
					(
						t,
					) => html`<div key=${t.name} class="flex items-center justify-between py-1.5 border-b border-[var(--border)]">
            <div class="flex items-center gap-2 min-w-0 flex-1 overflow-hidden">
              <span class="font-mono text-sm font-medium text-[var(--text-strong)] whitespace-nowrap">${t.name}</span>
              ${t.description && html`<span class="text-[var(--muted)] text-xs overflow-hidden text-ellipsis whitespace-nowrap">${t.description}</span>`}
            </div>
          </div>`,
				)}
      </div>`
			}
      ${tools.value && tools.value.length === 0 && html`<div class="text-[var(--muted)] text-sm py-2">No tools exposed by this server.</div>`}
    </div>`
		}
  </div>`;
}

function ConfiguredServersSection() {
	var s = servers.value;
	return html`<div>
    <h3 class="text-sm font-medium text-[var(--text-strong)] mb-2">Configured MCP Tools</h3>
    <div>
      ${(!s || s.length === 0) && !loading.value && html`<div class="p-3 text-[var(--muted)] text-sm">No MCP tools configured. Add one from the popular list above or enter a custom command.</div>`}
      ${s.map((server) => html`<${ServerCard} key=${server.name} server=${server} />`)}
    </div>
  </div>`;
}

function McpPage() {
	useEffect(() => {
		refreshServers();
		// Listen for health status broadcasts from the server.
		var off = onEvent("mcp.status", (payload) => {
			if (Array.isArray(payload)) {
				servers.value = payload;
				updateNavCount("mcp", payload.filter((s) => s.state === "running").length);
			}
		});
		return off;
	}, []);

	return html`
    <div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
      <div class="flex items-center gap-3">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">MCP Tools</h2>
        <button class="provider-btn provider-btn-secondary provider-btn-sm" onClick=${refreshServers}>Refresh</button>
      </div>
      <div class="max-w-[600px] bg-[var(--surface2)] border border-[var(--border)] rounded-[var(--radius)] px-5 py-4 leading-relaxed">
        <p class="text-sm text-[var(--text)] mb-2.5">
          <strong class="text-[var(--text-strong)]">MCP (Model Context Protocol)</strong> tools extend the AI agent with external capabilities — file access, web fetch, database queries, code search, and more.
        </p>
        <div class="flex items-center gap-2 my-3 px-3.5 py-2.5 bg-[var(--surface)] rounded-[var(--radius-sm)] font-mono text-xs text-[var(--text-strong)]">
          <span class="opacity-50">Agent</span>
          <span class="text-[var(--accent)]">\u2192</span>
          <span>Moltis</span>
          <span class="text-[var(--accent)]">\u2192</span>
          <span>Local MCP process</span>
          <span class="text-[var(--accent)]">\u2192</span>
          <span class="opacity-50">External API</span>
        </div>
        <p class="text-xs text-[var(--muted)]">
          Each tool runs as a <strong>local process</strong> on your machine (spawned via npm/uvx). Moltis connects to it over stdio and the process makes outbound API calls on your behalf using your tokens. No data is sent to third-party MCP hosts.
        </p>
      </div>
      <div class="skills-warn max-w-[600px]">
        <div class="skills-warn-title">\u26a0\ufe0f MCP servers run as local processes \u2014 review before enabling</div>
        <div>Each MCP server runs with <strong>your full system privileges</strong>. A malicious or compromised server can read your files, exfiltrate credentials, or execute arbitrary commands \u2014 just like any local process.</div>
        <div style="margin-top:4px"><strong>Triple-check the source code</strong> of any MCP server before enabling it. Only install servers from authors you trust, and keep them updated.</div>
        <div style="margin-top:4px">Each enabled server also adds tool definitions to every chat session's context, consuming tokens. Only enable servers you actively need.</div>
      </div>
      <${InstallBox} />
      <${FeaturedSection} />
      <${ConfiguredServersSection} />
      ${loading.value && servers.value.length === 0 && html`<div class="p-6 text-center text-[var(--muted)] text-sm">Loading MCP tools\u2026</div>`}
    </div>
    <${Toasts} />
    <${ConfirmDialog} />
  `;
}

// ── Router integration ──────────────────────────────────────
registerPage(
	"/mcp",
	function initMcp(container) {
		container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
		render(html`<${McpPage} />`, container);
	},
	function teardownMcp() {
		var container = S.$("pageContent");
		if (container) render(null, container);
	},
);
