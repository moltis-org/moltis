// ── MCP store (signal-based) ─────────────────────────────────
//
// Single source of truth for MCP server data.
// Centralizes signals previously local to page-mcp.js.

import { signal } from "@preact/signals";
import { sendRpc } from "../helpers.js";
import { updateNavCount } from "../nav-counts.js";

// ── Signals ──────────────────────────────────────────────────
export var servers = signal([]);
export var loading = signal(false);

// ── Methods ──────────────────────────────────────────────────

export async function refresh() {
	loading.value = true;
	try {
		var res = await fetch("/api/mcp");
		if (res.ok) {
			servers.value = (await res.json()) || [];
		}
	} catch {
		var rpc = await sendRpc("mcp.list", {});
		if (rpc.ok) servers.value = rpc.payload || [];
	}
	loading.value = false;
	updateNavCount("mcp", servers.value.filter((s) => s.state === "running").length);
}

export function setAll(arr) {
	servers.value = arr || [];
}

export var mcpStore = { servers, loading, refresh, setAll };
