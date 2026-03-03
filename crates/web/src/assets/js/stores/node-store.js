// ── Node store (signal-based) ──────────────────────────────
//
// Single source of truth for connected remote nodes.

import { computed, signal } from "@preact/signals";
import { sendRpc } from "../helpers.js";

// ── Signals ──────────────────────────────────────────────────
export var nodes = signal([]);
export var selectedNodeId = signal(null);

export var selectedNode = computed(() => {
	var id = selectedNodeId.value;
	if (!id) return null;
	return nodes.value.find((n) => n.nodeId === id) || null;
});

// ── Methods ──────────────────────────────────────────────────

/** Replace the full node list from an RPC fetch. */
export function setAll(arr) {
	nodes.value = arr || [];
}

/** Fetch connected nodes from the server via RPC. */
export function fetch() {
	return sendRpc("node.list", {}).then((res) => {
		if (!res?.ok) return;
		setAll(res.payload || []);
	});
}

/** Select a node by id. Pass null to clear (local execution). */
export function select(id) {
	selectedNodeId.value = id || null;
}

/** Look up a node by id. */
export function getById(id) {
	return nodes.value.find((n) => n.nodeId === id) || null;
}

export var nodeStore = { nodes, selectedNodeId, selectedNode, setAll, fetch, select, getById };
