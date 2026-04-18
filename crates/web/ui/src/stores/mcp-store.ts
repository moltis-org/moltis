// ── MCP store (signal-based) ─────────────────────────────────
//
// Single source of truth for MCP server data.
// Centralizes signals previously local to page-mcp.js.

import { signal } from "@preact/signals";
import { sendRpc } from "../helpers";
import { updateNavCount } from "../nav-counts";
import type { McpServerInfo } from "../types";

// ── Signals ──────────────────────────────────────────────────
export const servers = signal<McpServerInfo[]>([]);
export const loading = signal<boolean>(false);

// ── Methods ──────────────────────────────────────────────────

export async function refresh(): Promise<void> {
	loading.value = true;
	try {
		const res = await window.fetch("/api/mcp");
		if (res.ok) {
			servers.value = (await res.json()) || [];
		}
	} catch {
		const rpc = await sendRpc("mcp.list", {});
		if (rpc.ok) servers.value = (rpc.payload as McpServerInfo[]) || [];
	}
	loading.value = false;
	updateNavCount("mcp", servers.value.filter((s) => s.state === "running").length);
}

export function setAll(arr: McpServerInfo[]): void {
	servers.value = arr || [];
}

export const mcpStore = { servers, loading, refresh, setAll };
