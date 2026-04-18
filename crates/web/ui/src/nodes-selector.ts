// ── Node selector (chat toolbar dropdown) ───────────────────

import { onEvent } from "./events";
import { sendRpc } from "./helpers";
import * as S from "./state";
import { nodeStore } from "./stores/node-store";
import type { NodeInfo } from "./types";

let nodeIdx = -1;
let eventUnsubs: Array<() => void> = [];

function isSshTargetNode(node: Partial<NodeInfo> | null): boolean {
	return node?.platform === "ssh" || String(node?.nodeId || "").startsWith("ssh:");
}

function nodeDisplayLabel(node: Partial<NodeInfo> | null): string {
	if (!node) return "Local";
	if (node.displayName) return node.displayName;
	if (isSshTargetNode(node)) {
		const target = String(node.nodeId || "").replace(/^ssh:/, "");
		return `SSH: ${target}`;
	}
	return node.nodeId || "";
}

function nodeMetaLabel(node: Partial<NodeInfo> | null): string {
	if (!node) return "";
	return isSshTargetNode(node) ? "OpenSSH target" : node.platform || "";
}

function fallbackNodeFromId(nodeId: string | null): Partial<NodeInfo> | null {
	if (!nodeId) return null;
	return isSshTargetNode({ nodeId: nodeId } as Partial<NodeInfo>)
		? { nodeId: nodeId, platform: "ssh" }
		: { nodeId: nodeId };
}

function getNodeByIdOrFallback(nodeId: string): Partial<NodeInfo> | null {
	return nodeStore.getById(nodeId) || fallbackNodeFromId(nodeId);
}

function getSelectedNodeForDisplay(): Partial<NodeInfo> | null {
	const selectedId = nodeStore.selectedNodeId.value;
	if (!selectedId) return null;
	return nodeStore.selectedNode.value || fallbackNodeFromId(selectedId);
}

function setSessionNode(sessionKey: string, nodeId: string | null): void {
	sendRpc("nodes.set_session", { session_key: sessionKey, node_id: nodeId || null });
}

function updateNodeComboLabel(node: Partial<NodeInfo> | null): void {
	if (S.nodeComboLabel) {
		S.nodeComboLabel.textContent = nodeDisplayLabel(node);
	}
	if (S.nodeComboBtn) {
		S.nodeComboBtn.title = node
			? isSshTargetNode(node)
				? `Execution target: ${nodeDisplayLabel(node)}`
				: `Execution target: ${nodeDisplayLabel(node)}`
			: "Execution target: Local";
	}
}

export function fetchNodes(): Promise<void> {
	return nodeStore.fetch().then(() => {
		const allNodes = nodeStore.nodes.value;
		const selectedId = nodeStore.selectedNodeId.value;
		// Show or hide the node selector depending on whether nodes are connected.
		if (S.nodeCombo) {
			if (allNodes.length > 0 || selectedId) {
				S.nodeCombo.classList.remove("hidden");
			} else {
				S.nodeCombo.classList.add("hidden");
			}
		}
		const selected = getSelectedNodeForDisplay();
		updateNodeComboLabel(selected);
	});
}

export function selectNode(nodeId: string | null): void {
	nodeStore.select(nodeId);
	const node = nodeId ? getNodeByIdOrFallback(nodeId) : null;
	updateNodeComboLabel(node);
	setSessionNode(S.activeSessionKey, nodeId);
	closeNodeDropdown();
}

export function openNodeDropdown(): void {
	if (!S.nodeDropdown) return;
	S.nodeDropdown.classList.remove("hidden");
	nodeIdx = -1;
	renderNodeList();
}

export function closeNodeDropdown(): void {
	if (!S.nodeDropdown) return;
	S.nodeDropdown.classList.add("hidden");
	nodeIdx = -1;
}

function buildNodeItem(node: Partial<NodeInfo> | null, currentId: string | null): HTMLElement {
	const el = document.createElement("div");
	el.className = "model-dropdown-item";
	if (node && node.nodeId === currentId) el.classList.add("selected");
	if (!(node || currentId)) {
		// "Local" entry
		el.classList.add("selected");
	}

	const label = document.createElement("span");
	label.className = "model-item-label";
	label.textContent = nodeDisplayLabel(node);
	el.appendChild(label);

	if (node) {
		const meta = document.createElement("span");
		meta.className = "model-item-meta";
		const badge = document.createElement("span");
		badge.className = "model-item-provider";
		badge.textContent = nodeMetaLabel(node);
		meta.appendChild(badge);
		el.appendChild(meta);
	}

	el.addEventListener("click", () => selectNode(node ? node.nodeId || null : null));
	return el;
}

export function renderNodeList(): void {
	if (!S.nodeDropdownList) return;
	S.nodeDropdownList.textContent = "";
	const currentId = nodeStore.selectedNodeId.value;
	const allNodes = nodeStore.nodes.value;
	const remoteEntries: Array<Partial<NodeInfo>> = [];

	// "Local" as first item
	S.nodeDropdownList.appendChild(buildNodeItem(null, currentId));

	if (currentId && !allNodes.some((node) => node.nodeId === currentId)) {
		const fallback = getNodeByIdOrFallback(currentId);
		if (fallback) remoteEntries.push(fallback);
	}
	for (const n of allNodes) {
		remoteEntries.push(n);
	}

	if (remoteEntries.length > 0) {
		const divider = document.createElement("div");
		divider.className = "model-dropdown-divider";
		S.nodeDropdownList.appendChild(divider);
	}

	for (const entry of remoteEntries) {
		S.nodeDropdownList.appendChild(buildNodeItem(entry, currentId));
	}
}

function updateNodeActive(): void {
	if (!S.nodeDropdownList) return;
	const items = S.nodeDropdownList.querySelectorAll<HTMLElement>(".model-dropdown-item");
	items.forEach((el, i) => {
		el.classList.toggle("kb-active", i === nodeIdx);
	});
	if (nodeIdx >= 0 && items[nodeIdx]) {
		items[nodeIdx].scrollIntoView({ block: "nearest" });
	}
}

export function bindNodeComboEvents(): void {
	if (!(S.nodeComboBtn && S.nodeDropdownList && S.nodeCombo)) return;

	S.nodeComboBtn.addEventListener("click", () => {
		if (S.nodeDropdown?.classList.contains("hidden")) {
			openNodeDropdown();
		} else {
			closeNodeDropdown();
		}
	});

	S.nodeDropdown?.addEventListener("keydown", (e: KeyboardEvent) => {
		const items = S.nodeDropdownList?.querySelectorAll<HTMLElement>(".model-dropdown-item");
		if (!items) return;
		if (e.key === "ArrowDown") {
			e.preventDefault();
			nodeIdx = Math.min(nodeIdx + 1, items.length - 1);
			updateNodeActive();
		} else if (e.key === "ArrowUp") {
			e.preventDefault();
			nodeIdx = Math.max(nodeIdx - 1, 0);
			updateNodeActive();
		} else if (e.key === "Enter") {
			e.preventDefault();
			if (nodeIdx >= 0 && items[nodeIdx]) items[nodeIdx].click();
		} else if (e.key === "Escape") {
			closeNodeDropdown();
			if (S.nodeComboBtn) S.nodeComboBtn.focus();
		}
	});

	// Subscribe to presence and telemetry events for live updates.
	eventUnsubs.push(onEvent("presence", () => fetchNodes()));
	eventUnsubs.push(onEvent("node.telemetry", () => fetchNodes()));
}

export function unbindNodeEvents(): void {
	for (const unsub of eventUnsubs) unsub();
	eventUnsubs = [];
}

document.addEventListener("click", (e: MouseEvent) => {
	if (S.nodeCombo && !S.nodeCombo.contains(e.target as Node)) {
		closeNodeDropdown();
	}
});

/** Restore node selection from session metadata (called on session switch). */
export function restoreNodeSelection(nodeId: string | null): void {
	nodeStore.select(nodeId || null);
	const node = nodeId ? getNodeByIdOrFallback(nodeId) : null;
	updateNodeComboLabel(node);
}
