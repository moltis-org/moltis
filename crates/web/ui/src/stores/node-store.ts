// ── Node store (signal-based) ──────────────────────────────
//
// Single source of truth for connected remote nodes.

import { computed, signal } from "@preact/signals";
import { sendRpc } from "../helpers";
import type { NodeInfo, RpcResponse } from "../types";

// ── Signals ──────────────────────────────────────────────────
export const nodes = signal<NodeInfo[]>([]);
export const selectedNodeId = signal<string | null>(null);

export const selectedNode = computed<NodeInfo | null>(() => {
	const id = selectedNodeId.value;
	if (!id) return null;
	return nodes.value.find((n) => n.nodeId === id) || null;
});

// ── Methods ──────────────────────────────────────────────────

/** Replace the full node list from an RPC fetch. */
export function setAll(arr: NodeInfo[]): void {
	nodes.value = arr || [];
}

/** Fetch connected nodes from the server via RPC. */
export function fetch(): Promise<void> {
	return sendRpc("node.list", {}).then((r) => {
		const res = r as RpcResponse<NodeInfo[]>;
		if (!res?.ok) return;
		setAll(res.payload || []);
	});
}

/** Select a node by id. Pass null to clear (local execution). */
export function select(id: string | null): void {
	selectedNodeId.value = id || null;
}

/** Look up a node by id. */
export function getById(id: string): NodeInfo | null {
	return nodes.value.find((n) => n.nodeId === id) || null;
}

export const nodeStore = { nodes, selectedNodeId, selectedNode, setAll, fetch, select, getById };
