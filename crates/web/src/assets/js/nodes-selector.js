// ── Node selector (chat toolbar dropdown) ───────────────────

import { onEvent } from "./events.js";
import { sendRpc } from "./helpers.js";
import * as S from "./state.js";
import { nodeStore } from "./stores/node-store.js";

var nodeIdx = -1;
var eventUnsubs = [];

function isSshTargetNode(node) {
	return node?.platform === "ssh" || String(node?.nodeId || "").startsWith("ssh:");
}

function nodeDisplayLabel(node) {
	if (!node) return "Local";
	if (node.displayName) return node.displayName;
	if (isSshTargetNode(node)) {
		var target = String(node.nodeId || "").replace(/^ssh:/, "");
		return `SSH: ${target}`;
	}
	return node.nodeId;
}

function nodeMetaLabel(node) {
	if (!node) return "";
	return isSshTargetNode(node) ? "OpenSSH target" : node.platform;
}

function fallbackNodeFromId(nodeId) {
	if (!nodeId) return null;
	return isSshTargetNode({ nodeId: nodeId }) ? { nodeId: nodeId, platform: "ssh" } : { nodeId: nodeId };
}

function getNodeByIdOrFallback(nodeId) {
	return nodeStore.getById(nodeId) || fallbackNodeFromId(nodeId);
}

function getSelectedNodeForDisplay() {
	var selectedId = nodeStore.selectedNodeId.value;
	if (!selectedId) return null;
	return nodeStore.selectedNode.value || fallbackNodeFromId(selectedId);
}

function setSessionNode(sessionKey, nodeId) {
	sendRpc("nodes.set_session", { session_key: sessionKey, node_id: nodeId || null });
}

function updateNodeComboLabel(node) {
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

export function fetchNodes() {
	return nodeStore.fetch().then(() => {
		var allNodes = nodeStore.nodes.value;
		var selectedId = nodeStore.selectedNodeId.value;
		// Show or hide the node selector depending on whether nodes are connected.
		if (S.nodeCombo) {
			if (allNodes.length > 0 || selectedId) {
				S.nodeCombo.classList.remove("hidden");
			} else {
				S.nodeCombo.classList.add("hidden");
			}
		}
		var selected = getSelectedNodeForDisplay();
		updateNodeComboLabel(selected);
	});
}

export function selectNode(nodeId) {
	nodeStore.select(nodeId);
	var node = nodeId ? getNodeByIdOrFallback(nodeId) : null;
	updateNodeComboLabel(node);
	setSessionNode(S.activeSessionKey, nodeId);
	closeNodeDropdown();
}

export function openNodeDropdown() {
	if (!S.nodeDropdown) return;
	S.nodeDropdown.classList.remove("hidden");
	nodeIdx = -1;
	renderNodeList();
}

export function closeNodeDropdown() {
	if (!S.nodeDropdown) return;
	S.nodeDropdown.classList.add("hidden");
	nodeIdx = -1;
}

function buildNodeItem(node, currentId) {
	var el = document.createElement("div");
	el.className = "model-dropdown-item";
	if (node && node.nodeId === currentId) el.classList.add("selected");
	if (!(node || currentId)) {
		// "Local" entry
		el.classList.add("selected");
	}

	var label = document.createElement("span");
	label.className = "model-item-label";
	label.textContent = nodeDisplayLabel(node);
	el.appendChild(label);

	if (node) {
		var meta = document.createElement("span");
		meta.className = "model-item-meta";
		var badge = document.createElement("span");
		badge.className = "model-item-provider";
		badge.textContent = nodeMetaLabel(node);
		meta.appendChild(badge);
		el.appendChild(meta);
	}

	el.addEventListener("click", () => selectNode(node ? node.nodeId : null));
	return el;
}

export function renderNodeList() {
	if (!S.nodeDropdownList) return;
	S.nodeDropdownList.textContent = "";
	var currentId = nodeStore.selectedNodeId.value;
	var allNodes = nodeStore.nodes.value;
	var remoteEntries = [];

	// "Local" as first item
	S.nodeDropdownList.appendChild(buildNodeItem(null, currentId));

	if (currentId && !allNodes.some((node) => node.nodeId === currentId)) {
		remoteEntries.push(getNodeByIdOrFallback(currentId));
	}
	for (var n of allNodes) {
		remoteEntries.push(n);
	}

	if (remoteEntries.length > 0) {
		var divider = document.createElement("div");
		divider.className = "model-dropdown-divider";
		S.nodeDropdownList.appendChild(divider);
	}

	for (var entry of remoteEntries) {
		S.nodeDropdownList.appendChild(buildNodeItem(entry, currentId));
	}
}

function updateNodeActive() {
	if (!S.nodeDropdownList) return;
	var items = S.nodeDropdownList.querySelectorAll(".model-dropdown-item");
	items.forEach((el, i) => {
		el.classList.toggle("kb-active", i === nodeIdx);
	});
	if (nodeIdx >= 0 && items[nodeIdx]) {
		items[nodeIdx].scrollIntoView({ block: "nearest" });
	}
}

export function bindNodeComboEvents() {
	if (!(S.nodeComboBtn && S.nodeDropdownList && S.nodeCombo)) return;

	S.nodeComboBtn.addEventListener("click", () => {
		if (S.nodeDropdown.classList.contains("hidden")) {
			openNodeDropdown();
		} else {
			closeNodeDropdown();
		}
	});

	S.nodeDropdown.addEventListener("keydown", (e) => {
		var items = S.nodeDropdownList.querySelectorAll(".model-dropdown-item");
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

export function unbindNodeEvents() {
	for (var unsub of eventUnsubs) unsub();
	eventUnsubs = [];
}

document.addEventListener("click", (e) => {
	if (S.nodeCombo && !S.nodeCombo.contains(e.target)) {
		closeNodeDropdown();
	}
});

/** Restore node selection from session metadata (called on session switch). */
export function restoreNodeSelection(nodeId) {
	nodeStore.select(nodeId || null);
	var node = nodeId ? getNodeByIdOrFallback(nodeId) : null;
	updateNodeComboLabel(node);
}
